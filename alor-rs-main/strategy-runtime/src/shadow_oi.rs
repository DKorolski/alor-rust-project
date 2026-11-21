//! Opt-in OI validation and immutable offline evidence store. No HTTP client,
//! scheduler, strategy callback, broker credentials or order-emission capability.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::shadow_calendar::{Calendar, SessionKind};

#[cfg(feature = "shadow-oi-http")]
pub mod collector;

pub const HISTORY_URL: &str = "https://iss.moex.com/iss/history/engines/futures/markets/forts/boards/RFUD/securities/IMOEXF.json";
const SCHEMA: &str = "imoexf_pit_oi_evidence_v1";
const MAX_BYTES: usize = 1024 * 1024;
const MAX_CLOCK_OFFSET_MS: u64 = 1000;
const MAX_HTTP_SECONDS: i64 = 30;
const MAX_SERVER_SKEW_SECONDS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    schema_version: String,
    profile_id: String,
    model_sha256: String,
    calendar_sha256: String,
    decision_date: NaiveDate,
    previous: NaiveDate,
    previous2: NaiveDate,
}

impl Plan {
    pub fn new(calendar: &Calendar, day: NaiveDate, model_sha256: &str) -> Result<Self> {
        ensure!(
            model_sha256.len() == 64 && model_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "reviewed model hash required"
        );
        let selection = calendar.select_components(
            day,
            crate::shadow_calendar::ContextPolicy::RegularOnly,
            crate::shadow_calendar::ContextPolicy::RegularOnly,
            &BTreeMap::new(),
        );
        let session = calendar.session(day);
        ensure!(
            selection.symbol == "IMOEXF"
                && session.kind == SessionKind::Regular
                && session.context_open_minute == Some(420)
                && day.weekday().num_days_from_monday() < 5,
            "OI plan requires IMOEXF regular07 weekday"
        );
        let (previous, previous2) = calendar.prior_regular_dates(day)?;
        Ok(Self {
            schema_version: SCHEMA.into(),
            profile_id: "IMO_AB_OI".into(),
            model_sha256: model_sha256.into(),
            calendar_sha256: calendar.sha256().into(),
            decision_date: day,
            previous,
            previous2,
        })
    }

    pub fn request_params(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("from".into(), self.previous2.to_string()),
            ("till".into(), self.previous.to_string()),
            ("iss.meta".into(), "off".into()),
        ])
    }

    fn at(&self, hour: u32, minute: u32) -> DateTime<Utc> {
        (self.decision_date.and_hms_opt(hour, minute, 0).unwrap() - Duration::hours(3)).and_utc()
    }
}

/// Provided by a future trusted transport/durable-writer boundary, not by the
/// ISS body. Offline fixtures do not establish real receipt or clock accuracy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub url: String,
    pub params: BTreeMap<String, String>,
    pub http_status: u16,
    pub server_date: Option<String>,
    pub requested_at_utc: DateTime<Utc>,
    pub received_at_utc: DateTime<Utc>,
    pub durable_at_utc: DateTime<Utc>,
    pub clock_offset_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Decreasing,
    NonDecreasing,
    Missing,
    LateFreeze,
    LateReceipt,
    OutsideCaptureWindow,
    InvalidClock,
    WrongRequest,
    HttpFailure,
    InvalidSchema,
    IncompletePage,
    WrongInstrumentOrDate,
    DuplicateDate,
    MissingSourceDate,
    InvalidValue,
    ZeroDenominator,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Evaluation {
    pub reason: Reason,
    pub prior_oi: Option<f64>,
    pub prior2_oi: Option<f64>,
    pub delta_oi: Option<f64>,
}

impl Evaluation {
    fn rejected(reason: Reason) -> Self {
        Self {
            reason,
            prior_oi: None,
            prior2_oi: None,
            delta_oi: None,
        }
    }

    pub fn allow_new_bo(&self) -> bool {
        self.reason == Reason::Decreasing
    }

    fn valid_snapshot(&self) -> bool {
        matches!(self.reason, Reason::Decreasing | Reason::NonDecreasing)
    }
}

#[derive(Deserialize)]
struct Table {
    columns: Vec<String>,
    data: Vec<Vec<Value>>,
}

impl Table {
    fn column(&self, name: &str) -> std::result::Result<usize, Reason> {
        self.columns
            .iter()
            .position(|c| c == name)
            .ok_or(Reason::InvalidSchema)
    }

