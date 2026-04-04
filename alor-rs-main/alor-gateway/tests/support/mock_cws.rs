#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockCwsRequest {
    pub connection_id: usize,
    pub opcode: String,
    pub guid: String,
    pub order_id: Option<String>,
}

pub struct MockCwsServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<MockCwsRequest>>>,
    close_tx: Option<oneshot::Sender<()>>,
}

impl MockCwsServer {
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_task = requests.clone();
        let (close_tx, mut close_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut next_connection_id = 1usize;
            loop {
                tokio::select! {
                    biased;
                    _ = &mut close_rx => {
                        break;
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            continue;
                        };
                        let connection_id = next_connection_id;
                        next_connection_id = next_connection_id.saturating_add(1);
                        tokio::spawn(handle_connection(stream, connection_id, requests_task.clone()));
                    }
                }
            }
        });
        Ok(Self {
            addr,
            requests,
            close_tx: Some(close_tx),
        })
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn snapshot_requests(&self) -> Vec<MockCwsRequest> {
        self.requests.lock().clone()
    }
}

impl Drop for MockCwsServer {
    fn drop(&mut self) {
        if let Some(close_tx) = self.close_tx.take() {
            let _ = close_tx.send(());
        }
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    connection_id: usize,
    requests: Arc<Mutex<Vec<MockCwsRequest>>>,
) {
    let Ok(ws_stream) = accept_async(stream).await else {
        return;
    };
    let (mut sink, mut stream) = ws_stream.split();
    while let Some(msg) = stream.next().await {
        let Ok(Message::Text(text)) = msg else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let opcode = value
            .get("opcode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let guid = value
            .get("guid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let order_id = value
            .get("orderId")
            .and_then(|value| value.as_i64().map(|value| value.to_string()))
            .or_else(|| {
                value
                    .get("orderId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            });
        requests.lock().push(MockCwsRequest {
            connection_id,
            opcode,
            guid: guid.clone(),
            order_id,
        });
        let response = serde_json::json!({
            "guid": guid,
            "httpCode": 200,
            "status": 200,
            "cwsAuthorized": true,
            "orderNumber": if value.get("opcode").and_then(Value::as_str) == Some("create:limit") {
                serde_json::json!(1_000_000_i64 + connection_id as i64)
            } else {
                Value::Null
            },
        });
        let _ = sink.send(Message::Text(response.to_string().into())).await;
    }
}
