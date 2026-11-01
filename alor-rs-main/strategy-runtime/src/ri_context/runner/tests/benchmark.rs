//! Synthetic storage/recovery workload, NOT an independent numerical reference.
//! The clock override stays behind cfg(test); no production receipt import exists.
use std::io::Write;
use std::time::Instant;

use super::*;

fn next_weekday(mut d: NaiveDate) -> NaiveDate {
    loop {
        d = d.succ_opt().unwrap();
        if chrono::Datelike::weekday(&d).num_days_from_monday() < 5 {
            return d;
        }
    }
}

fn workload_bar(day: NaiveDate, index: i64, warmup: bool) -> BarEvent {
    let label = epoch(&format!("{day} 07:00")) + index * 600;
    let close = if warmup {
        90000.
    } else {
        match index {
            0 => 89900.,
            1 => 90200.,
            11..=95 => 91000.,
            96 => 91100.,
            _ => 90000.,
        }
    };
    let mut b = bar(
        label,
        if warmup {
            DataOrigin::History
        } else {
            DataOrigin::Live
        },
        close,
    );
    if warmup {
        b.h = 91000.;
        b.l = 89000.;
    }
    b
}

fn directory_usage(path: &Path) -> Result<(u64, u64)> {
    let mut bytes = 0;
    let mut files = 0;
    for e in fs::read_dir(path)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let (b, f) = directory_usage(&e.path())?;
            bytes += b;
            files += f;
        } else {
            bytes += e.metadata()?.len();
            files += 1;
        }
    }
    Ok((bytes, files))
}

fn rss_sample_kib() -> Option<u64> {
    let result = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !result.status.success() {
        return None;
    }
    String::from_utf8(result.stdout).ok()?.trim().parse().ok()
}

#[test]
fn synthetic_workload_is_closed_bar_and_includes_model_eod() {
    let d = date("2026-10-13");
    let bars: Vec<_> = (0..=100).map(|i| workload_bar(d, i, false)).collect();
    assert_eq!(bars.len(), 101);
    assert_eq!(bars[96].close_time_utc, epoch("2026-10-13 23:00"));
    assert_eq!(bars[100].close_time_utc, epoch("2026-10-13 23:40"));
    assert!(bars
        .iter()
        .all(|b| b.l <= b.o && b.o <= b.h && b.l <= b.close && b.close <= b.h));
    assert_eq!(next_weekday(date("2026-10-09")), date("2026-10-12"));
}

