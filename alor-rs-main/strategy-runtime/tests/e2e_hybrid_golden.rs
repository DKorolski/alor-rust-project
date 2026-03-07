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
