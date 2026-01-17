use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::auth::TokenProvider;
use crate::config::AlorGatewayConfig;
use crate::ws_subscriptions::{
    build_bars_subscribe, build_orders_subscribe, build_positions_subscribe,
};

#[derive(Debug)]
pub enum WsEvent {
    Raw(Value),
    Conn(ConnEvent),
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
    Shutdown,
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
            while should_run {
                if event_tx.send(WsEvent::Conn(ConnEvent::Reconnecting)).await.is_err() {
                    break;
                }
                match connect_and_run(&cfg, &token_provider, &event_tx, &mut cmd_rx).await {
                    Ok(()) => {
                        info!("ws hub ended gracefully");
                        should_run = false;
                    }
                    Err(error) => {
                        warn!(?error, "ws hub error; reconnecting");
                        let _ = event_tx.send(WsEvent::Conn(ConnEvent::Disconnected)).await;
                        tokio::time::sleep(Duration::from_millis(cfg.backoff_initial_ms)).await;
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

    info!("ws hub connected");
    let _ = event_tx.send(WsEvent::Conn(ConnEvent::Connected)).await;

    subscribe_all(cfg, &token, &mut ws_sink, &mut ws_stream, None).await?;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(HubCommand::Resubscribe { from_ts }) => {
                        info!("ws hub resubscribe requested");
                        subscribe_all(cfg, &token, &mut ws_sink, &mut ws_stream, from_ts).await?;
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
                        if let Ok(val) = serde_json::from_str::<Value>(&txt) {
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
) -> anyhow::Result<()> {
    let bars_from_ts = from_ts.unwrap_or(cfg.from_ts);
    let skip_history = from_ts.is_none() && cfg.skip_history_bars;
    for symbol in &cfg.symbols {
        let (guid, msg) = build_bars_subscribe(
            cfg,
            symbol,
            token,
            bars_from_ts,
            skip_history,
        );
        send_and_ack(ws_sink, ws_stream, &guid, &msg, "bars").await?;
    }

    let (guid, msg) = build_positions_subscribe(cfg, token, cfg.skip_history_positions);
    send_and_ack(ws_sink, ws_stream, &guid, &msg, "positions").await?;

    let (guid, msg) = build_orders_subscribe(cfg, token, cfg.skip_history_orders);
    send_and_ack(ws_sink, ws_stream, &guid, &msg, "orders").await?;

    Ok(())
}

async fn send_and_ack(
    ws_sink: &mut (impl futures_util::sink::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ws_stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    guid: &str,
    msg: &str,
    label: &str,
) -> anyhow::Result<()> {
    info!(guid, label, "ws subscribe send");
    ws_sink.send(Message::Text(msg.to_string().into())).await?;

    let ack = read_until_guid(ws_stream, guid, Duration::from_secs(5)).await?;
    debug!(?ack, guid, label, "ws subscribe ack");
    Ok(())
}

async fn read_until_guid(
    stream: &mut (impl futures_util::stream::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    guid: &str,
    timeout_dur: Duration,
) -> anyhow::Result<Value> {
    let fut = async move {
        while let Some(msg) = stream.next().await {
            let msg = msg?;
            if let Message::Text(txt) = msg {
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if guid_of(&val).as_deref() == Some(guid) {
                        return Ok(val);
                    }
                }
            }
        }
        Err(anyhow::anyhow!("ws stream ended before response"))
    };

    match tokio::time::timeout(timeout_dur, fut).await {
        Ok(inner) => inner,
        Err(_) => Err(anyhow::anyhow!("ws subscribe timeout")),
    }
}

fn guid_of(value: &Value) -> Option<String> {
    value
        .get("guid")
        .and_then(Value::as_str)
        .or_else(|| value.get("requestGuid").and_then(Value::as_str))
        .map(|value| value.to_string())
}
