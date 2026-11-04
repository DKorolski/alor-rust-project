use super::*;
use std::process::{Child, Command, Stdio};

struct OwnedRedis(Child);
impl Drop for OwnedRedis {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ts(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}

fn advance(r: &mut Runner, s: &str) {
    let utc = ts(s);
    let previous = r.test_clock.unwrap();
    assert!(utc >= previous.utc);
    r.test_clock = Some(Clock {
        utc,
        monotonic_ms: previous.monotonic_ms + ((utc - previous.utc) * 1000) as u64,
    });
    r.last_rx = utc;
}

fn quote(label: i64) -> serde_json::Value {
    json!({"data":{"symbol":"USDRUBF","time":label,"open":100.,"high":102.,"low":98.,"close":101.,"volume":1.}})
}

#[tokio::test]
#[ignore = "explicit owned Redis executable required; never connects to an existing server"]
async fn sparse_context_pipeline_preserves_receipts_prefix_and_restore() {
    tokio::time::timeout(Duration::from_secs(30), pipeline(false))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "explicit owned Redis executable required; never connects to an existing server"]
async fn ordinary_delayed_pipeline_retains_evidence_not_model_freshness() {
    tokio::time::timeout(Duration::from_secs(30), pipeline(true))
        .await
        .unwrap();
}

async fn pipeline(ordinary: bool) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let socket = root.join("redis.sock");
    let server = std::env::var("REDIS_SERVER_BIN").expect("set owned Redis executable explicitly");
    let mut owned = OwnedRedis(
        Command::new(server)
            .args([
                "--port",
                "0",
                "--save",
                "",
                "--appendonly",
                "no",
                "--unixsocket",
            ])
            .arg(&socket)
            .arg("--dir")
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = redis::Client::open(redis::ConnectionInfo {
        addr: redis::ConnectionAddr::Unix(socket.clone()),
        redis: Default::default(),
    })
    .unwrap();
    let mut conn = loop {
        assert!(owned.0.try_wait().unwrap().is_none(), "owned Redis exited");
        if socket.exists()
            && let Ok(conn) = client.get_multiplexed_async_connection().await
        {
            break conn;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(
        info.lines()
            .any(|line| line.trim() == format!("process_id:{}", owned.0.id())),
        "refusing unrelated Redis"
    );
    let mut cfg:SourceConfig=serde_json::from_value(json!({"symbol":"USDRUBF","stream":"shadow.source.closed10m.USDRUBF.owned_context",
        "bootstrap_start":"2026-11-05","first_live_session":"2026-11-09","excluded_dates":[],"context_finalization":true,"ordinary_observation_max_lateness_seconds":300,
        "gateway_config":"/fixture/gateway.toml","gateway_config_sha256":"a".repeat(64),
        "terminal":{"enabled":true,"first_live_session":"2026-11-07","grace_seconds":10,"max_lateness_seconds":90,
        "max_heartbeat_age_seconds":45,"max_clock_step_seconds":5,
        "symbols":{"USDRUBF":{"weekday_end":"23:50:00","weekend_end":"19:00:00","overrides":{}}}}})).unwrap();
    if ordinary {
        cfg.bootstrap_start = "2026-11-04".parse().unwrap();
        cfg.terminal.first_live_session = "2026-11-06".parse().unwrap();
        cfg.first_live_session = "2026-11-06".parse().unwrap();
    }
    let now = ts(if ordinary {
        "2026-11-06T06:00:00+03:00"
    } else {
        "2026-11-07T09:51:00+03:00"
    });
    let state = root.join("source");
    let mut r = Runner {
        store: Store::open(&state, &cfg, "create", now).unwrap(),
        core: cfg.finalizer().unwrap(),
        cfg: cfg.clone(),
        conn,
        started: Instant::now(),
        generation: 1,
        connected: true,
        acked: false,
        history_complete: false,
        readiness_deadline: ReadinessDeadline::initial(),
        subscribed_at: Some(now),
        last_rx: now,
        sequence: 0,
        history: BTreeMap::new(),
        current: None,
        test_clock: Some(Clock {
            utc: now,
            monotonic_ms: 0,
        }),
    };
    // Real closed historical frames before ACK are staged, never published early.
    let days = if ordinary {
        ["2026-11-04T07:00:00+03:00", "2026-11-05T07:00:00+03:00"]
    } else {
        ["2026-11-05T07:00:00+03:00", "2026-11-06T07:00:00+03:00"]
    };
    for day in days {
        for slot in 0..101 {
            r.raw(quote(ts(day) + slot * 600), 1).await.unwrap();
        }
    }
    if !ordinary {
        r.raw(quote(ts("2026-11-07T09:50:00+03:00")), 1)
            .await
            .unwrap();
    }
    r.drain_history().await.unwrap();
    assert!(r.store.records.is_empty());
    r.event(WsEvent::SubscriptionAck {
        subscription_type: "bars".into(),
        generation: 1,
    })
    .await
    .unwrap();
    r.drain_history().await.unwrap();
    assert_eq!(r.store.records.len(), 202);
    assert!(r.history_complete);
    if ordinary {
        advance(&mut r, "2026-11-06T19:50:03+03:00");
        r.feed().unwrap();
        r.raw(quote(ts("2026-11-06T19:50:00+03:00")), 1)
            .await
            .unwrap();
        advance(&mut r, "2026-11-06T20:02:05+03:00");
        r.feed().unwrap();
        r.raw(quote(ts("2026-11-06T20:00:00+03:00")), 1)
            .await
            .unwrap();
        let delayed = &r.store.records.last_key_value().unwrap().1;
        let release = delayed.release.as_ref().unwrap();
        assert_eq!(
            release.proof.reason,
            crate::terminal_bar::ReleaseReason::NextTimestampDelayed
        );
        assert_eq!(
            release.proof.observation_cutoff_utc - release.proof.closed_at_utc,
            125
        );
        assert_eq!(
            release.proof.first_received_at_utc,
            ts("2026-11-06T19:50:03+03:00")
        );
        assert_eq!(delayed.envelope["ts_utc"], r.clock().utc);
        assert_eq!(delayed.bar().unwrap().origin, DataOrigin::Live);
        advance(&mut r, "2026-11-06T20:10:01+03:00");
        r.feed().unwrap();
        r.raw(quote(ts("2026-11-06T20:10:00+03:00")), 1)
            .await
            .unwrap();
        assert_eq!(r.store.records.len(), 204);
        assert_eq!(
            r.store
                .records
                .last_key_value()
                .unwrap()
                .1
                .release
                .as_ref()
                .unwrap()
                .proof
                .reason,
            crate::terminal_bar::ReleaseReason::NextTimestamp
        );
        r.health("ready", true).unwrap();
        let health: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state.join("HEALTH.json")).unwrap()).unwrap();
        assert_eq!(health["ordinary_delayed_rows"], 1);
        assert_eq!(health["last_ordinary_delay_seconds"], 125);
        assert_eq!(health["model_freshness_policy"], "consumer_owned_unchanged");
        let rows = r
            .store
            .records
            .values()
            .rev()
            .filter_map(|r| r.release.clone())
            .take(2)
            .collect::<Vec<_>>();
        let clock = r.clock();
        r.store.verify_prefix(&cfg, &mut r.conn).await.unwrap();
        r.store.stopped(clock.utc).unwrap();
        drop(r);
        let store = Store::open(&state, &cfg, "resume", clock.utc).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        store.verify_prefix(&cfg, &mut conn).await.unwrap();
        let mut core = cfg.finalizer().unwrap();
        core.restore_committed(&rows.into_iter().rev().collect::<Vec<_>>(), clock)
            .unwrap();
        assert_eq!(
            core.last_committed_label("USDRUBF"),
            Some(ts("2026-11-06T20:00:00+03:00"))
        );
        store.stopped(clock.utc).unwrap();
        return;
    }
    advance(&mut r, "2026-11-07T10:00:10+03:00");
    r.feed().unwrap();
    let releases = r.core.tick(r.clock(), true).unwrap();
    assert_eq!(releases.len(), 1);
    r.release(releases[0].clone()).await.unwrap();
    let row = r.store.records.last_key_value().unwrap().1;
    assert_eq!(row.bar().unwrap().origin, DataOrigin::History);
    assert_eq!(
        row.release.as_ref().unwrap().proof.reason,
        crate::terminal_bar::ReleaseReason::ContextIntervalWs
    );
    assert_eq!(row.envelope["ts_utc"], r.clock().utc);
    assert_eq!(
        row.release.as_ref().unwrap().proof.last_received_at_utc,
        now
    );
    // Two missing intervals do not stall the healthy source or create empty candles.
    advance(&mut r, "2026-11-07T10:20:00+03:00");
    r.feed().unwrap();
    r.raw(quote(r.clock().utc), 1).await.unwrap();
    advance(&mut r, "2026-11-07T10:30:10+03:00");
    r.feed().unwrap();
    for release in r.core.tick(r.clock(), true).unwrap() {
        r.release(release).await.unwrap();
    }
    assert_eq!(r.store.records.len(), 204);
    for absent in ["2026-11-07T10:00:00+03:00", "2026-11-07T10:10:00+03:00"] {
        assert!(!r.store.records.contains_key(&ts(absent)));
    }
    r.store.verify_prefix(&cfg, &mut r.conn).await.unwrap();
    // Reconnect cannot back-insert unknown history behind the durable prefix.
    r.event(WsEvent::Conn {
        event: ConnEvent::Disconnected,
        generation: 2,
    })
    .await
    .unwrap();
    assert!(r.core.pending_releases().is_empty());
    r.subscribed_at = Some(r.clock().utc);
    assert!(
        r.raw(quote(ts("2026-11-07T10:00:00+03:00")), 2)
            .await
            .unwrap_err()
            .to_string()
            .contains("immutable source prefix")
    );
    let restored = r
        .store
        .records
        .values()
        .rev()
        .filter_map(|row| row.release.clone())
        .take(2)
        .collect::<Vec<_>>();
    let clock = r.clock();
    r.store.stopped(clock.utc).unwrap();
    drop(r);
    let store = Store::open(&state, &cfg, "resume", clock.utc).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    store.verify_prefix(&cfg, &mut conn).await.unwrap();
    let mut core = cfg.finalizer().unwrap();
    core.restore_committed(&restored.into_iter().rev().collect::<Vec<_>>(), clock)
        .unwrap();
    assert_eq!(
        core.last_committed_label("USDRUBF"),
        Some(ts("2026-11-07T10:20:00+03:00"))
    );
    store.stopped(clock.utc).unwrap();
}
