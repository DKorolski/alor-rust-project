use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use redis::streams::StreamRangeReply;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{MAX_AUDIT_ROWS, MAX_ROWS, VERSION, config::SourceConfig, digest};
use crate::{models::BarEvent, terminal_bar::Release};

const MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub envelope: Value,
    pub release: Option<Release>,
}
impl Record {
    pub fn new(
        config: &SourceConfig,
        bar: &BarEvent,
        proof: Value,
        release: Option<Release>,
        now: i64,
    ) -> Result<Self> {
        ensure!(
            bar.symbol == config.symbol && bar.close_time_utc > 0 && bar.close_time_utc % 600 == 0,
            "invalid source ID label"
        );
        let mut payload = serde_json::to_value(bar)?;
        payload["source_proof"] = proof;
        Ok(Self {
            id: format!("{}-0", bar.close_time_utc * 1000),
            envelope: json!({"schema_version":1,"ts_utc":now,"source":VERSION,"msg_type":"bar","payload":payload}),
            release,
        })
    }
    pub fn bar(&self) -> Result<BarEvent> {
        Ok(serde_json::from_value(self.envelope["payload"].clone())?)
    }
    pub fn payload(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.envelope)?)
    }
}

fn regular(path: &Path) -> Result<()> {
    ensure!(
        !path.is_symlink() && path.is_file(),
        "regular source evidence required"
    );
    Ok(())
}
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
pub fn publish_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut f, value)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    sync_dir(path.parent().context("missing parent")?)
}

pub struct Store {
    pub root: PathBuf,
    pub records: BTreeMap<i64, Record>,
    pub audit_rows: usize,
    pub review_dates: std::collections::BTreeSet<chrono::NaiveDate>,
    audit_seen: std::collections::BTreeSet<String>,
    head: String,
    attempt: usize,
    _lock: File,
}

