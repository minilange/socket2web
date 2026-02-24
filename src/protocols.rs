pub mod tcp;
use futures_util::{SinkExt, stream::{SplitSink, SplitStream}};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

use crate::protocols::tcp::TcpConnection;

pub enum Protocol {
    Tcp(TcpConnection),
}

pub type WebSocketSender = SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>;
pub type WebSocketReceiver = SplitStream<WebSocketStream<tokio::net::TcpStream>>;

pub trait ProxyTarget: Send {
    async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> ;
    async fn attach_handles(
        &mut self,
        ws_sender: WebSocketSender, 
        ws_receiver: WebSocketReceiver
    );

    async fn proxy_success(&self, ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Successfully connected, strarting to proxy",
        });
        let _ = ws_sender.send(Message::text(json_str.to_string())).await;
    }

    async fn proxy_failed(&self, reason: &str, ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Connection failed, cancelling proxy",
            "reason": reason
        });
        let _ = ws_sender.send(Message::Text(json_str.to_string())).await;
    }

}