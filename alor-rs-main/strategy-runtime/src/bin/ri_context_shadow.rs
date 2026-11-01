//! Explicit opt-in launcher; does not use Runtime, order streams or gateway APIs.
use std::{env, fs, io::Read, path::Path, time::Duration};

use anyhow::{bail, ensure, Context, Result};
use strategy_runtime::ri_context::runner::{RiShadowRunner, RunnerSpec};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    ensure!(args.len() == 4, "usage: ri_context_shadow <create|resume> <spec.toml> <new-or-existing-generation-dir>; Redis URL only via RI_SHADOW_REDIS_URL");
    let mut raw = String::new();
    fs::File::open(&args[2])?
        .take(64 * 1024 + 1)
        .read_to_string(&mut raw)?;
    ensure!(raw.len() <= 64 * 1024, "runner spec too large");
    let spec: RunnerSpec = toml::from_str(&raw)?;
    let client = redis::Client::open(
        env::var("RI_SHADOW_REDIS_URL").context("set dedicated read-only RI_SHADOW_REDIS_URL")?,
    )?;
    // Initial connection failure creates no generation. No reconnect loop can
    // silently turn a missing receipt interval into a clean comparison session.
    let mut connection = tokio::time::timeout(
        Duration::from_secs(10),
        client.get_multiplexed_async_connection(),
    )
    .await
    .context("shadow Redis connection timed out")??;
    let mut runner = match args[1].as_str() {
        "create" => RiShadowRunner::create(Path::new(&args[3]), spec)?,
        "resume" => RiShadowRunner::open(Path::new(&args[3]), spec)?,
        _ => bail!("expected create or resume"),
    };
    let mut last_progress = None;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let status = tokio::select! {
            signal = &mut shutdown => { signal?; break; }
            result = runner.poll_once(&mut connection) => result?,
        };
        let progress = (status.committed_transactions, status.source_caught_up);
        if last_progress != Some(progress) {
            println!("{}", serde_json::to_string(&status)?);
            last_progress = Some(progress);
        }
        tokio::select! {
            signal = &mut shutdown => { signal?; break; }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
    Ok(())
}
