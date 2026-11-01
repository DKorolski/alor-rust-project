//! Private request/response worker for reaction_shadow_service.py. No Runtime,
//! Strategy, command publisher or gateway. EOF is a supervisor failure, not STOP.
use anyhow::{bail, ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, BufRead, Read, Write},
    path::Path,
    time::Duration,
};
use strategy_runtime::shadow_session_context::collector::runner::{
    service::{LaunchSpec, ServiceGuard},
    ReactionShadowRunner,
};

fn respond(value: serde_json::Value) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    if let Err(error) = run().await {
        // Do not leak Redis credentials or raw market payloads through errors.
        let detail = format!("{error:#}");
        let digest = format!("{:x}", Sha256::digest(&detail));
        let reason = failure_class(&detail);
        let _ = respond(serde_json::json!({"event":"failed", "error_sha256":digest,
            "reason":reason, "order_emission_allowed":false}));
        bail!("reaction shadow worker failed; retain generation for audit");
    }
    Ok(())
}

fn failure_class(detail: &str) -> &'static str {
    for (needle, class) in [
        ("bootstrap", "historical_bootstrap"),
        ("queue age exceeded", "queue_age"),
        ("source lateness exceeded", "source_lateness"),
        ("live M10/EOD bar missing", "missing_model_bar"),
        ("readiness deadline", "readiness_deadline"),
        ("complete paired EOD", "incomplete_session_resume"),
        ("paired anchors not ready", "anchors_not_ready"),
        ("maintenance threshold", "storage_maintenance"),
        ("resume cursor missing", "source_cursor_missing"),
        ("resume payload changed", "source_cursor_changed"),
        ("supervisor closed pipe", "supervisor_lost"),
    ] {
        if detail.contains(needle) {
            return class;
        }
    }
    "worker_error"
}

async fn run() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    ensure!(
        args.len() == 5,
        "use the reaction shadow supervisor: mode spec generation ready-by-epoch"
    );
    ensure!(
        matches!(args[1].as_str(), "create" | "resume" | "inspect-bootstrap"),
        "invalid worker mode"
    );
    let mut raw = Vec::new();
    fs::File::open(&args[2])?
        .take(64 * 1024 + 1)
        .read_to_end(&mut raw)?;
    ensure!(raw.len() <= 64 * 1024, "launch spec too large");
    let spec: LaunchSpec = toml::from_str(std::str::from_utf8(&raw)?)?;
    spec.validate()?;
    let ready_by = if args[1] != "inspect-bootstrap" {
        let ready_by: i64 = args[4].parse()?;
        ensure!(
            chrono::Utc::now().timestamp() < ready_by,
            "expired launch deadline"
        );
        ready_by
    } else {
        0
    };
    let client = redis::Client::open(
        env::var("REACTION_SHADOW_REDIS_URL")
            .context("dedicated read-only REACTION_SHADOW_REDIS_URL required")?,
    )?;
    let mut connection = tokio::time::timeout(
        Duration::from_secs(10),
        client.get_multiplexed_async_connection(),
    )
    .await
    .context("shadow Redis connect timed out")??;
    if args[1] == "inspect-bootstrap" {
        let preview =
            strategy_runtime::shadow_session_context::collector::runner::bootstrap::inspect(
                &mut connection,
                &spec.runner,
                &args[3],
                &args[4],
            )
            .await?;
        return respond(
            serde_json::json!({"event":"bootstrap_inspected", "preview":preview,
            "order_emission_allowed":false}),
        );
    }
    let mut runner = match args[1].as_str() {
        "create" => ReactionShadowRunner::create(Path::new(&args[3]), spec.runner)?,
        _ => ReactionShadowRunner::open(Path::new(&args[3]), spec.runner)?,
    };
    let mut guard = ServiceGuard::new(&runner, spec.policy, ready_by)?;
    respond(
        serde_json::json!({"event":"opened", "spec_sha256":format!("{:x}", Sha256::digest(raw)),
        "status":guard.status(&runner)?, "order_emission_allowed":false}),
    )?;
    let mut input = io::stdin().lock();
    loop {
        let mut command = String::new();
        // The parent never sends another request while this one is in flight.
        (&mut input).take(16).read_line(&mut command)?;
        match command.as_str() {
            "poll\n" => respond(serde_json::json!({"event":"polled",
                "status":guard.poll_once(&mut runner, &mut connection).await?,
                "order_emission_allowed":false}))?,
            "stop\n" => {
                respond(
                    serde_json::json!({"event":"stopped", "status":guard.status(&runner)?,
                    "order_emission_allowed":false}),
                )?;
                return Ok(());
            }
            _ => bail!("supervisor closed pipe or sent invalid request"),
        }
    }
}
