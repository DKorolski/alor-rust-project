//! Explicit owned-process fixture. Compiled only into the test executable.
use super::*;
use std::io::{BufRead, Write};

fn reply(value: serde_json::Value) {
    println!("OWNED_REPLY:{value}");
    std::io::stdout().flush().unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "owned composed fixture only; needs explicit temporary Redis PID/config/clock"]
async fn source_process() {
    assert_eq!(
        std::env::var("SHADOW_OWNED_FIXTURE").unwrap(),
        "strict_composed_v1"
    );
    let root = Path::new(&std::env::var("SHADOW_OWNED_ROOT").unwrap()).to_path_buf();
    assert!(root.is_absolute() && root.join("OWNED_FIXTURE").is_file());
    let cfg: SourceConfig =
        serde_json::from_slice(&std::fs::read(root.join("source.json")).unwrap()).unwrap();
    let client = redis::Client::open(std::env::var("SHADOW_OWNED_REDIS_URL").unwrap()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(info.lines().any(|line| line.trim()
        == format!(
            "process_id:{}",
            std::env::var("SHADOW_OWNED_REDIS_PID").unwrap()
        )));
    let initial: i64 = std::fs::read_to_string(root.join("clock"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut runner = Runner {
        store: Store::open(&root.join("source"), &cfg, "create", initial).unwrap(),
        core: cfg.finalizer().unwrap(),
        cfg,
        conn,
        started: Instant::now(),
        generation: 1,
        connected: true,
        acked: false,
        history_complete: false,
        readiness_deadline: ReadinessDeadline::initial(),
        subscribed_at: Some(initial),
        last_rx: initial,
        sequence: 0,
        history: BTreeMap::new(),
        current: None,
        test_clock: Some(Clock {
            utc: initial,
            monotonic_ms: 0,
        }),
    };
    let (_tx, mut events) = tokio::sync::mpsc::channel(16);
    reply(json!({"event":"source_opened"}));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        assert!(line.len() < 65536);
        let command: serde_json::Value = serde_json::from_str(&line).unwrap();
        let utc = command["now"].as_i64().unwrap();
        let old = runner.clock();
        assert!(utc >= old.utc);
        runner.test_clock = Some(Clock {
            utc,
            monotonic_ms: old.monotonic_ms + ((utc - old.utc) * 1000) as u64,
        });
        let op = command["op"].as_str().unwrap();
        if op == "stop" {
            reply(json!({"event":"source_stopped"}));
            return;
        }
        if op == "history" {
            let mut day = runner.cfg.bootstrap_start;
            while day < runner.cfg.first_live_session {
                if !runner.cfg.context_day(day) {
                    let start = day.and_hms_opt(7, 0, 0).unwrap().and_utc().timestamp() - 10800;
                    for slot in 0..101 {
                        let value = json!({"data":{"symbol":runner.cfg.symbol,"time":start+slot*600,
                            "open":100.,"high":102.,"low":98.,"close":100.,"volume":1.}});
                        runner
                            .event(WsEvent::Raw {
                                value,
                                generation: runner.generation,
                                received_at_utc: Some(utc),
                            })
                            .await
                            .unwrap();
                    }
                }
                day = day.succ_opt().unwrap();
            }
            runner
                .event(WsEvent::SubscriptionAck {
                    subscription_type: "bars".into(),
                    generation: runner.generation,
                })
                .await
                .unwrap();
        } else if op == "disconnect" {
            runner
                .event(WsEvent::Conn {
                    event: ConnEvent::Disconnected,
                    generation: runner.generation,
                })
                .await
                .unwrap();
        } else if op == "resume" {
            let generation = runner.generation + 1;
            runner
                .event(WsEvent::Conn {
                    event: ConnEvent::Connected,
                    generation,
                })
                .await
                .unwrap();
            runner
                .event(WsEvent::Subscribed {
                    wallclock_ts: utc,
                    history_origin: DataOrigin::History,
                    bars_from_ts: utc - 1200,
                    skip_history: false,
                    generation,
                })
                .await
                .unwrap();
            runner
                .event(WsEvent::SubscriptionAck {
                    subscription_type: "bars".into(),
                    generation,
                })
                .await
                .unwrap();
        } else if op == "heartbeat" {
            runner
                .event(WsEvent::WsRx {
                    ts: utc,
                    generation: runner.generation,
                })
                .await
                .unwrap();
        }
        if let Some(label) = command["label"].as_i64() {
            let price = command["price"].as_f64().unwrap_or(100.);
            let value = json!({"data":{"symbol":runner.cfg.symbol,"time":label,
                "open":price,"high":price,"low":price,"close":price,"volume":1.}});
            runner
                .event(WsEvent::Raw {
                    value,
                    generation: runner.generation,
                    received_at_utc: Some(utc),
                })
                .await
                .unwrap();
        }
        runner.terminal_tick(&mut events).await.unwrap();
        runner
            .health(
                if runner.history_complete && runner.connected && runner.acked {
                    "ready"
                } else {
                    "reconnecting"
                },
                true,
            )
            .unwrap();
        reply(
            json!({"event":"source_updated","published_rows":runner.store.records.len(),
            "last_label":runner.store.records.last_key_value().map(|(label,_)|label),"generation":runner.generation}),
        );
    }
    panic!("owned source EOF without STOP");
}