#[test]
#[ignore = "explicit long local fsync benchmark; synthetic receipts, no Redis/network"]
fn ri_paired_runner_long_generation_benchmark() {
    let sessions: usize = std::env::var("RI_BENCH_SESSIONS")
        .unwrap_or_else(|_| "30".into())
        .parse()
        .unwrap();
    assert!((1..=30).contains(&sessions));
    let mut report_file = std::env::var_os("RI_BENCH_REPORT").map(|path| {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .expect("benchmark report must be NEW")
    });
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("synthetic-runner");
    let cfg = spec();
    set_clock(epoch("2026-10-09 06:00"));
    let mut r = RiShadowRunner::create_with_clock(&root, cfg.clone(), clock).unwrap();
    let mut report = serde_json::json!({
        "schema_version": "ri_runner_resource_probe_v1", "evidence_kind": "synthetic_clock_workload",
        "requested_sessions": sessions, "warmup_sessions": 2, "completed_sessions": 0,
        "numerical_parity_certified": false, "operational_receipt_parity_certified": false,
        "debug_assertions": cfg!(debug_assertions), "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "rss_before_kib": rss_sample_kib(), "restart_passed": false,
    });
    let mut latencies = vec![];
    let mut by_day = vec![];
    let started = Instant::now();
    let result = (|| -> Result<()> {
        let mut day = date("2026-10-09");
        for ordinal in 0..sessions + 2 {
            let day_started = Instant::now();
            let warmup = ordinal < 2;
            if !warmup {
                set_clock(epoch(&format!("{day} 07:00")));
                let barrier = r.journal.seal_cutoff(clock())?;
                r.commit(Operation::Freeze { date: day, barrier }, clock())?;
            }
            for index in 0..=100 {
                let event_started = Instant::now();
                let b = workload_bar(day, index, warmup);
                set_clock(b.close_time_utc + 600);
                let source_id = format!("{}-0", r.journal.last_sequence() + 1);
                r.journal
                    .append(&source_id, &serde_json::to_string(&b)?)?
                    .input?;
                let through = r.journal.last_sequence();
                r.commit(Operation::Ingest { through }, clock())?;
                r.commit(Operation::Observe { through }, clock())?;
                latencies.push(event_started.elapsed().as_secs_f64());
            }
            for a in &r.state.adapters {
                let trace = a.state_trace();
                ensure!(
                    trace["mr_position"].is_null()
                        && trace["bo_position"].is_null()
                        && trace["bo_pending"].is_null(),
                    "synthetic workload not flat at EOD"
                );
            }
            if !warmup {
                report["completed_sessions"] = (ordinal - 1).into();
            }
            let (bytes, files) = directory_usage(&root)?;
            let sample = serde_json::json!({"session": day, "warmup": warmup,
                "duration_sec": day_started.elapsed().as_secs_f64(), "bytes": bytes, "files": files,
                "rss_sample_kib": rss_sample_kib(), "transactions": r.status().committed_transactions});
            println!("RI_BENCH_DAY {sample}");
            by_day.push(sample);
            day = next_weekday(day);
        }
        Ok(())
    })();
    report["collection_sec"] = started.elapsed().as_secs_f64().into();
    report["daily"] = by_day.into();
    report["status_before_restart"] = serde_json::to_value(r.status()).unwrap();
    let (bytes, files) = directory_usage(&root).unwrap();
    report["directory_bytes"] = bytes.into();
    report["directory_files"] = files.into();
    latencies.sort_by(f64::total_cmp);
    if !latencies.is_empty() {
        report["event_p50_sec"] = latencies[latencies.len() / 2].into();
        report["event_p95_sec"] = latencies[(latencies.len() - 1) * 95 / 100].into();
        report["event_max_sec"] = latencies[latencies.len() - 1].into();
    }
    let mut failure = result.err().map(|e| format!("{e:#}"));
    if failure.is_none() {
        let expected: Vec<_> = r.state.adapters.iter().map(|a| a.state_trace()).collect();
        let count = r.status().committed_transactions;
        let last = r.committed_output(count).unwrap();
        drop(r);
        let restart = Instant::now();
        match RiShadowRunner::open_with_clock(&root, cfg, clock) {
            Ok(restored) => {
                let actual: Vec<_> = restored
                    .state
                    .adapters
                    .iter()
                    .map(|a| a.state_trace())
                    .collect();
                if actual == expected
                    && restored.status().committed_transactions == count
                    && restored.committed_output(count).unwrap() == last
                {
                    report["restart_passed"] = true.into();
                } else {
                    failure = Some("restart state/outbox mismatch".into());
                }
                report["rss_after_restart_kib"] = rss_sample_kib().into();
            }
            Err(e) => failure = Some(format!("restart failed: {e:#}")),
        }
        report["restart_sec"] = restart.elapsed().as_secs_f64().into();
        report["directory_unchanged_after_restart"] =
            (directory_usage(&root).unwrap() == (bytes, files)).into();
    }
    report["result"] = if failure.is_none() {
        "LOCAL_RESOURCE_REPLAY_COMPLETED"
    } else {
        "FAILED"
    }
    .into();
    report["failure"] = failure.clone().into();
    if let Some(file) = &mut report_file {
        serde_json::to_writer_pretty(&mut *file, &report).unwrap();
        writeln!(file).unwrap();
        file.sync_all().unwrap();
    }
    println!("RI_BENCH_RESULT {report}");
    assert!(failure.is_none(), "{}", failure.unwrap_or_default());
}
