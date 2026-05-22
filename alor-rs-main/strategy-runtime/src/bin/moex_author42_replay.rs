use std::env;
use std::fs::{self, File};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use strategy_runtime::strategies::moex_author41_42::{
    compare_author42_replay, load_model_bars, load_source_daily, load_source_trades,
    replay_author42, Author42ReplayComparison, ModelProfile, RegularSessionPolicy,
};

#[derive(Debug)]
struct Cli {
    bars_csv: PathBuf,
    source_trades_csv: PathBuf,
    source_daily_csv: PathBuf,
    model_id: String,
    tolerance: f64,
    out_json: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct Report {
    profile_id: String,
    author42_variant: String,
    model_id: String,
    bars_loaded: usize,
    comparison: Author42ReplayComparison,
}

fn main() -> Result<()> {
    let cli = Cli::parse()?;
    let profile = ModelProfile::ri_shadow_10m();

    let bars = load_model_bars(
        File::open(&cli.bars_csv)
            .with_context(|| format!("open bars csv {}", cli.bars_csv.display()))?,
    )
    .context("load prepared model bars")?;
    let source_trades = load_source_trades(
        File::open(&cli.source_trades_csv)
            .with_context(|| format!("open source trades {}", cli.source_trades_csv.display()))?,
        &cli.model_id,
    )
    .context("load source trades")?;
    let source_daily = load_source_daily(
        File::open(&cli.source_daily_csv)
            .with_context(|| format!("open source daily {}", cli.source_daily_csv.display()))?,
        &cli.model_id,
    )
    .context("load source daily")?;

    let replay = replay_author42(
        &bars,
        strategy_runtime::strategies::moex_author41_42::Author42Config::ri_grid_k042_both(),
        RegularSessionPolicy::moex_10m(),
    );
    let comparison = compare_author42_replay(&replay, &source_trades, &source_daily, cli.tolerance);
    let report = Report {
        profile_id: profile.profile_id.as_str().to_string(),
        author42_variant: profile.author42_variant.to_string(),
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
        let mut source_trades_csv = None;
        let mut source_daily_csv = None;
        let mut model_id = "ri_author42_bo_primary".to_string();
        let mut tolerance = 1e-9;
        let mut out_json = None;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bars-csv" => bars_csv = Some(next_path(&mut args, "--bars-csv")?),
                "--source-trades-csv" => {
                    source_trades_csv = Some(next_path(&mut args, "--source-trades-csv")?)
                }
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
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => bail!("unsupported argument {other:?}; use --help"),
            }
        }

        Ok(Self {
            bars_csv: bars_csv.context("missing --bars-csv")?,
            source_trades_csv: source_trades_csv.context("missing --source-trades-csv")?,
            source_daily_csv: source_daily_csv.context("missing --source-daily-csv")?,
            model_id,
            tolerance,
            out_json,
        })
    }
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
        "Usage: moex_author42_replay \\
  --bars-csv PATH \\
  --source-trades-csv PATH \\
  --source-daily-csv PATH \\
  [--model-id ri_author42_bo_primary] \\
  [--tolerance 1e-9] \\
  [--out-json PATH]"
    );
}
