//! Historical prices with a declared fixture clock. Never production receipts.
//! Expected model states are not supplied to this exporter.
use super::*;
use std::io::{BufWriter, Write};
use std::time::Instant;

const SCHEMA: &str = "reaction_continuous_input_v1";
const EVIDENCE: &str = "historical_continuous_fixture_clock_not_production_receipts";

mod recovery;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: String,
    evidence_kind: String,
    spec: RunnerSpec,
    created_at_utc: i64,
    operations: Vec<Input>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Receipt { bar: BarEvent, now: i64 },
    Freeze { day: NaiveDate, now: i64 },
    Restart { now: i64 },
    SessionEnd { day: NaiveDate, now: i64 },
}
impl Input {
    fn now(&self) -> i64 {
        match self {
            Self::Receipt { now, .. }
            | Self::Freeze { now, .. }
            | Self::Restart { now }
            | Self::SessionEnd { now, .. } => *now,
        }
    }
}
impl Fixture {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == SCHEMA && self.evidence_kind == EVIDENCE,
            "wrong fixture schema/evidence"
        );
        self.spec.header()?;
        ensure!(
            !self.operations.is_empty() && self.operations.len() <= 6500,
            "fixture size limit"
        );
        let mut last = self.created_at_utc;
        let mut label = None;
        for op in &self.operations {
            ensure!(op.now() >= last, "fixture clock regression");
            last = op.now();
            if let Input::Receipt { bar, now } = op {
                ensure!(
                    bar.origin == DataOrigin::Live
                        && bar.symbol == self.spec.ingress.symbol
                        && *now == bar.close_time_utc + 600,
                    "not the declared fixture receipt clock"
                );
                ensure!(
                    label.is_none_or(|old| bar.close_time_utc > old),
                    "unordered fixture prices"
                );
                label = Some(bar.close_time_utc);
            }
        }
        Ok(())
    }
}
fn usage(root: &Path) -> Result<(u64, usize)> {
    let (mut bytes, mut files) = (0, 0);
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let (b, f) = usage(&entry.path())?;
            bytes += b;
            files += f;
        } else {
            ensure!(entry.file_type()?.is_file(), "non-regular generation file");
            bytes += entry.metadata()?.len();
            files += 1;
        }
    }
    Ok((bytes, files))
}
fn rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
fn line(file: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    writeln!(file)?;
    Ok(())
}
fn create_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}
fn binary_hash() -> Result<String> {
    use std::io::Read;
    let mut file = File::open(std::env::current_exe()?)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[test]
fn continuous_export_rejects_wrong_clock_schema_and_order() {
    let b = bar("IMOEXF", epoch("2026-10-26 07:00"), 100.1);
    let mut f = Fixture {
        schema_version: SCHEMA.into(),
        evidence_kind: EVIDENCE.into(),
        spec: spec("IMOEXF"),
        created_at_utc: epoch("2026-10-22 06:00"),
        operations: vec![Input::Receipt {
            bar: b.clone(),
            now: b.close_time_utc + 600,
        }],
    };
    f.validate().unwrap();
    f.operations.push(Input::Receipt {
        bar: b.clone(),
        now: b.close_time_utc + 600,
    });
    assert!(f.validate().is_err());
    f.operations.pop();
    f.operations[0] = Input::Receipt {
        bar: b.clone(),
        now: b.close_time_utc + 601,
    };
    assert!(f.validate().is_err());
    f.operations[0] = Input::Receipt {
        bar: b.clone(),
        now: b.close_time_utc + 600,
    };
    f.evidence_kind = "production_receipts".into();
    assert!(f.validate().is_err());
}

#[test]
#[ignore = "explicit continuous fsync/recovery export; fixture clock, no Redis/network"]
fn reaction_continuous_export_actual() {
    let input = std::env::var("REACTION_CONTINUOUS_INPUT").expect("explicit input");
    let bytes = fs::read(input).unwrap();
    assert!(bytes.len() <= 8 * 1024 * 1024);
    let fixture: Fixture = serde_json::from_slice(&bytes).unwrap();
    fixture.validate().unwrap();
    let output =
        PathBuf::from(std::env::var("REACTION_CONTINUOUS_OUTPUT").expect("new output directory"));
    fs::create_dir(&output).unwrap();
    let dir = tempfile::tempdir_in(&output).unwrap();
    let root = dir.path().join("generation");
    let mut trace = BufWriter::new(create_file(&output.join("trace.jsonl")).unwrap());
    now(fixture.created_at_utc);
    let mut r =
        ReactionShadowRunner::create_with_clock(&root, fixture.spec.clone(), clock).unwrap();
    let started = Instant::now();
    let mut samples = vec![];
    let mut restarts = vec![];
    let mut latencies = vec![];
    let mut released = 0;
    let mut last_index = None;
    let mut recovery_witness = None;
    let mut report = serde_json::json!({
        "schema_version":"reaction_continuous_actual_v1", "evidence_kind":EVIDENCE,
        "input_sha256":format!("{:x}",Sha256::digest(&bytes)),
        "binary_sha256":binary_hash().unwrap(),
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,"debug_assertions":cfg!(debug_assertions),
        "rss_before_kib":rss_kib(),"production_receipts_certified":false,"redis_poll_certified":false,
        "numerical_parity_certified":false,"deployment":false,
    });
    let result = (|| -> Result<()> {
        for (index, op) in fixture.operations.iter().enumerate() {
            last_index = Some(index);
            now(op.now());
            let op_started = Instant::now();
            match op {
                Input::Receipt { bar, .. } => {
                    let through = r.collector.journal.last_sequence() + 1;
                    r.collector
                        .journal
                        .append(&format!("{through}-0"), &serde_json::to_string(bar)?)?
                        .input?;
                    r.commit(Operation::Ingest { through }, clock())?;
                    r.commit(Operation::Observe { through }, clock())?;
                    latencies.push(op_started.elapsed().as_secs_f64());
                }
                Input::Freeze { day, .. } => {
                    let raw = r.collector.journal.seal_cutoff(clock())?;
                    let cutoff = r.collector.project_cutoff(&raw)?;
                    r.commit(
                        Operation::Freeze {
                            date: *day,
                            cutoff: Box::new(cutoff),
                        },
                        clock(),
                    )?;
                }
                Input::Restart { .. } => {
                    let witness = r.state.model_hash()?;
                    let head = r.log.head().to_owned();
                    let count = r.log.count();
                    let before = usage(&root)?;
                    drop(r);
                    r = ReactionShadowRunner::open_with_clock(&root, fixture.spec.clone(), clock)?;
                    ensure!(
                        r.state.model_hash()? == witness
                            && r.log.head() == head
                            && r.log.count() == count,
                        "restart changed model/outbox"
                    );
                    ensure!(usage(&root)? == before, "restart changed directory usage");
                    let sample = serde_json::json!({"kind":"restart","input_index":index,
                        "model_sha256":witness,"head_sha256":head,"committed_transactions":count,
                        "directory_unchanged":true,"duration_sec":op_started.elapsed().as_secs_f64(),
                        "rss_sample_kib":rss_kib()});
                    line(&mut trace, &sample)?;
                    restarts.push(sample);
                }
                Input::SessionEnd { day, .. } => {
                    ensure!(
                        r.state.day == Some(*day)
                            && r.status().pair_ready
                            && r.status().session_complete,
                        "incomplete or unready paired session"
                    );
                    let (size, files) = usage(&root)?;
                    let sample = serde_json::json!({"kind":"session_end","input_index":index,"day":day,
                        "status":r.status(),"states":r.state.states()?,"directory_bytes":size,"directory_files":files,
                        "elapsed_sec":started.elapsed().as_secs_f64(),"rss_sample_kib":rss_kib()});
                    line(&mut trace, &sample)?;
                    samples.push(sample);
                    trace.flush()?;
                    println!(
                        "CONTINUOUS_SESSION {day} transactions={} bytes={size}",
                        r.log.count()
                    );
                }
            }
            for tx in released + 1..=r.log.count() {
                line(
                    &mut trace,
                    &serde_json::json!({"kind":"transaction","input_index":index,"output":r.committed_output(tx)?}),
                )?;
            }
            released = r.log.count();
        }
        recovery_witness = Some(serde_json::json!({
            "model_sha256":r.state.model_hash()?,
            "head_sha256":r.log.head(),"transactions":r.log.count(),
            "final_clock":clock(),"status":r.status(),
        }));
        Ok(())
    })();
    trace.flush().unwrap();
    trace.get_ref().sync_all().unwrap();
    report["duration_sec"] = started.elapsed().as_secs_f64().into();
    report["last_input_index"] = last_index.into();
    report["completed_sessions"] = samples.len().into();
    report["daily"] = samples.into();
    report["restarts"] = restarts.into();
    let (size, files) = usage(&root).unwrap();
    report["directory_bytes"] = size.into();
    report["directory_files"] = files.into();
    report["rss_after_kib"] = rss_kib().into();
    latencies.sort_by(f64::total_cmp);
    if !latencies.is_empty() {
        report["receipt_p50_sec"] = latencies[latencies.len() / 2].into();
        report["receipt_p95_sec"] = latencies[(latencies.len() - 1) * 95 / 100].into();
        report["receipt_max_sec"] = latencies[latencies.len() - 1].into();
    }
    let error = result.err().map(|e| format!("{e:#}"));
    report["result"] = if error.is_none() {
        "EXPORTED_CONTINUOUS_CANDIDATE"
    } else {
        "FAILED"
    }
    .into();
    report["failure"] = error.clone().into();
    if error.is_some() {
        report["failed_generation_retained"] = dir.keep().to_string_lossy().to_string().into();
    } else if std::env::var("REACTION_RETAIN_GENERATION").as_deref() == Ok("1") {
        report["retained_generation"] =
            root.strip_prefix(&output).unwrap().to_str().unwrap().into();
        report["recovery_witness"] = recovery_witness.unwrap();
        let _ = dir.keep();
    }
    let mut file = create_file(&output.join("report.json")).unwrap();
    serde_json::to_writer_pretty(&mut file, &report).unwrap();
    writeln!(file).unwrap();
    file.sync_all().unwrap();
    assert!(error.is_none(), "{}", error.unwrap_or_default());
}
