use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

pub struct MockCwsServer {
    addr: SocketAddr,
}

impl MockCwsServer {
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                tokio::spawn(handle_connection(stream));
            }
        });
        Ok(Self { addr })
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }
}

async fn handle_connection(stream: tokio::net::TcpStream) {
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
        let guid = value
            .get("guid")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let response = serde_json::json!({
            "guid": guid,
            "httpCode": 200,
            "status": 200,
            "cwsAuthorized": true,
        });
        let _ = sink.send(Message::Text(response.to_string().into())).await;
    }
}
