use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use chrono::{DateTime, TimeZone, Utc};
use tracing::{debug, info, warn};

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::gateway_events::{GatewayEvent, log_event};
use crate::ws_subscriptions::{
    build_bars_subscribe, build_orders_subscribe, build_positions_subscribe,
};

#[derive(Debug)]
pub enum WsEvent {
    Raw(Value),
    Conn(ConnEvent),
    Subscribed { wallclock_ts: i64 },
}

#[derive(Debug)]
pub enum ConnEvent {
    Connected,
    Disconnected,
    Reconnecting,
}

#[derive(Debug, Clone)]
pub struct WsHubHandle {
    cmd_tx: mpsc::Sender<HubCommand>,
}

#[derive(Debug)]
enum HubCommand {
    Resubscribe { from_ts: Option<i64> },
    Reconnect,
    Shutdown,
}

#[derive(Debug, Clone)]
struct Subscription {
    guid: String,
    symbol: String,
    subscription_type: String,
    is_active: bool,
}

#[derive(Debug, Default)]
struct SubscriptionManager {
    desired_subscriptions: HashMap<String, Subscription>,
    active_subscriptions: HashMap<String, Subscription>,
}

impl SubscriptionManager {
    fn add_subscription(&mut self, subscription: Subscription) {
        self.desired_subscriptions
            .insert(subscription.guid.clone(), subscription);
    }

    fn activate_subscription(&mut self, guid: &str) -> Option<Subscription> {
        if let Some(mut subscription) = self.desired_subscriptions.remove(guid) {
            subscription.is_active = true;
            self.active_subscriptions
                .insert(guid.to_string(), subscription.clone());
            return Some(subscription);
        }
        None
    }

    fn reset(&mut self) {
        self.desired_subscriptions.clear();
        self.active_subscriptions.clear();
    }
}

pub struct WsHub;

impl WsHub {
    pub fn start(
        cfg: AlorGatewayConfig,
        token_provider: TokenProvider,
    ) -> (WsHubHandle, mpsc::Receiver<WsEvent>) {
        let (event_tx, event_rx) = mpsc::channel(1024);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);

        tokio::spawn(async move {
            let mut should_run = true;
            let mut attempt: u64 = 0;
            let mut backoff = Duration::from_millis(cfg.backoff_initial_ms);
            while should_run {
                if event_tx.send(WsEvent::Conn(ConnEvent::Reconnecting)).await.is_err() {
                    break;
                }
                attempt += 1;
                log_event(GatewayEvent::Reconnecting { attempt });
                match connect_and_run(&cfg, &token_provider, &event_tx, &mut cmd_rx).await {
                    Ok(()) => {
                        info!("ws hub ended gracefully");
                        should_run = false;
                    }
                    Err(error) => {
                        warn!(?error, "ws hub error; reconnecting");
                        let _ = event_tx.send(WsEvent::Conn(ConnEvent::Disconnected)).await;
                        tokio::time::sleep(jittered(backoff)).await;
                        backoff = next_backoff(backoff, &cfg);
                    }
                }
            }
        });

        (
            WsHubHandle { cmd_tx },
            event_rx,
        )
    }
}

impl WsHubHandle {
    pub async fn resubscribe_all(&self) {
        let _ = self
            .cmd_tx
            .send(HubCommand::Resubscribe { from_ts: None })
            .await;
    }

    pub async fn resubscribe_from(&self, from_ts: i64) {
        let _ = self
            .cmd_tx
            .send(HubCommand::Resubscribe { from_ts: Some(from_ts) })
            .await;
    }

    pub async fn reconnect(&self) {
        let _ = self.cmd_tx.send(HubCommand::Reconnect).await;
    }

    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(HubCommand::Shutdown).await;
    }
}

