use super::*;

fn retained_root(output: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    ensure!(
        relative.components().count() == 2
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && relative.file_name().is_some_and(|n| n == "generation"),
        "only an owned fixture generation is allowed"
    );
    let root = output.join(relative);
    let base = fs::canonicalize(output)?;
    ensure!(
        !fs::symlink_metadata(root.parent().unwrap())?
            .file_type()
            .is_symlink()
            && !fs::symlink_metadata(&root)?.file_type().is_symlink()
            && fs::canonicalize(&root)?.starts_with(base),
        "fixture generation escapes output"
    );
    Ok(root)
}

#[test]
fn continuous_recovery_rejects_external_generation() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "/tmp/generation",
        "../generation",
        "generation",
        "x/../generation",
    ] {
        assert!(retained_root(dir.path(), path).is_err());
    }
    fs::create_dir_all(dir.path().join("owned/generation")).unwrap();
    assert!(retained_root(dir.path(), "owned/generation").is_ok());
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), dir.path().join("link")).unwrap();
        assert!(retained_root(dir.path(), "link/generation").is_err());
    }
}

#[test]
#[ignore = "explicit new-process recovery of an owned historical fixture; no Redis/network"]
fn reaction_continuous_process_recovery_actual() {
    let input = fs::read(std::env::var("REACTION_CONTINUOUS_INPUT").unwrap()).unwrap();
    let f: Fixture = serde_json::from_slice(&input).unwrap();
    f.validate().unwrap();
    let output = PathBuf::from(std::env::var("REACTION_CONTINUOUS_OUTPUT").unwrap());
    let prior: Value =
        serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    let binary = binary_hash().unwrap();
    assert_eq!(prior["result"], "EXPORTED_CONTINUOUS_CANDIDATE");
    assert_eq!(prior["evidence_kind"], EVIDENCE);
    assert_eq!(
        prior["input_sha256"],
        format!("{:x}", Sha256::digest(&input))
    );
    assert_eq!(prior["binary_sha256"], binary);
    let witness = &prior["recovery_witness"];
    assert_eq!(
        witness["final_clock"].as_i64(),
        f.operations.last().map(Input::now)
    );
    let root = retained_root(&output, prior["retained_generation"].as_str().unwrap()).unwrap();
    // Create-new before opening: repeated measurement cannot silently overwrite evidence.
    let mut file = create_file(&output.join("process_recovery.json")).unwrap();
    let before = usage(&root).unwrap();
    now(witness["final_clock"].as_i64().unwrap());
    let started = Instant::now();
    let r = ReactionShadowRunner::open_with_clock(&root, f.spec, clock).unwrap();
    let duration = started.elapsed().as_secs_f64();
    assert_eq!(serde_json::to_value(r.status()).unwrap(), witness["status"]);
    assert_eq!(
        r.state.model_hash().unwrap(),
        witness["model_sha256"].as_str().unwrap()
    );
    assert_eq!(r.log.head(), witness["head_sha256"].as_str().unwrap());
    assert_eq!(Some(r.log.count()), witness["transactions"].as_u64());
    assert_eq!(usage(&root).unwrap(), before);
    assert!(!r.status().order_emission_allowed);
    let report = serde_json::json!({
        "schema_version":"reaction_process_recovery_v1", "evidence_kind":EVIDENCE,
        "result":"PASS_NEW_PROCESS_RESTORE", "input_sha256":prior["input_sha256"],
        "binary_sha256":binary, "duration_sec":duration,"rss_after_kib":rss_kib(),
        "directory_bytes":before.0,"directory_files":before.1,
        "witness":witness,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "debug_assertions":cfg!(debug_assertions),"new_process":true,"cold_filesystem_cache":false,
        "deployment":false,"production_receipts_certified":false,
    });
    serde_json::to_writer_pretty(&mut file, &report).unwrap();
    writeln!(file).unwrap();
    file.sync_all().unwrap();
}
