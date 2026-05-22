use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use strategy_runtime::strategies::moex_author41_42::{
    build_ri_author41_42_combo_shadow_journal, compare_combo_replay, load_model_bars,
    load_source_daily, replay_ri_author41_42_combo_cost2, ComboReplayComparison, ModelBar,
    ModelProfile, RegularSessionPolicy,
};

#[derive(Debug)]
struct Cli {
    bars_csv: PathBuf,
    source_daily_csv: PathBuf,
    model_id: String,
    tolerance: f64,
    out_json: Option<PathBuf>,
    out_journal_jsonl: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct Report {
    profile_id: String,
    combo_variant: String,
    model_id: String,
    bars_loaded: usize,
    comparison: ComboReplayComparison,
}

fn main() -> Result<()> {
    let cli = Cli::parse()?;
    let profile = ModelProfile::ri_shadow_10m();

    let bars = load_model_bars(
        File::open(&cli.bars_csv)
            .with_context(|| format!("open bars csv {}", cli.bars_csv.display()))?,
    )
    .context("load prepared model bars")?;
    let source_daily = load_source_daily(
        File::open(&cli.source_daily_csv)
            .with_context(|| format!("open source daily {}", cli.source_daily_csv.display()))?,
        &cli.model_id,
    )
    .context("load source daily")?;

    let replay = replay_ri_author41_42_combo_cost2(&bars, RegularSessionPolicy::moex_10m());
    let comparison = compare_combo_replay(&replay, &source_daily, cli.tolerance);
    if let Some(path) = &cli.out_journal_jsonl {
        write_shadow_journal(&bars, path)?;
    }
    let report = Report {
        profile_id: profile.profile_id.as_str().to_string(),
        combo_variant: profile.combo_variant.to_string(),
        model_id: cli.model_id,
        bars_loaded: bars.len(),
        comparison,
    };

    let json = serde_json::to_string_pretty(&report).context("serialize report")?;
    if let Some(path) = cli.out_json {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create output dir {}", parent.display()))?;
        }
        fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("write report {}", path.display()))?;
    }
    println!("{json}");
    Ok(())
}

impl Cli {
    fn parse() -> Result<Self> {
        let mut bars_csv = None;
        let mut source_daily_csv = None;
        let mut model_id = "ri_author41_42_primary_combo_cost2".to_string();
        let mut tolerance = 1e-9;
        let mut out_json = None;
        let mut out_journal_jsonl = None;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bars-csv" => bars_csv = Some(next_path(&mut args, "--bars-csv")?),
                "--source-daily-csv" => {
                    source_daily_csv = Some(next_path(&mut args, "--source-daily-csv")?)
                }
                "--model-id" => model_id = next_string(&mut args, "--model-id")?,
                "--tolerance" => {
                    tolerance = next_string(&mut args, "--tolerance")?
                        .parse()
                        .context("parse --tolerance")?;
                }
                "--out-json" => out_json = Some(next_path(&mut args, "--out-json")?),
                "--out-journal-jsonl" => {
                    out_journal_jsonl = Some(next_path(&mut args, "--out-journal-jsonl")?)
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => bail!("unsupported argument {other:?}; use --help"),
            }
        }

        Ok(Self {
            bars_csv: bars_csv.context("missing --bars-csv")?,
            source_daily_csv: source_daily_csv.context("missing --source-daily-csv")?,
            model_id,
            tolerance,
            out_json,
            out_journal_jsonl,
        })
    }
}

fn write_shadow_journal(bars: &[ModelBar], path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create journal output dir {}", parent.display()))?;
    }
    let mut writer = BufWriter::new(
        File::create(path).with_context(|| format!("create journal {}", path.display()))?,
    );
    for record in build_ri_author41_42_combo_shadow_journal(bars, RegularSessionPolicy::moex_10m())
    {
        serde_json::to_writer(&mut writer, &record).context("serialize shadow journal record")?;
        writer
            .write_all(b"\n")
            .context("write shadow journal newline")?;
    }
    writer.flush().context("flush shadow journal")?;
    Ok(())
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(next_string(args, flag)?))
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("missing value for {flag}"))
}

fn print_usage() {
    println!(
        "Usage: moex_author41_42_combo_replay \\
  --bars-csv PATH \\
  --source-daily-csv PATH \\
  [--model-id ri_author41_42_primary_combo_cost2] \\
  [--tolerance 1e-9] \\
  [--out-json PATH] \\
  [--out-journal-jsonl PATH]"
    );
}
