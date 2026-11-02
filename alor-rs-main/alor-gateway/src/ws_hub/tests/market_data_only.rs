//! Owned in-memory WS transport. No Redis, real socket, broker or OAuth request.
use super::*;
use std::{
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};

type Reply = Result<Message, tokio_tungstenite::tungstenite::Error>;
struct AckSink {
    replies: futures::channel::mpsc::UnboundedSender<Reply>,
    opcodes: Arc<Mutex<Vec<String>>>,
    refuse_accounts: bool,
    ack_code: Option<i64>,
}
impl futures_util::Sink<Message> for AckSink {
    type Error = tokio_tungstenite::tungstenite::Error;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(self: Pin<&mut Self>, message: Message) -> Result<(), Self::Error> {
        let this = self.get_mut();
        let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        let opcode = value["opcode"].as_str().unwrap().to_string();
        this.opcodes.lock().unwrap().push(opcode.clone());
        if this.refuse_accounts && opcode != "BarsGetAndSubscribe" {
            return Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        }
        assert!(!value["token"].as_str().unwrap().is_empty());
        let mut ack = serde_json::json!({"guid":value["guid"],"message":"fixture"});
        if let Some(code) = this.ack_code {
            ack["httpCode"] = code.into();
        }
        this.replies
            .unbounded_send(Ok(Message::Text(ack.to_string().into())))
            .map_err(|_| tokio_tungstenite::tungstenite::Error::ConnectionClosed)
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

fn config() -> AlorGatewayConfig {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
portfolio = "fixture-no-account"
refresh_token = "fixture-not-a-credential"
symbols = ["IMOEXF", "USDRUBF"]
tf_sec = 600
"#,
    )
    .unwrap();
    AlorGatewayConfig::from_file_with_sources(tmp.path().to_str().unwrap().into())
        .unwrap()
        .config
}

async fn scoped_subscribe(scope: SubscriptionScope, warm: bool) -> Vec<String> {
    let cfg = config();
    let (tx, mut replies) = futures::channel::mpsc::unbounded();
    let opcodes = Arc::new(Mutex::new(vec![]));
    let mut sink = AckSink {
        replies: tx,
        opcodes: opcodes.clone(),
        refuse_accounts: scope == SubscriptionScope::MarketDataOnly,
        ack_code: Some(200),
    };
    let (events, mut event_rx) = mpsc::channel(128);
    let mut subscriptions = SubscriptionManager::default();
    let last = Arc::new(AtomicI64::new(0));
    let map = subscribe_all(
        &cfg,
        "fixture-access-token",
        &mut sink,
        &mut replies,
        warm.then_some(1200),
        1200,
        false,
        &events,
        &mut subscriptions,
        &last,
        1000,
        1,
        1,
        scope,
    )
    .await
    .unwrap();
    assert_eq!(map.len(), 2);
    let expected = if scope == SubscriptionScope::MarketDataOnly {
        2
    } else {
        6
    };
    assert_eq!(subscriptions.active_count(), expected);
    assert_eq!(subscriptions.pending_counts().bars, 0);
    let mut acks = vec![];
    while let Ok(event) = event_rx.try_recv() {
        if let WsEvent::SubscriptionAck {
            subscription_type, ..
        } = event
        {
            acks.push(subscription_type);
        }
    }
    assert_eq!(acks.len() as u32, expected);
    if scope == SubscriptionScope::MarketDataOnly {
        assert_eq!(acks, vec!["bars", "bars"]);
    }
    opcodes.lock().unwrap().clone()
}

#[tokio::test]
async fn md_only_does_not_activate_rejected_or_missing_status_ack() {
    for ack_code in [Some(401), None] {
        let cfg = config();
        let (tx, mut replies) = futures::channel::mpsc::unbounded();
        let mut sink = AckSink {
            replies: tx,
            opcodes: Arc::new(Mutex::new(vec![])),
            refuse_accounts: true,
            ack_code,
        };
        let (events, mut event_rx) = mpsc::channel(128);
        let mut subscriptions = SubscriptionManager::default();
        let last = Arc::new(AtomicI64::new(0));
        assert!(
            subscribe_all(
                &cfg,
                "fixture",
                &mut sink,
                &mut replies,
                None,
                1200,
                false,
                &events,
                &mut subscriptions,
                &last,
                1000,
                1,
                1,
                SubscriptionScope::MarketDataOnly
            )
            .await
            .is_err()
        );
        assert_eq!(subscriptions.active_count(), 0);
        while let Ok(event) = event_rx.try_recv() {
            assert!(!matches!(event, WsEvent::SubscriptionAck { .. }));
        }
        assert_eq!(*sink.opcodes.lock().unwrap(), vec!["BarsGetAndSubscribe"]);
    }
}

#[tokio::test]
async fn md_only_initial_and_resubscribe_never_emit_account_opcodes() {
    for warm in [false, true] {
        assert_eq!(
            scoped_subscribe(SubscriptionScope::MarketDataOnly, warm).await,
            vec!["BarsGetAndSubscribe", "BarsGetAndSubscribe"]
        );
    }
}

#[tokio::test]
async fn default_trading_scope_preserves_all_existing_subscriptions() {
    assert_eq!(
        scoped_subscribe(SubscriptionScope::Trading, false).await,
        vec![
            "BarsGetAndSubscribe",
            "BarsGetAndSubscribe",
            "PositionsGetAndSubscribeV2",
            "OrdersGetAndSubscribeV2",
            "StopOrdersGetAndSubscribeV2",
            "TradesGetAndSubscribeV2"
        ]
    );
}

#[tokio::test]
async fn md_only_hub_connects_acks_receives_and_unsubscribes_on_owned_ws() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let server_seen = seen.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let mut subscriptions = vec![];
        let mut unsubscriptions = 0;
        while let Some(frame) = ws.next().await {
            match frame.unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    let opcode = value["opcode"].as_str().unwrap().to_string();
                    server_seen.lock().unwrap().push(opcode.clone());
                    match opcode.as_str() {
                        "BarsGetAndSubscribe" => {
                            assert!(value.get("portfolio").is_none());
                            let guid = value["guid"].as_str().unwrap().to_string();
                            subscriptions.push(guid.clone());
                            ws.send(Message::Text(
                                serde_json::json!({"guid":guid,"httpCode":200})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                            if subscriptions.len() == 2 {
                                for guid in &subscriptions {
                                    ws.send(Message::Text(serde_json::json!({"guid":guid,"data":{
                                        "time":1200,"open":100.,"high":105.,"low":99.,"close":103.,"volume":10.
                                    }}).to_string().into())).await.unwrap();
                                }
                            }
                        }
                        "unsubscribe" => {
                            unsubscriptions += 1;
                        }
                        other => panic!("forbidden account/order opcode: {other}"),
                    }
                }
                Message::Ping(body) => ws.send(Message::Pong(body)).await.unwrap(),
                Message::Close(_) => {
                    ws.flush().await.unwrap();
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(unsubscriptions, 2);
    });
    let mut cfg = config();
    cfg.ws_url = format!("ws://{addr}");
    let token =
        TokenProvider::new_with_token("http://127.0.0.1:1/unused-oauth", "fixture", "fixture");
    let (handle, mut events) = WsHub::start_market_data_only(cfg, token);
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut raw_symbols = std::collections::BTreeSet::new();
        let mut active = false;
        while let Some(event) = events.recv().await {
            match event {
                WsEvent::SubscriptionStats {
                    desired, active: n, ..
                } => {
                    assert!(desired <= 2);
                    active = n == 2;
                }
                WsEvent::SubscriptionAck {
                    subscription_type, ..
                } => assert_eq!(subscription_type, "bars"),
                WsEvent::Raw { value, .. } => {
                    raw_symbols.insert(value["data"]["symbol"].as_str().unwrap().to_string());
                }
                _ => {}
            }
            if active && raw_symbols.len() == 2 {
                break;
            }
        }
        assert_eq!(
            raw_symbols,
            std::collections::BTreeSet::from(["IMOEXF".into(), "USDRUBF".into()])
        );
    })
    .await;
    handle.shutdown().await;
    result.unwrap();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "BarsGetAndSubscribe",
            "BarsGetAndSubscribe",
            "unsubscribe",
            "unsubscribe"
        ]
    );
}
