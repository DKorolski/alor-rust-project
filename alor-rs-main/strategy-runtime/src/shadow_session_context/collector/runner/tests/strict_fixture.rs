//! Test-only native process: real runner/service, declared fixture clock, owned Redis.
use super::*;
use crate::shadow_session_context::collector::runner::service::{LaunchSpec, ServiceGuard};
use std::io::{BufRead, Write};

fn owned_clock() -> i64 {
    let root = PathBuf::from(std::env::var("SHADOW_OWNED_ROOT").unwrap());
    assert!(root.join("OWNED_FIXTURE").is_file());
    fs::read_to_string(root.join("clock"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}
fn reply(value: Value) {
    println!("OWNED_REPLY:{value}");
    std::io::stdout().flush().unwrap();
}

#[test]
#[ignore = "owned source manifest inspection only; explicit fixture root required"]
fn bootstrap_manifest() {
    assert_eq!(
        std::env::var("SHADOW_OWNED_FIXTURE").unwrap(),
        "strict_composed_v1"
    );
    let root = PathBuf::from(std::env::var("SHADOW_OWNED_ROOT").unwrap());
    assert!(root.join("OWNED_FIXTURE").is_file());
    let rows: Vec<Receipt> = fs::read_to_string(root.join("source/committed.jsonl"))
        .unwrap()
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let row: Value = serde_json::from_str(line).unwrap();
            Receipt {
                sequence: index as u64 + 1,
                source_id: row["record"]["id"].as_str().unwrap().into(),
                bar: serde_json::from_value(row["record"]["envelope"]["payload"].clone()).unwrap(),
                received_at_utc: Some(owned_clock()),
                durable_at_utc: Some(owned_clock()),
            }
        })
        .collect();
    let pin = bootstrap::HistoricalBootstrap {
        first_source_id: rows.first().unwrap().source_id.clone(),
        last_source_id: rows.last().unwrap().source_id.clone(),
        rows: rows.len() as u64,
        source_sha256: bootstrap::HistoricalBootstrap::digest(&rows).unwrap(),
        first_session: local(rows.first().unwrap().bar.close_time_utc)
            .unwrap()
            .date(),
        last_session: local(rows.last().unwrap().bar.close_time_utc)
            .unwrap()
            .date(),
    };
    pin.verify(&rows, local(owned_clock()).unwrap().date())
        .unwrap();
    fs::write(
        root.join("owned-bootstrap.json"),
        serde_json::to_vec(&pin).unwrap(),
    )
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "owned composed fixture only; never a production worker/clock override"]
async fn model_process() {
    assert_eq!(
        std::env::var("SHADOW_OWNED_FIXTURE").unwrap(),
        "strict_composed_v1"
    );
    let root = PathBuf::from(std::env::var("SHADOW_OWNED_ROOT").unwrap());
    let raw = fs::read(root.join("worker.toml")).unwrap();
    let launch: LaunchSpec = toml::from_str(std::str::from_utf8(&raw).unwrap()).unwrap();
    assert!(launch.runner.late_start.is_none());
    let client = redis::Client::open(std::env::var("REACTION_SHADOW_REDIS_URL").unwrap()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let mut runner =
        ReactionShadowRunner::create_with_clock(&root.join("model"), launch.runner, owned_clock)
            .unwrap();
    let ready = root.join("ready_by");
    let ready: i64 = fs::read_to_string(ready).unwrap().trim().parse().unwrap();
    let mut guard = ServiceGuard::new(&runner, launch.policy, ready).unwrap();
    reply(
        serde_json::json!({"event":"opened","spec_sha256":format!("{:x}",Sha256::digest(&raw)),
        "capabilities":["split_collect_advance_v1"],"status":guard.status(&runner).unwrap(),"order_emission_allowed":false}),
    );
    for command in std::io::stdin().lock().lines() {
        let command = command.unwrap();
        let result = match command.as_str() {
            "collect" => guard
                .collect_once(&mut runner, &mut conn)
                .await
                .map(|s| ("collected", s)),
            "advance" => guard
                .advance_collected(&mut runner)
                .map(|s| ("advanced", s)),
            "stop" => guard.status(&runner).map(|s| ("stopped", s)),
            _ => panic!("invalid owned protocol"),
        };
        match result {
            Ok((event, status)) => {
                reply(
                    serde_json::json!({"event":event,"status":status,"order_emission_allowed":false}),
                );
                if command == "stop" {
                    return;
                }
            }
            Err(error) => {
                eprintln!("owned strict worker: {error:#}");
                fs::write(root.join("owned-failure.txt"), format!("{error:#}")).unwrap();
                reply(serde_json::json!({"event":"failed","reason":"worker_error",
                    "error_sha256":format!("{:x}",Sha256::digest(format!("{error:#}"))),"order_emission_allowed":false}));
                return;
            }
        }
    }
    panic!("owned model EOF without STOP");
}
