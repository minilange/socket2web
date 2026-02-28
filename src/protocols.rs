pub mod tcp;
use futures_util::{stream::{SplitSink, SplitStream}};
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
}