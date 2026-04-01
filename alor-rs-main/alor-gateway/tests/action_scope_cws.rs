mod support;

use std::sync::Arc;
use std::time::Duration;

use alor_gateway::action_scope_cws::{ActionScopeCwsConfig, ActionScopeCwsManager};
use alor_gateway::auth::TokenProvider;
use alor_gateway::health::HealthState;
use parking_lot::RwLock;

use support::mock_cws::MockCwsServer;

fn test_manager(cws_url: String, health: Arc<RwLock<HealthState>>) -> ActionScopeCwsManager {
    ActionScopeCwsManager::new(
        cws_url,
        "RFUD".to_string(),
        TokenProvider::new_with_token(
            "http://localhost/unused",
            "refresh-token",
            "test-access-token",
        ),
        health,
        ActionScopeCwsConfig {
            open_timeout: Duration::from_secs(2),
            authorize_timeout: Duration::from_secs(2),
            followup_window: Duration::from_secs(2),
            max_session_lifetime: Duration::from_secs(5),
            close_timeout: Duration::from_secs(2),
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_action_windows_open_distinct_cws_sessions() {
    let cws_server = MockCwsServer::start().await.expect("start mock cws");
    let health = Arc::new(RwLock::new(HealthState {
        control_cws_mode: "action_scoped".to_string(),
        ..HealthState::default()
    }));
    let manager = test_manager(cws_server.ws_url(), health.clone());

    let create = manager
        .create_limit(
            "TEST",
            "MOEX",
            "USDRUBF",
            79.5,
            1.0,
            "buy",
            Some("create"),
            "req-create",
        )
        .await
        .expect("create action-scope request");
    let order_id = create
        .get("orderNumber")
        .and_then(|value| value.as_i64())
        .expect("mock create order id");

    manager
        .cancel("TEST", "MOEX", order_id, "req-cancel")
        .await
        .expect("cancel action-scope request");

    let requests = cws_server.snapshot_requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].opcode, "authorize");
    assert_eq!(requests[1].opcode, "create:limit");
    assert_eq!(requests[2].opcode, "authorize");
    assert_eq!(requests[3].opcode, "delete:limit");
    assert_ne!(requests[0].connection_id, requests[2].connection_id);
    assert_eq!(requests[0].connection_id, requests[1].connection_id);
    assert_eq!(requests[2].connection_id, requests[3].connection_id);

    let guard = health.read();
    assert_eq!(guard.action_scope_open_total, 2);
    assert_eq!(guard.action_scope_close_total, 2);
    assert_eq!(guard.action_scope_send_total, 4);
    assert_eq!(
        guard.last_action_scope_primary_opcode.as_deref(),
        Some("delete:limit")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_create_delete_chain_reuses_only_immediate_window() {
    let cws_server = MockCwsServer::start().await.expect("start mock cws");
    let health = Arc::new(RwLock::new(HealthState {
        control_cws_mode: "action_scoped".to_string(),
        ..HealthState::default()
    }));
    let manager = test_manager(cws_server.ws_url(), health.clone());

    let (_create, _delete) = manager
        .create_then_cancel(
            "TEST",
            "MOEX",
            "USDRUBF",
            79.5,
            1.0,
            "buy",
            Some("create-then-delete"),
            "req-bounded",
        )
        .await
        .expect("bounded create-delete");

    let requests = cws_server.snapshot_requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].opcode, "authorize");
    assert_eq!(requests[1].opcode, "create:limit");
    assert_eq!(requests[2].opcode, "delete:limit");
    assert_eq!(requests[0].connection_id, requests[1].connection_id);
    assert_eq!(requests[1].connection_id, requests[2].connection_id);

    let guard = health.read();
    assert_eq!(guard.action_scope_open_total, 1);
    assert_eq!(guard.action_scope_close_total, 1);
    assert_eq!(guard.action_scope_followup_total, 1);
    assert_eq!(
        guard.last_action_scope_followup_opcode.as_deref(),
        Some("delete:limit")
    );
    assert_eq!(
        guard.last_action_scope_primary_opcode.as_deref(),
        Some("create:limit")
    );
}
