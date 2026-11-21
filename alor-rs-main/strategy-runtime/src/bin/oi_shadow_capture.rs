use std::path::Path;

use anyhow::{ensure, Context, Result};
use strategy_runtime::shadow_oi::collector::io::{Clock, IssTransport, SystemClock, SystemDisk};
use strategy_runtime::shadow_oi::collector::{collect, diagnostic, Config, Status};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 && matches!(args[0].as_str(), "plan" | "clock" | "probe" | "collect"),
        "usage: oi_shadow_capture <plan|clock|probe|collect> /absolute/reviewed-config.json"
    );
    let config = Config::load(Path::new(&args[1]))?;
    let plan = config.plan()?;
    if args[0] == "plan" {
        println!("{}", serde_json::to_string(&plan)?);
        return Ok(());
    }
    if args[0] == "clock" {
        let reading = SystemClock::default().read()?;
        println!("{}", serde_json::to_string(&reading)?);
        ensure!(reading.healthy(), "native host clock failed OI bounds");
        return Ok(());
    }
    let mut transport = IssTransport::new().context("read-only ISS client setup")?;
    if args[0] == "probe" {
        let report = diagnostic::probe(&plan, &mut SystemClock::default(), &mut transport).await?;
        println!("{}", serde_json::to_string(&report)?);
        ensure!(report.passed, "non-PIT OI transport/schema probe failed");
        return Ok(());
    }
    let result = collect(
        &config.store_root,
        &plan,
        &config.policy,
        &mut SystemClock::default(),
        &mut transport,
        &mut SystemDisk,
    )
    .await?;
    println!("{}", serde_json::to_string(&result)?);
    ensure!(
        result.status == Status::AcceptedSnapshot,
        "OI capture is BO-disabled: {:?}",
        result.status
    );
    Ok(())
}
