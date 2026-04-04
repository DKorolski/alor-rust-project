#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

#[allow(dead_code)]
#[derive(Debug)]
pub enum MockWsEvent {
    Connected { id: usize },
    Text { id: usize, text: String },
    Ping { id: usize },
    Closed { id: usize },
}

#[allow(dead_code)]
enum ServerCommand {
    SendText { id: usize, text: String },
    SetRespondToPing { id: usize, respond: bool },
    Close { id: usize },
}

struct ConnectionState {
    tx: mpsc::Sender<Message>,
    respond_to_ping: Arc<AtomicBool>,
}

pub struct MockWsServer {
    addr: SocketAddr,
    cmd_tx: mpsc::Sender<ServerCommand>,
}

impl MockWsServer {
    pub async fn start() -> anyhow::Result<(Self, mpsc::Receiver<MockWsEvent>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (event_tx, event_rx) = mpsc::channel(256);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(256);

        tokio::spawn(async move {
            let mut next_id: usize = 0;
            let mut connections: HashMap<usize, ConnectionState> = HashMap::new();
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(ServerCommand::SendText { id, text }) => {
                                if let Some(conn) = connections.get(&id) {
                                    let _ = conn.tx.send(Message::Text(text.into())).await;
                                }
                            }
                            Some(ServerCommand::SetRespondToPing { id, respond }) => {
                                if let Some(conn) = connections.get(&id) {
                                    conn.respond_to_ping.store(respond, Ordering::SeqCst);
                                }
                            }
                            Some(ServerCommand::Close { id }) => {
                                if let Some(conn) = connections.get(&id) {
                                    let _ = conn.tx.send(Message::Close(None)).await;
                                }
                            }
                            None => break,
                        }
                    }
                    accept = listener.accept() => {
                        let Ok((stream, _)) = accept else {
                            continue;
                        };
                        let id = next_id;
                        next_id = next_id.saturating_add(1);
                        let (tx, rx) = mpsc::channel(128);
                        let respond_to_ping = Arc::new(AtomicBool::new(true));
                        connections.insert(
                            id,
                            ConnectionState {
                                tx: tx.clone(),
                                respond_to_ping: respond_to_ping.clone(),
                            },
                        );
                        let event_tx_clone = event_tx.clone();
                        tokio::spawn(handle_connection(id, stream, rx, respond_to_ping, event_tx_clone));
                        let _ = event_tx.send(MockWsEvent::Connected { id }).await;
                    }
                }
            }
        });

        Ok((Self { addr, cmd_tx }, event_rx))
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub async fn send_text(&self, id: usize, text: impl Into<String>) -> anyhow::Result<()> {
        self.cmd_tx
            .send(ServerCommand::SendText {
                id,
                text: text.into(),
            })
            .await
            .map_err(|_| anyhow::anyhow!("mock ws command channel closed"))
    }

    pub async fn set_respond_to_ping(&self, id: usize, respond: bool) -> anyhow::Result<()> {
        self.cmd_tx
            .send(ServerCommand::SetRespondToPing { id, respond })
            .await
            .map_err(|_| anyhow::anyhow!("mock ws command channel closed"))
    }

    #[allow(dead_code)]
    pub async fn close(&self, id: usize) -> anyhow::Result<()> {
        self.cmd_tx
            .send(ServerCommand::Close { id })
            .await
            .map_err(|_| anyhow::anyhow!("mock ws command channel closed"))
    }
}

pub fn parse_opcode(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .get("opcode")
        .and_then(Value::as_str)
        .map(|opcode| opcode.to_string())
}

pub fn parse_guid(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .get("guid")
        .and_then(Value::as_str)
        .map(|guid| guid.to_string())
}

async fn handle_connection(
    id: usize,
    stream: tokio::net::TcpStream,
    mut cmd_rx: mpsc::Receiver<Message>,
    respond_to_ping: Arc<AtomicBool>,
    event_tx: mpsc::Sender<MockWsEvent>,
) {
    let Ok(ws_stream) = accept_async(stream).await else {
        return;
    };
    let (mut sink, mut stream) = ws_stream.split();
    loop {
        tokio::select! {
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let _ = event_tx
                            .send(MockWsEvent::Text { id, text: text.to_string() })
                            .await;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = event_tx.send(MockWsEvent::Ping { id }).await;
                        if respond_to_ping.load(Ordering::SeqCst) {
                            let _ = sink.send(Message::Pong(payload)).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        let _ = event_tx.send(MockWsEvent::Closed { id }).await;
                        return;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        let _ = event_tx.send(MockWsEvent::Closed { id }).await;
                        return;
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(msg) => {
                        let _ = sink.send(msg).await;
                    }
                    None => return,
                }
            }
        }
    }
}