async fn connect_and_run(
    cfg: &AlorGatewayConfig,
    token_provider: &TokenProvider,
    event_tx: &mpsc::Sender<WsEvent>,
    cmd_rx: &mut mpsc::Receiver<HubCommand>,
) -> anyhow::Result<()> {
    let token = token_provider.access_token().await?;
    let (ws_stream, _) = tokio_tungstenite::connect_async(&cfg.ws_url).await?;
    let (mut ws_sink, mut ws_stream) = ws_stream.split();
    let mut subscription_manager = SubscriptionManager::default();

    info!("ws hub connected");
    log_event(GatewayEvent::Connected);
    let _ = event_tx.send(WsEvent::Conn(ConnEvent::Connected)).await;

    let subscribe_wallclock_ts = Utc::now().timestamp();
    let _ = event_tx
        .send(WsEvent::Subscribed {
            wallclock_ts: subscribe_wallclock_ts,
        })
        .await;
    subscription_manager.reset();
    let mut bars_guid_map = subscribe_all(
        cfg,
        &token,
        &mut ws_sink,
        &mut ws_stream,
        None,
        event_tx,
        &mut subscription_manager,
    )
    .await?;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(HubCommand::Resubscribe { from_ts }) => {
                        info!("ws hub resubscribe requested");
                        log_event(GatewayEvent::ResyncStarted);
                        let subscribe_wallclock_ts = Utc::now().timestamp();
                        let _ = event_tx
                            .send(WsEvent::Subscribed {
                                wallclock_ts: subscribe_wallclock_ts,
                            })
                            .await;
                        subscription_manager.reset();
                        bars_guid_map = subscribe_all(
                            cfg,
                            &token,
                            &mut ws_sink,
                            &mut ws_stream,
                            from_ts,
                            event_tx,
                            &mut subscription_manager,
                        )
                        .await?;
                        log_event(GatewayEvent::ResyncDone);
                    }
                    Some(HubCommand::Reconnect) => {
                        info!("ws hub reconnect requested");
                        return Err(anyhow::anyhow!("forced reconnect"));
                    }
                    Some(HubCommand::Shutdown) | None => {
                        info!("ws hub shutdown requested");
                        return Ok(());
                    }
                }
            }
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        tracing::trace!(payload = %txt, "ws recv text");
                        if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                            let val = attach_symbol(val, &bars_guid_map);
                            if event_tx.send(WsEvent::Raw(val)).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws_sink.send(Message::Pong(payload)).await?;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        info!(?frame, "ws close received");
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => return Ok(()),
                }
            }
        }
    }
}

