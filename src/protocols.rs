pub mod tcp;

use futures_util::stream::{SplitSink, SplitStream};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

use crate::protocols::tcp::TcpConnection;

/// Enumerates the supported proxy target protocols.
pub enum Protocol {
    /// A raw TCP socket connection.
    Tcp(TcpConnection),
}

/// The sending half of a WebSocket connection.
pub type WebSocketSender = SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>;

/// The receiving half of a WebSocket connection.
pub type WebSocketReceiver = SplitStream<WebSocketStream<tokio::net::TcpStream>>;

use tokio_util::sync::CancellationToken;

/// Trait for proxy targets that can be connected to and proxied through.
///
/// Implementors handle establishing the outbound connection and
/// attaching the bidirectional data flow between the WebSocket and the target.
pub trait ProxyTarget: Send {
    /// Establishes the outbound connection to the target.
    async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Attaches the WebSocket sender and receiver to begin proxying data.
    ///
    /// The provided `cancel_token` allows the caller to signal the proxy
    /// to shut down gracefully from the outside.
    ///
    /// This method should run until the connection is closed from either side
    /// or the cancellation token is triggered.
    async fn attach_handles(&mut self, ws_sender: WebSocketSender, ws_receiver: WebSocketReceiver, cancel_token: CancellationToken);
}

/// Describes the handshake schema for a supported protocol.
///
/// Contains the protocol name and a JSON value describing
/// the fields required in the client's handshake message.
pub struct Schema {
    pub name: String,
    pub requirements: serde_json::Value,
}

/// Trait for protocols that can describe their handshake requirements.
///
/// Implementors return a static protocol name and a JSON schema
/// of the fields a client must provide during the handshake.
/// This is used by [`get_greeting`](crate::web_socket::WebSocket::get_greeting)
/// to build the greeting message sent to connecting clients.
pub trait ProtocolSchema {
    /// Returns the protocol name and its required handshake fields.
    fn schema() -> Schema;
}
