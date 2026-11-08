use std::{collections::BTreeSet, net::IpAddr, path::PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::{Datelike, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{config::AlorGatewayConfig, terminal_bar::TerminalPolicy};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub symbol: String,
    pub stream: String,
    pub bootstrap_start: NaiveDate,
    /// Model-origin activation, distinct from terminal.first_live_session.
    pub first_live_session: NaiveDate,
    pub excluded_dates: Vec<NaiveDate>,
    pub gateway_config: PathBuf,
    pub gateway_config_sha256: String,
    pub terminal: TerminalPolicy,
    #[serde(default)]
    pub context_finalization: bool,
    #[serde(default)]
    pub ordinary_observation_max_lateness_seconds: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub late_context_history_policy: Option<LateContextHistoryPolicy>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LateContextHistoryPolicy {
    AuditOnlyV1,
}

impl SourceConfig {
    pub fn version(&self) -> &'static str {
        if self.late_context_history_policy.is_some() {
            "shadow_gateway_ws_closed_m10_v4"
        } else {
            super::VERSION
        }
    }

    pub fn validate(&self, now: i64) -> Result<()> {
        self.terminal.validate()?;
        ensure!(
            self.context_finalization,
            "explicit context finalization required for WS source v3"
        );
        ensure!(
            (self.terminal.max_lateness_seconds..=300)
                .contains(&self.ordinary_observation_max_lateness_seconds),
            "explicit bounded ordinary observation policy required"
        );
        ensure!(
            self.terminal.enabled
                && self.terminal.symbols.len() == 1
                && self.terminal.symbols.contains_key(&self.symbol),
            "one enabled symbol required"
        );
        ensure!(
            self.symbol.len() <= 32 && self.symbol.bytes().all(|c| c.is_ascii_alphanumeric()),
            "invalid source symbol"
        );
        ensure!(
            self.stream
                .starts_with(&format!("shadow.source.closed10m.{}.", self.symbol))
                && self.stream.len() <= 180
                && self
                    .stream
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "invalid owned stream"
        );
        let today = super::local(now)?.date();
        ensure!(
            self.bootstrap_start < self.first_live_session
                && self
                    .first_live_session
                    .signed_duration_since(self.bootstrap_start)
                    .num_days()
                    <= 14
                && self.bootstrap_start <= today
                && today.signed_duration_since(self.bootstrap_start).num_days() <= 40
                && self
                    .first_live_session
                    .signed_duration_since(today)
                    .num_days()
                    <= 7,
            "source date/lifetime bounds"
        );
        ensure!(
            self.terminal.first_live_session <= today
                && self.terminal.first_live_session <= self.first_live_session
                && self.terminal.first_live_session >= self.bootstrap_start,
            "observation activation bounds"
        );
        ensure!(
            self.excluded_dates.len() <= 16
                && self.excluded_dates.iter().collect::<BTreeSet<_>>().len()
                    == self.excluded_dates.len()
                && self.first_live_session.weekday().num_days_from_monday() < 5
                && !self.excluded_dates.contains(&self.first_live_session),
            "invalid activation/calendar"
        );
        ensure!(
            self.gateway_config_sha256.len() == 64
                && self
                    .gateway_config_sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit()),
            "missing gateway config hash"
        );
        Ok(())
    }

    pub fn context_day(&self, day: NaiveDate) -> bool {
        day.weekday().num_days_from_monday() >= 5 || self.excluded_dates.contains(&day)
    }

    pub fn finalizer(&self) -> Result<crate::terminal_bar::TerminalFinalizer> {
        ensure!(self.context_finalization, "context policy not enabled");
        crate::terminal_bar::TerminalFinalizer::new_with_observation(
            self.terminal.clone(),
            self.excluded_dates.clone(),
            self.ordinary_observation_max_lateness_seconds,
        )
    }

    pub fn context_cutoff(&self, day: NaiveDate) -> Result<i64> {
        let mut next = day.succ_opt().context("context date overflow")?;
        for _ in 0..32 {
            if !self.context_day(next) {
                return Ok(next.and_hms_opt(7, 0, 0).unwrap().and_utc().timestamp() - 10800);
            }
            next = next.succ_opt().context("context date overflow")?;
        }
        anyhow::bail!("context cutoff calendar budget")
    }

    pub fn history_import_mode(
        &self,
        label: i64,
        received: i64,
        published: i64,
    ) -> Result<&'static str> {
        ensure!(
            self.eligible(label)? && received >= label + 600 && received <= published,
            "invalid historical receipt"
        );
        let day = super::local(label)?.date();
        if day < self.first_live_session && published < self.model_cutoff() {
            return Ok("bootstrap");
        }
        ensure!(
            self.context_day(day) && published < self.context_cutoff(day)?,
            "unobserved active history cannot become timely Live or rewrite frozen context"
        );
        Ok("context_recovery_before_cutoff")
    }

    pub fn check_history_completion(
        &self,
        label: i64,
        received: i64,
        started: i64,
        completed: i64,
    ) -> Result<()> {
        ensure!(completed >= started, "history completion clock rollback");
        ensure!(
            self.history_import_mode(label, received, started)?
                == self.history_import_mode(label, received, completed)?,
            "history import crossed activation/cutoff"
        );
        Ok(())
    }

    pub fn eligible(&self, ts: i64) -> Result<bool> {
        let d = super::local(ts)?;
        if d.date() < self.bootstrap_start || ts % 600 != 0 {
            return Ok(false);
        }
        let Some(last) = self.terminal.symbols[&self.symbol].terminal_label(d.date()) else {
            return Ok(false);
        };
        let start = if self.context_day(d.date()) {
            9 * 60 + 50
        } else {
            7 * 60
        };
        Ok(d.hour() * 60 + d.minute() >= start && ts <= last)
    }

    pub fn model_cutoff(&self) -> i64 {
        self.first_live_session
            .and_hms_opt(7, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
            - 10800
    }

    pub fn gateway(&self, fixture: bool) -> Result<AlorGatewayConfig> {
        let path = &self.gateway_config;
        ensure!(
            !path.is_symlink() && path.is_file() && path.metadata()?.len() <= 65536,
            "regular bounded gateway config required"
        );
        let raw = std::fs::read(path)?;
        ensure!(
            format!("{:x}", Sha256::digest(&raw)) == self.gateway_config_sha256,
            "gateway config hash changed"
        );
        let cfg = AlorGatewayConfig::from_file_with_sources(
            path.to_str().context("invalid config path")?.into(),
        )?
        .config;
        ensure!(
            cfg.portfolio == "__market_data_only__"
                && cfg.symbols == vec![self.symbol.clone()]
                && cfg.tf_sec == 600
                && !cfg.skip_history_bars,
            "MD-only sentinel/symbol/M10/history required"
        );
        let from = self
            .bootstrap_start
            .and_hms_opt(7, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
            - 10800;
        ensure!(cfg.from_ts == from, "WS bootstrap start mismatch");
        if fixture {
            ensure!(
                cfg.ws_url.starts_with("ws://127.0.0.1:")
                    && cfg.oauth_url.starts_with("http://127.0.0.1:"),
                "fixture endpoints must be owned loopback"
            );
        } else {
            ensure!(
                cfg.ws_url == "wss://api.alor.ru/ws"
                    && cfg.oauth_url == "https://oauth.alor.ru/refresh",
                "production MD/auth endpoints not approved"
            );
        }
        Ok(cfg)
    }
}

pub fn redis_client(raw: &str, fixture: bool) -> Result<redis::Client> {
    let client = redis::Client::open(raw).context("invalid source Redis URL")?;
    let info = client.get_connection_info();
    let redis::ConnectionAddr::Tcp(host, _) = &info.addr else {
        anyhow::bail!("private TCP Redis required")
    };
    let address: IpAddr = host
        .parse()
        .context("literal private Redis address required")?;
    let private = match address {
        IpAddr::V4(x) => x.is_private() || x.is_loopback(),
        IpAddr::V6(x) => x.is_unique_local() || x.is_loopback(),
    };
    ensure!(
        private
            && (!fixture || address.is_loopback())
            && info.redis.db == 0
            && info.redis.username.as_ref().is_some_and(|s| !s.is_empty())
            && info.redis.password.as_ref().is_some_and(|s| !s.is_empty()),
        "dedicated private source Redis ACL required"
    );
    Ok(client)
}