async fn subscribe_all(
    cfg: &AlorGatewayConfig,
    token: &str,
    ws_sink: &mut (impl futures_util::sink::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    from_ts: Option<i64>,
    event_tx: &mpsc::Sender<WsEvent>,
    subscription_manager: &mut SubscriptionManager,
) -> anyhow::Result<HashMap<String, String>> {
    let mut bars_guid_map = HashMap::new();
    let bars_from_ts = from_ts.unwrap_or(cfg.from_ts);
    let skip_history = from_ts.is_none() && cfg.skip_history_bars;
    debug!(
        bars_from_ts,
        bars_from_ts_rfc3339 = %format_ts(bars_from_ts),
        skip_history,
        "ws bars subscribe window"
    );
    for symbol in &cfg.symbols {
        let (guid, msg) = build_bars_subscribe(
            cfg,
            symbol,
            token,
            bars_from_ts,
            skip_history,
        );
        bars_guid_map.insert(guid.clone(), symbol.clone());
        subscription_manager.add_subscription(Subscription {
            guid: guid.clone(),
            symbol: symbol.clone(),
            subscription_type: "bars".to_string(),
            is_active: false,
        });
        send_and_ack(
            ws_sink,
            ws_stream,
            &guid,
            &msg,
            "bars",
            event_tx,
            &bars_guid_map,
            subscription_manager,
        )
        .await?;
    }

    let positions_skip_history = if from_ts.is_some() {
        false
    } else {
        cfg.skip_history_positions
    };
    let (guid, msg) = build_positions_subscribe(cfg, token, positions_skip_history);
    subscription_manager.add_subscription(Subscription {
        guid: guid.clone(),
        symbol: cfg.portfolio.clone(),
        subscription_type: "positions".to_string(),
        is_active: false,
    });
    send_and_ack(
        ws_sink,
        ws_stream,
        &guid,
        &msg,
        "positions",
        event_tx,
        &bars_guid_map,
        subscription_manager,
    )
    .await?;

    let orders_skip_history = if from_ts.is_some() {
        false
    } else {
        cfg.skip_history_orders
    };
    let (guid, msg) = build_orders_subscribe(cfg, token, orders_skip_history);
    subscription_manager.add_subscription(Subscription {
        guid: guid.clone(),
        symbol: cfg.portfolio.clone(),
        subscription_type: "orders".to_string(),
        is_active: false,
    });
    send_and_ack(
        ws_sink,
        ws_stream,
        &guid,
        &msg,
        "orders",
        event_tx,
        &bars_guid_map,
        subscription_manager,
    )
    .await?;

    Ok(bars_guid_map)
}

async fn send_and_ack(
    ws_sink: &mut (impl futures_util::sink::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    guid: &str,
    msg: &str,
    label: &str,
    event_tx: &mpsc::Sender<WsEvent>,
    bars_guid_map: &HashMap<String, String>,
    subscription_manager: &mut SubscriptionManager,
) -> anyhow::Result<()> {
    info!(guid, label, "ws subscribe send");
    debug!(payload = %msg, guid, label, "ws subscribe payload");
    ws_sink.send(Message::Text(msg.to_string().into())).await?;

    let ack = read_until_guid(
        ws_stream,
        guid,
        Duration::from_secs(5),
        event_tx,
        bars_guid_map,
        subscription_manager,
    )
    .await?;
    if let Some(subscription) = subscription_manager.activate_subscription(guid) {
        log_event(GatewayEvent::Subscribed {
            symbol: subscription.symbol,
            subscription_type: subscription.subscription_type,
        });
    }
    debug!(payload = %ack, guid, label, "ws subscribe ack");
    Ok(())
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
    event_tx: &mpsc::Sender<WsEvent>,
    bars_guid_map: &HashMap<String, String>,
    subscription_manager: &SubscriptionManager,
) -> anyhow::Result<Value> {
    let fut = async move {
        while let Some(msg) = stream.next().await {
            let msg = msg?;
            if let Message::Text(txt) = msg {
                tracing::trace!(payload = %txt, "ws recv text (awaiting guid)");
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if guid_of(&val).as_deref() == Some(guid) {
                        return Ok(val);
                    }
                    let val = attach_symbol(val, bars_guid_map);
                    let _ = event_tx.send(WsEvent::Raw(val)).await;
                }
            }
        }
        Err(anyhow::anyhow!("ws stream ended before response"))
    };

    match tokio::time::timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => {
            if let Some(subscription) = subscription_manager.desired_subscriptions.get(guid) {
                log_event(GatewayEvent::AckTimeout {
                    symbol: subscription.symbol.clone(),
                    subscription_type: subscription.subscription_type.clone(),
                });
            } else {
                log_event(GatewayEvent::AckTimeout {
                    symbol: "<unknown>".to_string(),
                    subscription_type: "<unknown>".to_string(),
                });
            }
            Err(anyhow::anyhow!("ws subscribe timeout"))
        }
    }
}

fn guid_of(value: &Value) -> Option<String> {
    value
        .get("guid")
        .and_then(Value::as_str)
        .or_else(|| value.get("requestGuid").and_then(Value::as_str))
        .map(|value| value.to_string())
}

fn attach_symbol(mut value: Value, bars_guid_map: &HashMap<String, String>) -> Value {
    let Some(guid) = guid_of(&value) else {
        return value;
    };
    let Some(symbol) = bars_guid_map.get(&guid) else {
        return value;
    };
    let Some(data) = value.get_mut("data") else {
        return value;
    };
    if let Some(obj) = data.as_object_mut() {
        if !obj.contains_key("symbol") && !obj.contains_key("code") {
            obj.insert("symbol".to_string(), Value::String(symbol.clone()));
        }
    } else if let Some(arr) = data.as_array_mut() {
        for item in arr.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                if !obj.contains_key("symbol") && !obj.contains_key("code") {
                    obj.insert("symbol".to_string(), Value::String(symbol.clone()));
                }
            }
        }
    }
    value
}

fn format_ts(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
        .to_rfc3339()
}

fn next_backoff(current: Duration, cfg: &AlorGatewayConfig) -> Duration {
    (current * cfg.backoff_multiplier as u32).min(Duration::from_millis(cfg.backoff_max_ms))
}

fn jittered(duration: Duration) -> Duration {
    let jitter_pct = 0.2;
    let millis = duration.as_millis() as f64;
    let jitter = rand::random::<f64>() * jitter_pct;
    let offset = millis * jitter;
    let lower = millis - offset;
    let upper = millis + offset;
    let jittered = lower + (rand::random::<f64>() * (upper - lower));
    Duration::from_millis(jittered.max(0.0) as u64)
}
