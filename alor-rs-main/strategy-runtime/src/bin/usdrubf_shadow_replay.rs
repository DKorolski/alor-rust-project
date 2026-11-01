//! Offline raw-M10 parity harness, deliberately not registered as a live strategy.
use strategy_runtime::shadow_reaction::usdrubf as model;
#[cfg(test)]
#[path = "usdrubf_shadow_replay/tests.rs"]
mod tests;
#[path = "usdrubf_shadow_replay/trace.rs"]
mod trace;

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

fn write_csv<T: Serialize>(path: &Path, rows: impl Iterator<Item = T>) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn run(handoff: &Path, reference: &Path, output: &Path) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = parent
        .canonicalize()?
        .join(output.file_name().context("output directory name")?);
    ensure!(
        !destination.starts_with(handoff.canonicalize()?)
            && !destination.starts_with(reference.canonicalize()?),
        "output must be outside immutable handoff/reference directories"
    );
    let contract_bytes = fs::read(handoff.join("shadow_contract.json"))?;
    ensure!(
        trace::sha(&contract_bytes) == trace::CONTRACT,
        "unsupported frozen contract"
    );
    ensure!(
        trace::sha(&fs::read(handoff.join("reference_model.py"))?) == trace::REFERENCE,
        "unsupported frozen reference"
    );
    let spec: Value = serde_json::from_slice(&contract_bytes)?;
    let raw_bytes = fs::read(handoff.join("raw_model_input.csv"))?;
    let raw_hash = trace::sha(&raw_bytes);
    let raw: Vec<model::Bar> = csv::Reader::from_reader(raw_bytes.as_slice())
        .deserialize()
        .collect::<std::result::Result<_, _>>()?;
    model::validate_raw(&raw)?;
    ensure!(!output.exists(), "output must be a NEW directory");
    // create_dir, not create_dir_all: never reuse an old output or overwrite handoff files.
    fs::create_dir(output)?;
    let mut checks = Vec::new();
    for profile in spec["profiles"].as_array().context("profiles missing")? {
        let cfg: model::Config = serde_json::from_value(profile["engine"].clone())?;
        cfg.validate()?;
        let manifest_path = reference.join(&cfg.model_id).join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: Value = serde_json::from_slice(&manifest_bytes)?;
        ensure!(
            manifest["model_input_file_sha256"] == raw_hash,
            "raw file hash mismatch"
        );
        let identity = trace::identity(
            &cfg,
            profile["config_hash"].as_str().context("config hash")?,
        );
        ensure!(identity == manifest["identity"], "identity mismatch");
        let inputs = trace::verify_selection(&manifest, &raw, &cfg)?;
        let prepared: Vec<_> = raw.iter().filter(|b| cfg.eligible(b)).copied().collect();
        let directory = output.join(&cfg.model_id);
        fs::create_dir(&directory)?;
        let trace_path = directory.join("actual_state_trace.jsonl");
        let file = BufWriter::new(File::create(&trace_path)?);
        let mut writer = trace::Trace::new(file, identity, inputs);
        let mut engine = model::Engine::new(cfg);
        engine.replay(&prepared, |phase, engine, bar| {
            writer.emit(phase, engine, bar)
        })?;
        writer.finish()?;
        let start = chrono::NaiveDate::from_ymd_opt(2026, 7, 14).unwrap();
        write_csv(
            &directory.join("trades.csv"),
            engine.trades.iter().filter(|t| t.entry_ts.date() >= start),
        )?;
        write_csv(
            &directory.join("events.csv"),
            engine.events.iter().filter(|e| e.ts.date() >= start),
        )?;
        checks.push(json!({"model_id":engine.cfg.model_id,"prepared_inputs":prepared.len(),
            "raw_selection_checked":true,"trace_rows":writer.rows,"trace_sha256":trace::sha(&fs::read(&trace_path)?),
            "manifest_sha256":trace::sha(&manifest_bytes)}));
    }
    let report = json!({"status":"GENERATED_NOT_YET_COMPARED","checks":checks,"raw_file_sha256":raw_hash,
        "source_sha256":{"main":trace::sha(include_bytes!("usdrubf_shadow_replay.rs")),
            "model":trace::sha(include_bytes!("usdrubf_shadow_replay/model.rs")),
            "trace":trace::sha(include_bytes!("usdrubf_shadow_replay/trace.rs"))},
        "binary_sha256":trace::sha(&fs::read(std::env::current_exe()?)?),
        "rust_state_parity_certified":false,"operational_receipt_restart_certified":false,"deployment":false});
    let mut file = File::create(output.join("run_metadata.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() -> Result<()> {
    let mut handoff = None;
    let mut reference = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!("usdrubf_shadow_replay --handoff DIR --reference-state DIR --output NEW_DIR (offline only)");
            return Ok(());
        }
        let value = PathBuf::from(args.next().context("missing argument value")?);
        let target = match arg.as_str() {
            "--handoff" => &mut handoff,
            "--reference-state" => &mut reference,
            "--output" => &mut output,
            _ => bail!("unknown argument {arg}"),
        };
        ensure!(target.replace(value).is_none(), "duplicate argument {arg}");
    }
    run(
        &handoff.context("--handoff required")?,
        &reference.context("--reference-state required")?,
        &output.context("--output required")?,
    )
}