    fn validate(&self) -> std::result::Result<(), Reason> {
        let unique: BTreeSet<_> = self.columns.iter().collect();
        if self.columns.is_empty()
            || self.columns.len() > 128
            || unique.len() != self.columns.len()
            || self.data.len() > 128
            || self.data.iter().any(|row| row.len() != self.columns.len())
        {
            return Err(Reason::InvalidSchema);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Payload {
    history: Table,
    #[serde(rename = "history.cursor")]
    cursor: Option<Table>,
}

fn dated_values(plan: &Plan, raw: &[u8]) -> std::result::Result<(f64, f64), Reason> {
    if raw.len() > MAX_BYTES {
        return Err(Reason::InvalidSchema);
    }
    let payload: Payload = serde_json::from_slice(raw).map_err(|_| Reason::InvalidSchema)?;
    let table = payload.history;
    table.validate()?;
    let board = table.column("BOARDID")?;
    let symbol = table.column("SECID")?;
    let date = table.column("TRADEDATE")?;
    let oi = table.column("OPENPOSITION")?;
    if let Some(cursor) = payload.cursor {
        cursor.validate()?;
        let index = cursor.column("INDEX")?;
        let total = cursor.column("TOTAL")?;
        let page = cursor.column("PAGESIZE")?;
        if cursor.data.len() != 1
            || cursor.data[0][index].as_u64() != Some(0)
            || cursor.data[0][total].as_u64() != Some(table.data.len() as u64)
            || cursor.data[0][page]
                .as_u64()
                .is_none_or(|n| n < table.data.len() as u64)
        {
            return Err(Reason::IncompletePage);
        }
    }
    let mut records = BTreeMap::new();
    for row in &table.data {
        if row[board].as_str() != Some("RFUD") || row[symbol].as_str() != Some("IMOEXF") {
            return Err(Reason::WrongInstrumentOrDate);
        }
        let text = row[date].as_str().ok_or(Reason::WrongInstrumentOrDate)?;
        let day = NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map_err(|_| Reason::WrongInstrumentOrDate)?;
        if day.to_string() != text || day < plan.previous2 || day > plan.previous {
            return Err(Reason::WrongInstrumentOrDate);
        }
        let value = row[oi]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0. && *v <= 9_007_199_254_740_991.)
            .ok_or(Reason::InvalidValue)?;
        if records.insert(day, value).is_some() {
            return Err(Reason::DuplicateDate);
        }
    }
    let prior = *records
        .get(&plan.previous)
        .ok_or(Reason::MissingSourceDate)?;
    let prior2 = *records
        .get(&plan.previous2)
        .ok_or(Reason::MissingSourceDate)?;
    if prior2 == 0. {
        return Err(Reason::ZeroDenominator);
    }
    Ok((prior, prior2))
}

pub fn evaluate(plan: &Plan, receipt: &Receipt, raw: &[u8]) -> Evaluation {
    let reject = Evaluation::rejected;
    if receipt.url != HISTORY_URL || receipt.params != plan.request_params() {
        return reject(Reason::WrongRequest);
    }
    if receipt.http_status != 200 {
        return reject(Reason::HttpFailure);
    }
    if receipt
        .clock_offset_ms
        .is_none_or(|v| v.unsigned_abs() > MAX_CLOCK_OFFSET_MS)
        || receipt.requested_at_utc > receipt.received_at_utc
        || receipt.received_at_utc > receipt.durable_at_utc
        || receipt.received_at_utc - receipt.requested_at_utc > Duration::seconds(MAX_HTTP_SECONDS)
    {
        return reject(Reason::InvalidClock);
    }
    let server = receipt
        .server_date
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc2822(v).ok());
    if !server.is_some_and(|v| {
        (receipt.received_at_utc - v.with_timezone(&Utc))
            .num_seconds()
            .unsigned_abs()
            <= MAX_SERVER_SKEW_SECONDS
    }) {
        return reject(Reason::InvalidClock);
    }
    if receipt.durable_at_utc > plan.at(6, 55) {
        return reject(Reason::LateReceipt);
    }
    if receipt.requested_at_utc < plan.at(6, 45) {
        return reject(Reason::OutsideCaptureWindow);
    }
    match dated_values(plan, raw) {
        Ok((prior, prior2)) => {
            let delta = prior / prior2 - 1.;
            if !delta.is_finite() {
                return reject(Reason::InvalidValue);
            }
            Evaluation {
                reason: if prior < prior2 {
                    Reason::Decreasing
                } else {
                    Reason::NonDecreasing
                },
                prior_oi: Some(prior),
                prior2_oi: Some(prior2),
                delta_oi: Some(delta),
            }
        }
        Err(reason) => reject(reason),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Capture {
    plan: Plan,
    receipt: Receipt,
    received_at_msk: String,
    durable_at_msk: String,
    raw_sha256: String,
    evaluation: Evaluation,
}

pub struct ArchivedCapture {
    id: Uuid,
    manifest_sha256: String,
    capture: Capture,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DailyDecision {
    plan: Plan,
    frozen_at_utc: DateTime<Utc>,
    frozen_at_msk: String,
    evaluation: Evaluation,
    capture_id: Option<Uuid>,
    capture_manifest_sha256: Option<String>,
}

impl DailyDecision {
    pub fn allow_new_bo(&self) -> bool {
        self.evaluation.allow_new_bo()
    }
    pub fn reason(&self) -> Reason {
        self.evaluation.reason
    }
    pub fn delta_oi(&self) -> Option<f64> {
        self.evaluation.delta_oi
    }
}

/// Local offline boundary only. The later collector must produce trustworthy
/// receipt/durability evidence and independently attest final commit deadlines.
/// No caller may treat these files alone as prospective admission or PIT proof.
pub struct Store {
    root: PathBuf,
}

fn msk(value: DateTime<Utc>) -> String {
    value
        .with_timezone(&chrono::FixedOffset::east_opt(3 * 3600).unwrap())
        .to_rfc3339()
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "symlink evidence"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "evidence size bound");
    Ok(bytes)
}

fn publish_once(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_BYTES, "evidence size bound");
    let parent = path.parent().context("evidence parent")?;
    let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // Atomic no-replace publication; a racing writer cannot overwrite history.
        std::fs::hard_link(&temporary, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("captures"))?;
        std::fs::create_dir_all(root.join("decisions"))?;
        Ok(Self { root })
    }

    pub fn archive(&self, plan: &Plan, receipt: Receipt, raw: &[u8]) -> Result<ArchivedCapture> {
        self.archive_inner(plan, receipt, raw, || Ok(None))
    }

    #[cfg(feature = "shadow-oi-http")]
    fn archive_measured(
        &self,
        plan: &Plan,
        receipt: Receipt,
        raw: &[u8],
        durable_clock: impl FnOnce() -> Result<DateTime<Utc>>,
    ) -> Result<ArchivedCapture> {
        self.archive_inner(plan, receipt, raw, || durable_clock().map(Some))
    }

    fn archive_inner(
        &self,
        plan: &Plan,
        mut receipt: Receipt,
        raw: &[u8],
        durable_clock: impl FnOnce() -> Result<Option<DateTime<Utc>>>,
    ) -> Result<ArchivedCapture> {
        ensure!(raw.len() <= MAX_BYTES, "capture size bound");
        ensure!(
            receipt.url == HISTORY_URL && receipt.params == plan.request_params(),
            "only public dated request metadata can be archived"
        );
        let id = Uuid::new_v4();
        let directory = self.root.join("captures").join(id.to_string());
        std::fs::create_dir(&directory)?;
        publish_once(&directory.join("history.json"), raw)?;
        std::fs::File::open(self.root.join("captures"))?.sync_all()?;
        if let Some(measured) = durable_clock()? {
            receipt.durable_at_utc = measured;
        }
        let capture = Capture {
            plan: plan.clone(),
            received_at_msk: msk(receipt.received_at_utc),
            durable_at_msk: msk(receipt.durable_at_utc),
            raw_sha256: digest(raw),
            evaluation: evaluate(plan, &receipt, raw),
            receipt,
        };
        let bytes = serde_json::to_vec(&capture)?;
        publish_once(&directory.join("manifest.json"), &bytes)?;
        std::fs::File::open(self.root.join("captures"))?.sync_all()?;
        Ok(ArchivedCapture {
            id,
            manifest_sha256: digest(&bytes),
            capture,
        })
    }

    pub fn load(&self, plan: &Plan) -> Result<Option<DailyDecision>> {
        self.load_with_reader(plan, &read_bounded)
    }

    // One validator for the original archive and a frozen session bundle.
    fn load_with_reader(
        &self,
        plan: &Plan,
        read: &impl Fn(&Path) -> Result<Vec<u8>>,
    ) -> Result<Option<DailyDecision>> {
        let path = self
            .root
            .join("decisions")
            .join(format!("{}.json", plan.decision_date));
        let bytes = match read(&path) {
            Ok(bytes) => bytes,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let decision: DailyDecision = serde_json::from_slice(&bytes)?;
        ensure!(
            decision.plan == *plan,
            "frozen OI identity mismatch; explicit rebuild required"
        );
        ensure!(
            decision.frozen_at_msk == msk(decision.frozen_at_utc),
            "frozen clock metadata mismatch"
        );
        if let Some(id) = decision.capture_id {
            let directory = self.root.join("captures").join(id.to_string());
            let manifest = read(&directory.join("manifest.json"))?;
            ensure!(
                Some(digest(&manifest)) == decision.capture_manifest_sha256,
                "capture manifest changed"
            );
            let capture: Capture = serde_json::from_slice(&manifest)?;
            let raw = read(&directory.join("history.json"))?;
            ensure!(
                capture.plan == *plan && digest(&raw) == capture.raw_sha256,
                "capture identity/hash mismatch"
            );
            ensure!(
                capture.evaluation == evaluate(plan, &capture.receipt, &raw),
                "capture evaluation mismatch"
            );
            ensure!(
                decision.evaluation
                    == frozen_evaluation(plan, Some(&capture), decision.frozen_at_utc)?,
                "decision evaluation mismatch"
            );
        } else {
            ensure!(
                decision.capture_manifest_sha256.is_none(),
                "unbound capture hash"
            );
            ensure!(
                decision.evaluation == frozen_evaluation(plan, None, decision.frozen_at_utc)?,
                "missing decision mismatch"
            );
        }
        Ok(Some(decision))
    }

    pub fn freeze(
        &self,
        plan: &Plan,
        capture: Option<&ArchivedCapture>,
        now: DateTime<Utc>,
    ) -> Result<DailyDecision> {
        if let Some(existing) = self.load(plan)? {
            return Ok(existing);
        }
        if let Some(capture) = capture {
            ensure!(capture.capture.plan == *plan, "capture plan mismatch");
            let directory = self.root.join("captures").join(capture.id.to_string());
            ensure!(
                digest(&read_bounded(&directory.join("manifest.json"))?) == capture.manifest_sha256,
                "capture not in this store or changed"
            );
            ensure!(
                digest(&read_bounded(&directory.join("history.json"))?)
                    == capture.capture.raw_sha256,
                "archived raw changed"
            );
        }
        let decision = DailyDecision {
            plan: plan.clone(),
            frozen_at_utc: now,
            frozen_at_msk: msk(now),
            evaluation: frozen_evaluation(plan, capture.map(|c| &c.capture), now)?,
            capture_id: capture.map(|c| c.id),
            capture_manifest_sha256: capture.map(|c| c.manifest_sha256.clone()),
        };
        let path = self
            .root
            .join("decisions")
            .join(format!("{}.json", plan.decision_date));
        match publish_once(&path, &serde_json::to_vec(&decision)?) {
            Ok(()) => self.load(plan)?.context("decision missing after publish"),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                self.load(plan)?.ok_or(error)
            }
            Err(error) => Err(error),
        }
    }
}

fn frozen_evaluation(
    plan: &Plan,
    capture: Option<&Capture>,
    now: DateTime<Utc>,
) -> Result<Evaluation> {
    ensure!(now >= plan.at(6, 45), "freeze before capture window");
    if let Some(capture) = capture {
        ensure!(
            now >= capture.receipt.durable_at_utc,
            "freeze before durable receipt"
        );
    }
    if now > plan.at(6, 55) {
        return Ok(Evaluation::rejected(Reason::LateFreeze));
    }
    if let Some(capture) = capture {
        if capture.evaluation.valid_snapshot() {
            return Ok(capture.evaluation.clone());
        }
    }
    ensure!(
        now >= plan.at(6, 55),
        "retry window remains; do not freeze an outage early"
    );
    Ok(capture
        .map(|c| c.evaluation.clone())
        .unwrap_or_else(|| Evaluation::rejected(Reason::Missing)))
}

#[cfg(test)]
mod tests;
