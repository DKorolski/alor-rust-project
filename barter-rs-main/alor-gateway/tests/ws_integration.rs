mod support;

use std::time::Duration;

use alor_gateway::config::AlorGatewayConfig;
use alor_gateway::models::BarEvent;
use alor_gateway::supervisor::{GatewayHandle, Supervisor};
use alor_scalping::strategy::{Action, StrategyBar, StrategyContext, StrategyCore};
use serde_json::Value;
use tempfile::NamedTempFile;
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};

use support::fixtures::{ack_for_guid, bar_message};
use support::mock_cws::MockCwsServer;
use support::mock_ws::{MockWsEvent, MockWsServer, parse_guid, parse_opcode};

#[derive(Default)]
struct TestStrategy;

impl StrategyCore for TestStrategy {
    fn on_bar(&mut self, _bar: StrategyBar, _ctx: StrategyContext) -> Vec<Action> {
        Vec::new()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_resubscribe_no_duplicates() {
    let result = timeout(Duration::from_secs(15), async {
        let (ws_server, mut ws_events) = MockWsServer::start().await.unwrap();
        let cws_server = MockCwsServer::start().await.unwrap();

        let report = NamedTempFile::new().unwrap();
        let bar_dump = NamedTempFile::new().unwrap();
        let cfg = test_config(
            ws_server.ws_url(),
            cws_server.ws_url(),
            report.path().to_string_lossy().to_string(),
            bar_dump.path().to_string_lossy().to_string(),
        );
        let supervisor = Supervisor::new_with_token(cfg, "test-token".to_string());
        let handle = supervisor.start_with_handle(TestStrategy::default());
        let mut emitted = handle.subscribe_emitted_bars();

        let first_conn = wait_for_connection(&mut ws_events).await;
        let first_guid = ack_subscriptions(&ws_server, &mut ws_events, first_conn).await;
        sleep(Duration::from_millis(50)).await;

        for ts in [60, 120, 180] {
            ws_server
                .send_text(first_conn, bar_message("TEST", ts, &first_guid))
                .await
                .unwrap();
        }

        ws_server.set_respond_to_ping(first_conn, false).await.unwrap();
        handle.request_reconnect().await.unwrap();
        wait_for_ws_reconnect(&handle, 1).await;

        let second_conn = wait_for_connection(&mut ws_events).await;
        let second_guid = ack_subscriptions(&ws_server, &mut ws_events, second_conn).await;
        sleep(Duration::from_millis(50)).await;

        for ts in [180, 240, 300] {
            ws_server
                .send_text(second_conn, bar_message("TEST", ts, &second_guid))
                .await
                .unwrap();
        }

        let times = collect_close_times(&mut emitted, 3, Duration::from_secs(2)).await;
        assert!(times.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(times.contains(&240));

        sleep(Duration::from_millis(200)).await;
        handle.stop().await.unwrap();
    })
    .await;
    assert!(result.is_ok(), "test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_guid_after_reconnect_ignored() {
    let result = timeout(Duration::from_secs(15), async {
        let (ws_server, mut ws_events) = MockWsServer::start().await.unwrap();
        let cws_server = MockCwsServer::start().await.unwrap();

        let report = NamedTempFile::new().unwrap();
        let bar_dump = NamedTempFile::new().unwrap();
        let cfg = test_config(
            ws_server.ws_url(),
            cws_server.ws_url(),
            report.path().to_string_lossy().to_string(),
            bar_dump.path().to_string_lossy().to_string(),
        );
        let supervisor = Supervisor::new_with_token(cfg, "test-token".to_string());
        let handle = supervisor.start_with_handle(TestStrategy::default());
        let mut emitted = handle.subscribe_emitted_bars();

        let first_conn = wait_for_connection(&mut ws_events).await;
        let first_guid = ack_subscriptions(&ws_server, &mut ws_events, first_conn).await;

        ws_server
            .send_text(first_conn, bar_message("TEST", 60, &first_guid))
            .await
            .unwrap();

        let _ = collect_close_times(&mut emitted, 1, Duration::from_secs(2)).await;

        ws_server.set_respond_to_ping(first_conn, false).await.unwrap();
        handle.request_reconnect().await.unwrap();
        wait_for_ws_reconnect(&handle, 1).await;

        let second_conn = wait_for_connection(&mut ws_events).await;
        let second_guid = ack_subscriptions(&ws_server, &mut ws_events, second_conn).await;

        ws_server
            .send_text(second_conn, bar_message("TEST", 120, &first_guid))
            .await
            .unwrap();
        ws_server
            .send_text(second_conn, bar_message("TEST", 180, &second_guid))
            .await
            .unwrap();

        let times = collect_close_times(&mut emitted, 1, Duration::from_secs(2)).await;
        assert_eq!(times, vec![180]);

        handle.stop().await.unwrap();

        let report_json: Value =
            serde_json::from_reader(std::fs::File::open(report.path()).unwrap()).unwrap();
        let ignored_entries = report_json
            .get("ignored")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ignored = ignored_entries.iter().find(|entry| {
            entry.get("reason").and_then(Value::as_str) == Some("UnknownGuid")
        });
        assert!(ignored.is_some());
    })
    .await;
    assert!(result.is_ok(), "test timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delayed_ack_retry_ignores_old_guid() {
    let result = timeout(Duration::from_secs(15), async {
        let (ws_server, mut ws_events) = MockWsServer::start().await.unwrap();
        let cws_server = MockCwsServer::start().await.unwrap();

        let report = NamedTempFile::new().unwrap();
        let bar_dump = NamedTempFile::new().unwrap();
        let cfg = test_config(
            ws_server.ws_url(),
            cws_server.ws_url(),
            report.path().to_string_lossy().to_string(),
            bar_dump.path().to_string_lossy().to_string(),
        );
        let supervisor = Supervisor::new_with_token(cfg, "test-token".to_string());
        let handle = supervisor.start_with_handle(TestStrategy::default());
        let mut emitted = handle.subscribe_emitted_bars();

        let conn = wait_for_connection(&mut ws_events).await;
        let (first_guid, second_guid) =
            delayed_bars_ack(&ws_server, &mut ws_events, conn).await;

        for ts in [60, 120] {
            ws_server
                .send_text(conn, bar_message("TEST", ts, &second_guid))
                .await
                .unwrap();
        }

        let times = collect_close_times(&mut emitted, 2, Duration::from_secs(2)).await;
        assert_eq!(times, vec![60, 120]);

        handle.stop().await.unwrap();

        let report_json: Value =
            serde_json::from_reader(std::fs::File::open(report.path()).unwrap()).unwrap();
        let ignored_entries = report_json
            .get("ignored")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ignored = ignored_entries
            .iter()
            .find(|entry| entry.get("reason").and_then(Value::as_str) == Some("InactiveGuid"));
        assert!(ignored.is_some());
        assert_ne!(first_guid, second_guid);
    })
    .await;
    assert!(result.is_ok(), "test timed out");
}

async fn wait_for_connection(events: &mut tokio::sync::mpsc::Receiver<MockWsEvent>) -> usize {
    loop {
        if let Some(MockWsEvent::Connected { id }) = events.recv().await {
            return id;
        }
    }
}

async fn ack_subscriptions(
    server: &MockWsServer,
    events: &mut tokio::sync::mpsc::Receiver<MockWsEvent>,
    conn_id: usize,
) -> String {
    let mut bars_guid = None;
    let mut positions_ok = false;
    let mut orders_ok = false;
    while bars_guid.is_none() || !positions_ok || !orders_ok {
        let Some(event) = events.recv().await else {
            continue;
        };
        let MockWsEvent::Text { id, text } = event else {
            continue;
        };
        if id != conn_id {
            continue;
        }
        let opcode = parse_opcode(&text).unwrap_or_default();
        let guid = parse_guid(&text).unwrap_or_default();
        match opcode.as_str() {
            "BarsGetAndSubscribe" => {
                server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                bars_guid = Some(guid);
            }
            "PositionsGetAndSubscribeV2" => {
                server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                positions_ok = true;
            }
            "OrdersGetAndSubscribeV2" => {
                server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                orders_ok = true;
            }
            _ => {}
        }
    }
    bars_guid.expect("bars guid missing")
}

async fn delayed_bars_ack(
    server: &MockWsServer,
    events: &mut tokio::sync::mpsc::Receiver<MockWsEvent>,
    conn_id: usize,
) -> (String, String) {
    let mut first_guid = None;
    let mut second_guid = None;
    let mut positions_ok = false;
    let mut orders_ok = false;

    while second_guid.is_none() || !positions_ok || !orders_ok {
        let Some(event) = events.recv().await else {
            continue;
        };
        let MockWsEvent::Text { id, text } = event else {
            continue;
        };
        if id != conn_id {
            continue;
        }
        let opcode = parse_opcode(&text).unwrap_or_default();
        let guid = parse_guid(&text).unwrap_or_default();
        match opcode.as_str() {
            "BarsGetAndSubscribe" => {
                match first_guid.as_ref() {
                    None => {
                        first_guid = Some(guid);
                        sleep(Duration::from_millis(350)).await;
                    }
                    Some(_) => {
                        second_guid = Some(guid.clone());
                        let first = first_guid.clone().unwrap();
                        server.send_text(conn_id, ack_for_guid(&first)).await.unwrap();
                        server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                    }
                }
            }
            "PositionsGetAndSubscribeV2" => {
                server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                positions_ok = true;
            }
            "OrdersGetAndSubscribeV2" => {
                server.send_text(conn_id, ack_for_guid(&guid)).await.unwrap();
                orders_ok = true;
            }
            _ => {}
        }
    }

    (first_guid.unwrap(), second_guid.unwrap())
}

async fn collect_close_times(
    rx: &mut broadcast::Receiver<BarEvent>,
    count: usize,
    per_item_timeout: Duration,
) -> Vec<i64> {
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let bar = timeout(per_item_timeout, rx.recv())
            .await
            .expect("timed out waiting for bar")
            .expect("broadcast recv failed");
        times.push(bar.close_time_utc);
    }
    times
}

async fn wait_for_ws_reconnect(handle: &GatewayHandle, reconnects: u64) {
    let result = timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = handle.state_snapshot();
            if snapshot.ws_reconnects_total >= reconnects && snapshot.ws_connected {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(result.is_ok(), "timed out waiting for ws reconnect");
}

fn test_config(
    ws_url: String,
    cws_url: String,
    report_path: String,
    bar_dump_path: String,
) -> AlorGatewayConfig {
    AlorGatewayConfig {
        portfolio: "TEST".to_string(),
        exchange: "MOEX".to_string(),
        instrument_group: "RFUD".to_string(),
        symbols: vec!["TEST".to_string()],
        tf_sec: 60,
        from_ts: 0,
        ws_url,
        cws_url,
        oauth_url: "http://localhost/unused".to_string(),
        refresh_token: "unused".to_string(),
        skip_history_bars: false,
        skip_history_positions: true,
        skip_history_orders: true,
        split_adjust: false,
        format: "Simple".to_string(),
        frequency_ms: 250,
        backoff_initial_ms: 10,
        backoff_max_ms: 50,
        backoff_multiplier: 2,
        max_silence_bars_sec: 600,
        history_sessions: 1,
        history_days_back: 1,
        session_rollover_hour_utc: 0,
        health_listen_addr: "127.0.0.1:0".to_string(),
        price_step: 0.0,
        volume_step: 0.0,
        log_positions_filter: vec![],
        log_cash_positions: false,
        cash_symbols: vec![],
        log_existing_snapshot_orders: false,
        ws_idle_timeout_sec: 1,
        ws_ping_interval_sec: 1,
        ws_ping_timeout_sec: 2,
        subscribe_ack_timeout_ms: 200,
        subscribe_ack_timeout_positions_ms: 200,
        subscribe_ack_retries: 2,
        warm_reconnect_max_gap_sec: 10,
        gap_backfill_padding_bars: 1,
        cold_start_history_days_back: 1,
        bar_silence_resync_min_sec: 60,
        data_report_path: Some(report_path),
        bar_dump_path: Some(bar_dump_path),
    }
}
