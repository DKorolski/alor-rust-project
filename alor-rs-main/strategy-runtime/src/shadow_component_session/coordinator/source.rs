//! Read-only dependency checks. A pin is reviewed input, not an admission grant.
use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{read_small, Spec};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessPin {
    pub pid: u32,
    pub linux_start_ticks: u64,
    pub boot_id: String,
}
impl ProcessPin {
    /// Inspection only. The returned identity must be separately reviewed/pinned.
    pub fn inspect(pid: u32) -> Result<Self> {
        ensure!(
            cfg!(target_os = "linux") && pid > 1,
            "Linux source process identity required"
        );
        let boot = String::from_utf8(read_small(
            std::path::Path::new("/proc/sys/kernel/random/boot_id"),
            128,
        )?)?;
        let raw = String::from_utf8(read_small(
            &PathBuf::from(format!("/proc/{pid}/stat")),
            16384,
        )?)?;
        let rest = raw.rsplit_once(')').context("source process stat")?.1;
        let ticks = rest
            .split_whitespace()
            .nth(19)
            .context("source start ticks")?
            .parse()?;
        Ok(Self {
            pid,
            linux_start_ticks: ticks,
            boot_id: boot.trim().into(),
        })
    }
}
pub(super) fn check_process(pin: &ProcessPin) -> Result<()> {
    let actual = ProcessPin::inspect(pin.pid)?;
    ensure!(
        actual.boot_id == pin.boot_id && actual.linux_start_ticks == pin.linux_start_ticks,
        "source process/boot changed; reauthorization required"
    );
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePin {
    pub root: PathBuf,
    pub manifest_sha256: String,
    pub process: ProcessPin,
    /// Exact reviewed pre-freeze journals. A permanent dependency-aware
    /// classifier is separate; no blanket approval of growing audits exists.
    pub reviewed_price_audit_sha256: Option<String>,
    pub reviewed_late_context_sha256: Option<String>,
}
fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit())
}
impl SourcePin {
    pub(super) fn validate(&self, spec: &Spec) -> Result<()> {
        ensure!(
            self.root.is_absolute()
                && is_hash(&self.manifest_sha256)
                && self.manifest_sha256 == spec.source_manifest_sha256
                && self.process.pid > 1
                && self.process.linux_start_ticks > 0
                && self.process.boot_id.len() == 36,
            "invalid source/process pin"
        );
        ensure!(
            [
                &self.reviewed_price_audit_sha256,
                &self.reviewed_late_context_sha256
            ]
            .iter()
            .all(|v| v.as_ref().is_none_or(|s| is_hash(s))),
            "invalid reviewed audit pin"
        );
        Ok(())
    }
}

