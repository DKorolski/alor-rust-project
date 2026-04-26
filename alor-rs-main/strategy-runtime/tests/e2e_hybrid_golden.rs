use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn hybrid_replay_mini_golden_passes_check() {
    let bin = env!("CARGO_BIN_EXE_hybrid_replay");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bundle_dir = manifest_dir.join("tests/fixtures/hybrid_mini_bundle");
    assert!(
        bundle_dir.exists(),
        "missing fixture bundle: {}",
        bundle_dir.display()
    );

    let out_dir = tempdir().expect("temp dir");
    let output = Command::new(bin)
        .args([
            "--bundle-dir",
            bundle_dir.to_str().expect("utf-8 path"),
            "--split",
            "golden",
            "--out-dir",
            out_dir.path().to_str().expect("utf-8 path"),
            "--check",
            "--strict",
            "--assert-gap-flatten",
        ])
        .output()
        .expect("run hybrid_replay");

    assert!(
        output.status.success(),
        "hybrid_replay failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn hybrid_replay_gap_flatten_assertion_fails_on_next_day_eod_fill() {
    let bin = env!("CARGO_BIN_EXE_hybrid_replay");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source_bundle = manifest_dir.join("tests/fixtures/hybrid_mini_bundle");
    assert!(
        source_bundle.exists(),
        "missing fixture bundle: {}",
        source_bundle.display()
    );

    let temp = tempdir().expect("temp dir");
    let bundle_dir = temp.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("bundle dir");

    let prepared_path = source_bundle.join("prepared_golden.csv");
    let prepared = fs::read_to_string(&prepared_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", prepared_path.display()));
    let prepared_without_same_day_fill = prepared
        .lines()
        .filter(|line| !line.starts_with("2026-01-08 23:40:00,"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(
        bundle_dir.join("prepared_golden.csv"),
        prepared_without_same_day_fill,
    )
    .expect("write prepared fixture");

    let out_dir = temp.path().join("out");
    let output = Command::new(bin)
        .args([
            "--bundle-dir",
            bundle_dir.to_str().expect("utf-8 path"),
            "--split",
            "golden",
            "--out-dir",
            out_dir.to_str().expect("utf-8 path"),
            "--assert-gap-flatten",
        ])
        .output()
        .expect("run hybrid_replay");

    assert!(
        !output.status.success(),
        "hybrid_replay unexpectedly passed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_path = out_dir.join("parity_report_golden.json");
    let report = fs::read_to_string(&report_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", report_path.display()));
    assert!(report.contains("\"gap_flatten_violations\": 1"), "{report}");
    assert!(report.contains("2026-01-08 23:30:00"), "{report}");
    assert!(report.contains("2026-01-09 09:00:00"), "{report}");
}

#[test]
fn hybrid_replay_baseline_skip_does_not_fill_pending_exit_on_weekend_bar() {
    let bin = env!("CARGO_BIN_EXE_hybrid_replay");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source_bundle = manifest_dir.join("tests/fixtures/hybrid_mini_bundle");

    let temp = tempdir().expect("temp dir");
    let bundle_dir = temp.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("bundle dir");

    let prepared_path = source_bundle.join("prepared_golden.csv");
    let prepared = fs::read_to_string(&prepared_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", prepared_path.display()));
    let mut rows = Vec::new();
    for line in prepared.lines() {
        if line.starts_with("2026-01-08 23:40:00,") || line.starts_with("2026-01-09 ") {
            continue;
        }
        rows.push(line.to_string());
        if line.starts_with("2026-01-08 23:30:00,") {
            rows.push(
                "2026-01-10 10:00:00,2725.5,2726.5,2725.0,2725.5,100,2765.0,2741.5,2754.0,23.5"
                    .to_string(),
            );
        }
    }
    fs::write(
        bundle_dir.join("prepared_golden.csv"),
        rows.join("\n") + "\n",
    )
    .expect("write prepared fixture");

    let out_dir = temp.path().join("out");
    let output = Command::new(bin)
        .args([
            "--bundle-dir",
            bundle_dir.to_str().expect("utf-8 path"),
            "--split",
            "golden",
            "--out-dir",
            out_dir.to_str().expect("utf-8 path"),
        ])
        .output()
        .expect("run hybrid_replay");

    assert!(
        output.status.success(),
        "hybrid_replay failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let trades_path = out_dir.join("actual_trades_golden.csv");
    let trades = fs::read_to_string(&trades_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", trades_path.display()));
    assert!(!trades.contains("2026-01-10 10:00:00"), "{trades}");
    assert!(trades.contains("2026-01-12 09:00:00"), "{trades}");
}

#[test]
fn hybrid_replay_reports_imoexf_bo_k053_profile() {
    let bin = env!("CARGO_BIN_EXE_hybrid_replay");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bundle_dir = manifest_dir.join("tests/fixtures/hybrid_mini_bundle");

    let out_dir = tempdir().expect("temp dir");
    let output = Command::new(bin)
        .args([
            "--bundle-dir",
            bundle_dir.to_str().expect("utf-8 path"),
            "--split",
            "golden",
            "--out-dir",
            out_dir.path().to_str().expect("utf-8 path"),
            "--profile",
            "imoexf_bo_k053",
        ])
        .output()
        .expect("run hybrid_replay");

    assert!(
        output.status.success(),
        "hybrid_replay failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_path = out_dir.path().join("parity_report_golden.json");
    let report = fs::read_to_string(&report_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", report_path.display()));
    assert!(
        report.contains("\"profile\": \"imoexf_bo_k053\""),
        "{report}"
    );
}