impl Store {
    pub fn open(root: &Path, config: &SourceConfig, mode: &str, now: i64) -> Result<Self> {
        config.validate(now)?;
        ensure!(
            root.is_absolute()
                && root.parent().context("state parent")?.canonicalize()? == root.parent().unwrap(),
            "canonical state path required"
        );
        let create = mode == "create";
        ensure!(
            create || mode == "resume",
            "explicit create/resume required"
        );
        if create {
            fs::create_dir(root)?;
        } else {
            ensure!(
                root.canonicalize()? == root,
                "canonical existing state required"
            );
        }
        ensure!(
            !(root.join("FAILED.json").exists() || root.join("FAILED.json").is_symlink()),
            "failed source cannot resume"
        );
        let lock_path = root.join("writer.lock");
        ensure!(!lock_path.is_symlink(), "invalid lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock.try_lock().context("source writer already running")?;
        let manifest = json!({"version":VERSION,"config":config,
            "config_sha256":digest(&serde_json::to_value(config)?)?});
        if create {
            publish_new(&root.join("manifest.json"), &manifest)?;
        } else {
            regular(&root.join("manifest.json"))?;
            ensure!(
                fs::metadata(root.join("manifest.json"))?.len() <= 65536,
                "manifest budget"
            );
            ensure!(
                serde_json::from_slice::<Value>(&fs::read(root.join("manifest.json"))?)?
                    == manifest,
                "source identity changed"
            );
        }
        let mut s = Self {
            root: root.into(),
            records: BTreeMap::new(),
            audit_rows: 0,
            review_dates: Default::default(),
            audit_seen: Default::default(),
            head: "genesis".into(),
            attempt: 1,
            _lock: lock,
        };
        let journal = root.join("committed.jsonl");
        if journal.exists() {
            regular(&journal)?;
            ensure!(
                journal.metadata()?.len() <= MAX_BYTES,
                "source journal budget"
            );
            ensure!(
                fs::read(&journal)?.last() == Some(&b'\n'),
                "torn source journal tail"
            );
            for line in BufReader::new(File::open(&journal)?).split(b'\n') {
                let bytes = line?;
                ensure!(
                    !bytes.is_empty() && bytes.len() <= 16384,
                    "torn/oversized source journal"
                );
                let item: Value = serde_json::from_slice(&bytes)?;
                let row: Record = serde_json::from_value(item["record"].clone())?;
                ensure!(
                    item["previous"] == s.head
                        && item["sha256"] == digest(&(s.head.clone(), &row))?,
                    "source journal checksum/chain"
                );
                let b = row.bar()?;
                s.validate_row(config, &row, &b)?;
                ensure!(
                    row.envelope["ts_utc"].as_i64().is_some_and(|ts| ts <= now),
                    "future canonical publication"
                );
                ensure!(
                    s.records
                        .last_key_value()
                        .is_none_or(|(ts, _)| *ts < b.close_time_utc),
                    "unordered/duplicate source journal"
                );
                s.head = item["sha256"].as_str().context("source hash")?.into();
                s.records.insert(b.close_time_utc, row);
                ensure!(s.records.len() <= MAX_ROWS, "source row budget");
            }
        }
        let audit = root.join("revisions.jsonl");
        if audit.exists() {
            regular(&audit)?;
            ensure!(audit.metadata()?.len() <= 1024 * 1024, "revision budget");
            ensure!(
                fs::read(&audit)?.last() == Some(&b'\n'),
                "torn revision audit tail"
            );
            for line in BufReader::new(File::open(audit)?).split(b'\n') {
                let line = line?;
                ensure!(
                    !line.is_empty() && line.len() <= 16384,
                    "torn revision audit"
                );
                let item: Value = serde_json::from_slice(&line)?;
                ensure!(
                    item["sha256"] == digest(&item["event"])?,
                    "revision checksum"
                );
                s.validate_audit(&item["event"], now)?;
                let identity = digest(&item["event"]["identity"])?;
                ensure!(s.audit_seen.insert(identity), "duplicate revision audit");
                s.audit_rows += 1;
                ensure!(s.audit_rows <= MAX_AUDIT_ROWS, "revision row budget");
                if item["event"]["review_required"] == true {
                    let date =
                        super::local(item["event"]["label"].as_i64().context("revision label")?)?
                            .date();
                    s.review_dates.insert(date);
                }
            }
        }
        let mut attempt_files = std::collections::BTreeSet::new();
        for (index, entry) in fs::read_dir(root)?.enumerate() {
            ensure!(index < 300, "source directory budget");
            let p = entry?.path();
            regular(&p)?;
            let name = p.file_name().unwrap().to_str().context("source filename")?;
            ensure!(
                matches!(
                    name,
                    "writer.lock"
                        | "manifest.json"
                        | "committed.jsonl"
                        | "revisions.jsonl"
                        | "HEALTH.json"
                ) || name.ends_with(".started.json")
                    || name.ends_with(".stopped.json"),
                "unknown/pending source evidence requires audit"
            );
            if name.ends_with(".started.json") || name.ends_with(".stopped.json") {
                ensure!(p.metadata()?.len() <= 4096, "attempt file budget");
                attempt_files.insert(name.to_string());
            }
        }
        while root.join(format!("{:04}.started.json", s.attempt)).exists() {
            let start = root.join(format!("{:04}.started.json", s.attempt));
            let stop = root.join(format!("{:04}.stopped.json", s.attempt));
            ensure!(stop.exists(), "incomplete source attempt cannot resume");
            let before: Value = serde_json::from_slice(&fs::read(&start)?)?;
            let record: Value = serde_json::from_slice(&fs::read(&stop)?)?;
            ensure!(
                record["reason"] == "operator_stop"
                    && before["attempt"] == s.attempt
                    && record["attempt"] == s.attempt,
                "invalid source stop"
            );
            attempt_files.remove(start.file_name().unwrap().to_str().unwrap());
            attempt_files.remove(stop.file_name().unwrap().to_str().unwrap());
            s.attempt += 1;
            ensure!(s.attempt <= 128, "source attempt budget");
        }
        ensure!(
            attempt_files.is_empty(),
            "orphan/unknown source attempt evidence"
        );
        publish_new(
            &root.join(format!("{:04}.started.json", s.attempt)),
            &json!({"attempt":s.attempt,"started_at_utc":now}),
        )?;
        Ok(s)
    }

    fn validate_row(&self, config: &SourceConfig, row: &Record, bar: &BarEvent) -> Result<()> {
        ensure!(
            config.eligible(bar.close_time_utc)?
                && bar.symbol == config.symbol
                && row.id == format!("{}-0", bar.close_time_utc * 1000)
                && row.envelope["source"] == VERSION
                && row.envelope["schema_version"] == 1
                && row.envelope["msg_type"] == "bar",
            "invalid canonical source identity"
        );
        ensure!(
            [bar.o, bar.h, bar.l, bar.c, bar.v]
                .iter()
                .all(|n| n.is_finite())
                && bar.l > 0.
                && bar.l <= bar.o.min(bar.c)
                && bar.h >= bar.o.max(bar.c)
                && bar.v >= 0.,
            "invalid canonical OHLCV"
        );
        let day = super::local(bar.close_time_utc)?.date();
        let historical = day < config.first_live_session || config.context_day(day);
        ensure!(
            bar.origin
                == if historical {
                    crate::models::DataOrigin::History
                } else {
                    crate::models::DataOrigin::Live
                },
            "canonical origin/calendar mismatch"
        );
        let published = row.envelope["ts_utc"]
            .as_i64()
            .context("publication clock missing")?;
        if let Some(release) = &row.release {
            let mut raw = release.bar.clone();
            raw.origin = bar.origin;
            ensure!(
                serde_json::to_value(&raw)? == serde_json::to_value(bar)?,
                "release/envelope quote mismatch"
            );
            ensure!(
                row.envelope["payload"]["source_proof"]["release_id"] == release.id
                    && row.envelope["payload"]["source_proof"]["release"]
                        == serde_json::to_value(&release.proof)?,
                "release proof mismatch"
            );
            config.finalizer()?.restore_committed(
                std::slice::from_ref(release),
                crate::terminal_bar::Clock {
                    utc: published,
                    monotonic_ms: 0,
                },
            )?;
        } else {
            let proof = &row.envelope["payload"]["source_proof"];
            let received = proof["received_at_utc"]
                .as_i64()
                .context("historical receipt missing")?;
            let mode = config.history_import_mode(bar.close_time_utc, received, published)?;
            ensure!(
                historical
                    && proof["kind"] == "historical_ws_import"
                    && proof["import_mode"] == mode
                    && proof["publication_started_at_utc"] == published
                    && proof["generation"].as_u64().is_some_and(|g| g > 0),
                "invalid historical receipt"
            );
        }
        Ok(())
    }

    pub async fn verify_prefix(
        &self,
        config: &SourceConfig,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<()> {
        let rows: StreamRangeReply = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            redis::cmd("XRANGE")
                .arg(&config.stream)
                .arg("-")
                .arg("+")
                .arg("COUNT")
                .arg(MAX_ROWS + 1)
                .query_async(conn),
        )
        .await??;
        ensure!(
            rows.ids.len() == self.records.len(),
            "canonical Redis prefix missing/extra/trimmed"
        );
        for (stored, expected) in rows.ids.iter().zip(self.records.values()) {
            ensure!(
                stored.id == expected.id
                    && stored.map.len() == 1
                    && stored
                        .get::<String>("payload")
                        .context("missing source payload")?
                        == expected.payload()?,
                "canonical Redis payload changed"
            );
        }
        Ok(())
    }

    pub async fn append(
        &mut self,
        config: &SourceConfig,
        row: Record,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<()> {
        let bar = row.bar()?;
        self.validate_row(config, &row, &bar)?;
        ensure!(
            self.records.len() < MAX_ROWS
                && self
                    .records
                    .last_key_value()
                    .is_none_or(|(ts, _)| *ts < bar.close_time_utc),
            "source order/capacity"
        );
        let path = self.root.join("pending.json");
        let len: u64 = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            redis::cmd("XLEN").arg(&config.stream).query_async(conn),
        )
        .await??;
        ensure!(
            len == self.records.len() as u64,
            "canonical source length changed before append"
        );
        publish_new(&path, &json!({"previous":self.head,"record":row}))?;
        let payload = row.payload()?;
        let result: Result<redis::RedisResult<String>, _> = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            redis::cmd("XADD")
                .arg(&config.stream)
                .arg(&row.id)
                .arg("payload")
                .arg(&payload)
                .query_async(conn),
        )
        .await;
        let known = matches!(result,Ok(Ok(ref id)) if id==&row.id);
        if !known {
            // Never retry an uncertain append: read back its exact canonical ID.
            let reply: StreamRangeReply = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                redis::cmd("XRANGE")
                    .arg(&config.stream)
                    .arg(&row.id)
                    .arg(&row.id)
                    .arg("COUNT")
                    .arg(1)
                    .query_async(conn),
            )
            .await
            .context("publication outcome unknown")??;
            ensure!(
                reply.ids.len() == 1
                    && reply.ids[0].map.len() == 1
                    && reply.ids[0].get::<String>("payload").as_deref() == Some(payload.as_str()),
                "publication outcome unknown/conflicting"
            );
        }
        let sha = digest(&(self.head.clone(), &row))?;
        let bytes = serde_json::to_vec(&json!({"previous":self.head,"record":row,"sha256":sha}))?;
        let journal = self.root.join("committed.jsonl");
        ensure!(
            bytes.len() <= 16384
                && fs::metadata(&journal).map(|m| m.len()).unwrap_or(0) + (bytes.len() as u64)
                    < MAX_BYTES,
            "source byte budget"
        );
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&journal)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::remove_file(path)?;
        sync_dir(&self.root)?;
        self.head = sha;
        self.records.insert(bar.close_time_utc, row);
        Ok(())
    }

    pub fn audit(
        &mut self,
        old: &BarEvent,
        new: &BarEvent,
        received: i64,
        generation: u64,
    ) -> Result<()> {
        let price = [old.o, old.h, old.l, old.c] != [new.o, new.h, new.l, new.c];
        if !price && old.v == new.v {
            return Ok(());
        }
        let identity = json!({"original":old,"observed":new});
        let id = digest(&identity)?;
        if self.audit_seen.contains(&id) {
            return Ok(());
        }
        let event = json!({"identity":identity,"label":new.close_time_utc,"kind":if price{"price_revision"}else{"volume_only_revision"},
            "received_at_utc":received,"generation":generation,"review_required":price});
        self.validate_audit(&event, received)?;
        let bytes = serde_json::to_vec(&json!({"sha256":digest(&event)?,"event":event}))?;
        let path = self.root.join("revisions.jsonl");
        ensure!(
            self.audit_rows < MAX_AUDIT_ROWS
                && fs::metadata(&path).map(|m| m.len()).unwrap_or(0) + (bytes.len() as u64)
                    < 1024 * 1024,
            "revision capacity"
        );
        let mut f = OpenOptions::new().append(true).create(true).open(path)?;
        f.write_all(&bytes)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        sync_dir(&self.root)?;
        self.audit_seen.insert(id);
        self.audit_rows += 1;
        if price {
            self.review_dates
                .insert(super::local(new.close_time_utc)?.date());
        }
        Ok(())
    }

    fn validate_audit(&self, event: &Value, now: i64) -> Result<()> {
        let original: BarEvent = serde_json::from_value(event["identity"]["original"].clone())?;
        let observed: BarEvent = serde_json::from_value(event["identity"]["observed"].clone())?;
        let stored = self
            .records
            .get(&original.close_time_utc)
            .context("revision lacks canonical original")?;
        ensure!(
            serde_json::to_value(stored.bar()?)? == serde_json::to_value(&original)?
                && observed.symbol == original.symbol
                && observed.close_time_utc == original.close_time_utc
                && event["label"] == observed.close_time_utc,
            "revision/canonical binding mismatch"
        );
        ensure!(
            [observed.o, observed.h, observed.l, observed.c, observed.v]
                .iter()
                .all(|x| x.is_finite())
                && observed.l > 0.
                && observed.l <= observed.o.min(observed.c)
                && observed.h >= observed.o.max(observed.c)
                && observed.v >= 0.,
            "invalid revision OHLCV"
        );
        let price = [original.o, original.h, original.l, original.c]
            != [observed.o, observed.h, observed.l, observed.c];
        ensure!(
            (price || original.v != observed.v)
                && event["review_required"] == price
                && event["kind"]
                    == if price {
                        "price_revision"
                    } else {
                        "volume_only_revision"
                    }
                && event["generation"].as_u64().is_some_and(|n| n > 0)
                && event["received_at_utc"]
                    .as_i64()
                    .is_some_and(|ts| ts >= original.close_time_utc + 600 && ts <= now),
            "invalid revision kind/receipt/generation"
        );
        Ok(())
    }

    pub fn health(&self, value: &Value) -> Result<()> {
        let tmp = self.root.join(".HEALTH.tmp");
        publish_new(&tmp, value)?;
        fs::rename(tmp, self.root.join("HEALTH.json"))?;
        sync_dir(&self.root)
    }
    pub fn fail(&self, class: &str, error: &str, now: i64) {
        let _ = publish_new(
            &self.root.join("FAILED.json"),
            &json!({"class":class,"error_sha256":digest(&error).ok(),"observed_at_utc":now}),
        );
    }
    pub fn stopped(&self, now: i64) -> Result<()> {
        publish_new(
            &self.root.join(format!("{:04}.stopped.json", self.attempt)),
            &json!({"attempt":self.attempt,"reason":"operator_stop","stopped_at_utc":now}),
        )
    }
}