pub(super) enum SourceState {
    Ready,
    Recovering,
}
pub(super) struct SourceGuard {
    pin: SourcePin,
    config_sha256: String,
    heartbeat_budget: i64,
    generation: Option<u64>,
    process: fn(&ProcessPin) -> Result<()>,
}
impl SourceGuard {
    pub(super) fn new(
        pin: SourcePin,
        spec: &Spec,
        queue_budget: u16,
        process: fn(&ProcessPin) -> Result<()>,
    ) -> Result<Self> {
        pin.validate(spec)?;
        ensure!(
            pin.root.canonicalize()? == pin.root,
            "canonical source root required"
        );
        process(&pin.process)?;
        let bytes = read_small(&pin.root.join("manifest.json"), 65536)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == pin.manifest_sha256,
            "source manifest changed"
        );
        let manifest: Value = serde_json::from_slice(&bytes)?;
        let cfg = &manifest["config"];
        let config_sha256 = manifest["config_sha256"]
            .as_str()
            .context("source config hash")?
            .to_string();
        let first: chrono::NaiveDate = cfg["first_live_session"]
            .as_str()
            .context("source activation")?
            .parse()?;
        let ordinary = cfg["ordinary_observation_max_lateness_seconds"]
            .as_u64()
            .context("source collection budget")?;
        let terminal = &cfg["terminal"];
        let terminal_lag = terminal["max_lateness_seconds"]
            .as_u64()
            .context("terminal collection budget")?;
        let heartbeat_budget = terminal["max_heartbeat_age_seconds"]
            .as_i64()
            .context("source heartbeat budget")?;
        ensure!(
            manifest["version"] == "shadow_gateway_ws_closed_m10_v4"
                && cfg["symbol"] == spec.ingress.symbol
                && cfg["stream"] == spec.ingress.source_stream
                && first <= spec.date
                && cfg["context_finalization"] == true
                && cfg["late_context_history_policy"] == "audit_only_v1"
                && terminal["enabled"] == true
                && terminal["symbols"] == serde_json::json!([spec.ingress.symbol])
                && (1..=60).contains(&heartbeat_budget)
                && terminal_lag > 0
                && ordinary >= terminal_lag
                && ordinary + u64::from(queue_budget) <= u64::from(spec.max_execution_lag_seconds)
                && format!("{:x}", Sha256::digest(serde_json::to_vec(cfg)?)) == config_sha256,
            "unqualified source identity/budgets"
        );
        Ok(Self {
            pin,
            config_sha256,
            heartbeat_budget,
            generation: None,
            process,
        })
    }
    pub(super) fn publisher(&self) -> &str {
        "shadow_gateway_ws_closed_m10_v4"
    }
    pub(super) fn generation(&self) -> Option<u64> {
        self.generation
    }
    pub(super) fn check(&mut self, now: i64, before_freeze: bool) -> Result<SourceState> {
        (self.process)(&self.pin.process)?;
        let manifest = read_small(&self.pin.root.join("manifest.json"), 65536)?;
        ensure!(
            format!("{:x}", Sha256::digest(&manifest)) == self.pin.manifest_sha256,
            "source manifest changed"
        );
        ensure!(
            !self.pin.root.join("FAILED.json").try_exists()?
                && !self.pin.root.join("FAILED.json").is_symlink(),
            "source failed; no automatic repair"
        );
        if before_freeze {
            for (name, expected) in [
                ("revisions.jsonl", &self.pin.reviewed_price_audit_sha256),
                ("late-context.jsonl", &self.pin.reviewed_late_context_sha256),
            ] {
                let path = self.pin.root.join(name);
                let actual = if path.try_exists()? {
                    Some(format!(
                        "{:x}",
                        Sha256::digest(read_small(&path, 2 * 1024 * 1024)?)
                    ))
                } else {
                    ensure!(!path.is_symlink(), "source audit symlink");
                    None
                };
                ensure!(
                    &actual == expected,
                    "unresolved pre-freeze source audit; context review required"
                );
            }
        }
        let health: Value =
            serde_json::from_slice(&read_small(&self.pin.root.join("HEALTH.json"), 65536)?)?;
        let generation = health["generation"]
            .as_u64()
            .context("source WS generation")?;
        let heartbeat = health["heartbeat_utc"]
            .as_i64()
            .context("source heartbeat")?;
        let rx = health["last_ws_rx_utc"]
            .as_i64()
            .context("source RX time")?;
        ensure!(
            health["version"] == self.publisher()
                && health["config_sha256"] == self.config_sha256
                && health["order_emission_allowed"] == false
                && generation >= self.generation.unwrap_or(1)
                && now
                    .checked_sub(heartbeat)
                    .is_some_and(|age| (-5..=15).contains(&age))
                && rx <= now.saturating_add(5)
                && matches!(health["status"].as_str(), Some("ready" | "reconnecting")),
            "source health/clock/identity invalid"
        );
        self.generation = Some(generation);
        if health["status"] == "ready"
            && health["connected"] == true
            && health["subscribed"] == true
            && health["history_complete"] == true
            && health["queue_drained"] == true
            && now.saturating_sub(rx) <= self.heartbeat_budget
        {
            Ok(SourceState::Ready)
        } else {
            Ok(SourceState::Recovering)
        }
    }
}
