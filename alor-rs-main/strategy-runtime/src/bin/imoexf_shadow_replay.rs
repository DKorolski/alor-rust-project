//! Offline numerical parity only. No runtime adapter, Redis or broker command path.
use strategy_runtime::shadow_reaction::imoexf as model;
#[cfg(test)]
#[path = "imoexf_shadow_replay/tests.rs"]
mod tests;
#[path = "imoexf_shadow_replay/trace.rs"]
mod trace;

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

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
        .join(output.file_name().context("output name")?);
    ensure!(
        !destination.starts_with(handoff.canonicalize()?)
            && !destination.starts_with(reference.canonicalize()?),
        "output must be outside immutable handoff/reference directories"
    );
    for (name, hash) in [
        ("shadow_contract.json", trace::CONTRACT),
        ("reference_model.py", trace::REFERENCE),
        ("cost_policy.json", trace::COSTS),
    ] {
        ensure!(
            trace::sha(&fs::read(handoff.join(name))?) == hash,
            "unsupported frozen file: {name}"
        );
    }
    let reference_manifest_hash = trace::sha(&fs::read(handoff.join("manifest.json"))?);
    let raw = fs::read(handoff.join("model_bars_07.csv"))?;
    let raw_hash = trace::sha(&raw);
    let bars: Vec<model::Bar> = csv::Reader::from_reader(raw.as_slice())
        .deserialize()
        .collect::<std::result::Result<_, _>>()?;
    model::validate_bars(&bars)?;
    let costs: model::Costs = serde_json::from_slice(&fs::read(handoff.join("cost_policy.json"))?)?;
    let mut manifests = Vec::new();
    // Validate both input manifests before writing any derivative output.
    for sleeve in ["mr", "combo"] {
        let bytes = fs::read(reference.join(sleeve).join("manifest.json"))?;
        let manifest: Value = serde_json::from_slice(&bytes)?;
        let inputs = trace::verify_inputs(
            &manifest,
            &bars,
            &raw_hash,
            sleeve,
            &reference_manifest_hash,
        )?;
        manifests.push((sleeve, bytes, inputs));
    }
    ensure!(!output.exists(), "output must be a NEW directory");
    fs::create_dir(output)?;
    let mut checks = Vec::new();
    for (sleeve, manifest_bytes, inputs) in manifests {
        let directory = output.join(sleeve);
        fs::create_dir(&directory)?;
        let path = directory.join("actual_state_trace.jsonl");
        let mut writer = trace::Trace::new(BufWriter::new(File::create(&path)?), sleeve, inputs);
        let mut engine = model::Engine::new(sleeve, costs.clone())?;
        engine.replay(&bars, |phase, e, b| writer.emit(phase, e, b))?;
        writer.finish()?;
        write_csv(
            &directory.join("trades.csv"),
            engine
                .trades
                .iter()
                .filter(|t| t.entry_ts.date() >= model::transition()),
        )?;
        write_csv(
            &directory.join("events.csv"),
            engine
                .events
                .iter()
                .filter(|e| e.ts.date() >= model::transition()),
        )?;
        checks.push(
            json!({"sleeve":sleeve,"prepared_inputs":bars.len(),"input_coverage_checked":true,
            "trace_rows":writer.rows,"trace_sha256":trace::sha(&fs::read(&path)?),
            "manifest_sha256":trace::sha(&manifest_bytes)}),
        );
    }
    let metadata = json!({"status":"GENERATED_NOT_YET_COMPARED","checks":checks,"raw_file_sha256":raw_hash,
        "source_sha256":{"main":trace::sha(include_bytes!("imoexf_shadow_replay.rs")),
            "model":trace::sha(include_bytes!("imoexf_shadow_replay/model.rs")),
            "trace":trace::sha(include_bytes!("imoexf_shadow_replay/trace.rs"))},
        "binary_sha256":trace::sha(&fs::read(std::env::current_exe()?)?),
        "rust_state_parity_certified":false,"operational_receipt_restart_certified":false,"deployment":false});
    let mut file = File::create(output.join("run_metadata.json"))?;
    serde_json::to_writer_pretty(&mut file, &metadata)?;
    file.write_all(b"\n")?;
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    Ok(())
}

fn main() -> Result<()> {
    let (mut handoff, mut reference, mut output) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!("imoexf_shadow_replay --handoff DIR --reference-state DIR --output NEW_DIR (offline only)");
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
