//! Isolated MD-only entrypoint. No Supervisor, CWS or order command consumer.
use alor_gateway::md_source::{config::SourceConfig, digest_for_error, run};
use anyhow::{Context, Result, ensure};
use std::path::Path;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("alor_gateway::ws_hub=warn,alor_gateway::auth=warn")
        .with_ansi(false)
        .init();
    if let Err(error) = start().await {
        eprintln!(
            "{}",
            serde_json::json!({"event":"ws_source_failed","error_sha256":digest_for_error(&format!("{error:#}")),"order_emission_allowed":false})
        );
        std::process::exit(1);
    }
    Ok(())
}

async fn start() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4 || args.len() == 5,
        "usage: alor_shadow_md_ws create|resume CONFIG STATE [--fixture]"
    );
    ensure!(
        matches!(args[1].as_str(), "create" | "resume"),
        "explicit source mode required"
    );
    let fixture = args.len() == 5;
    ensure!(!fixture || args[4] == "--fixture", "unknown fixture option");
    let path = Path::new(&args[2]);
    ensure!(
        !path.is_symlink() && path.is_file() && path.metadata()?.len() <= 65536,
        "regular bounded source config required"
    );
    let config: SourceConfig =
        serde_json::from_slice(&std::fs::read(path)?).context("invalid source config")?;
    run(config, Path::new(&args[3]), &args[1], fixture).await
}
