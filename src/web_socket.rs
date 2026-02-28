use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::arguments;
use crate::protocols::tcp::TcpConnection;
use crate::protocols::{Protocol, ProxyTarget, WebSocketReceiver, WebSocketSender};

pub struct WebSocket {
    address: String,
}

impl WebSocket {
    pub fn new(config: &arguments::WebSocketArguments) -> Self {
        Self {
            address: format!("{}:{}", config.ip, config.port),
        }
    }

    pub async fn start_listening(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.address)
            .await
            .expect(&format!("Failed to bind to address: {}", &self.address));
        tracing::info!("Now listening to address: {}", &self.address);

        while let Ok((stream, addr)) = listener.accept().await {
            let _ = tokio::spawn(WebSocket::handle_connection(stream, addr));
        }

        return Ok(());
    }

    async fn handle_connection(stream: TcpStream, addr: SocketAddr) {
        tracing::info!("Got new connection {}", addr.ip().to_string());

        let ws_stream = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!("WebSocket accept failed: {}", e);
                return;
            }
        };
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        match WebSocket::handle_handshake(&mut ws_sender, &mut ws_receiver).await {
            Ok(Protocol::Tcp(mut proxy)) => {
                if let Err(e) = proxy.connect().await {
                    WebSocket::proxy_failed(&e.to_string(), &mut ws_sender).await;
                } else {
                    WebSocket::proxy_success(&mut ws_sender).await;
                    proxy.attach_handles(ws_sender, ws_receiver).await;
                }
            }
            Err(e) => {
                tracing::error!("Handshake failed, closing connection: {}", e);
                let _ = ws_sender.close().await;
            }
        }
        tracing::info!("End of connection for {}", addr.ip());
        return;
    }

    async fn handle_handshake(
        ws_sender: &mut WebSocketSender,
        ws_receiver: &mut WebSocketReceiver,
    ) -> Result<Protocol, Box<dyn std::error::Error + Send + Sync>> {
        let _ = ws_sender.send(WebSocket::get_greeting()).await;

        if let Some(inc_msg) = ws_receiver.next().await {
            match inc_msg {
                Ok(Message::Text(json_str)) => {
                    return WebSocket::read_greeting(&json_str);
                }
                Err(e) => return Err(Box::new(e)), // propagate error
                _ => return Err("Received unexpected, closing connection!".into()), // propagate as custom error
            }
        } else {
            return Err("No handshake was recieved".into());
        }
    }

    fn get_greeting() -> Message {
        let greeting = serde_json::json!({
            "protocol": "tcp",
            "target_ip": "IP Address",
            "target_port": "Port for IP Address"
        });
        return Message::Text(greeting.to_string());
    }

    fn read_greeting(json_str: &str) -> Result<Protocol, Box<dyn std::error::Error + Send + Sync>> {
        let handshake: serde_json::Value = serde_json::from_str(&json_str)?;

        let protocol = handshake
            .get("protocol")
            .and_then(|v| v.as_str())
            .ok_or("Missing or invalid protocol field");

        match protocol {
            Ok("tcp") => {
                let target_ip = handshake
                    .get("target_ip")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing or invalid target_ip")?
                    .to_string();
                let target_port = handshake
                    .get("target_port")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing or invalid target_port")?
                    as u16;
                Ok(Protocol::Tcp(TcpConnection::new(target_ip, target_port)))
            }
            Err(e) => Err(e.into()),
            _ => Err("Unsupported protocol".into()),
        }
    }

    async fn proxy_success(ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Successfully connected, strarting to proxy",
        });
        let _ = ws_sender.send(Message::text(json_str.to_string())).await;
    }

    async fn proxy_failed(reason: &str, ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Connection failed, cancelling proxy",
            "reason": reason
        });
        let _ = ws_sender.send(Message::Text(json_str.to_string())).await;
    }
}
