use crate::protocols::{ProtocolSchema, ProxyTarget, Schema, WebSocketReceiver, WebSocketSender};
use futures_util::{SinkExt, StreamExt};
use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

/// Represents a proxied TCP connection to a target server.
///
/// Manages the lifecycle of a TCP connection including establishing
/// the connection and bidirectional data proxying between a WebSocket
/// client and the TCP target.
pub struct TcpConnection {
    ip_address: String,
    port: u16,
    stream: Option<TcpStream>,
}

impl TcpConnection {
    /// Creates a new `TcpConnection` targeting the given IP address and port.
    ///
    /// The connection is not established until [`connect`](TcpConnection::connect) is called.
    pub fn new(ip_address: String, port: u16) -> Self {
        Self {
            ip_address: ip_address,
            port: port,
            stream: None,
        }
    }

    /// Reads data from the TCP stream and forwards it to the WebSocket sender.
    ///
    /// Runs in a loop until the TCP stream sends EOF, an error occurs,
    /// or the cancellation token is triggered. Ensures the token is cancelled
    /// and the WebSocket sender is closed upon exit.
    async fn proxy_tcp_to_web(
        mut tcp_reader: OwnedReadHalf,
        mut ws_sender: WebSocketSender,
        token: tokio_util::sync::CancellationToken,
    ) {
        let mut buffer = vec![0u8; 65536];
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    break;
                }
                rx = tcp_reader.read(&mut buffer) => {
                    match rx {
                        Ok(0) => {
                            tracing::debug!("TCP stream reached end of stream");
                            token.cancel();
                            break;
                        }
                        Ok(rx_n) => {
                            if let Err(tx_err) = ws_sender
                            .send(Message::Binary(buffer[..rx_n].to_vec()))
                            .await
                            {
                                tracing::error!("Encountered an error while transmitting to web: {}", tx_err);
                                break;
                            }
                        }
                        Err(err) => {
                            if err.kind() != io::ErrorKind::WouldBlock {
                                tracing::error!("Failed to read from TCP socket: {}", err);
                                token.cancel();
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Ensure token is cancelled if out of loop
        if !token.is_cancelled() {
            token.cancel();
        }
        let _ = ws_sender.close().await;
        tracing::trace!("TCP->Web proxy ended");
    }

    /// Reads messages from the WebSocket receiver and forwards them to the TCP writer.
    ///
    /// Handles `Text`, `Binary`, and `Frame` messages by writing their data to TCP.
    /// `Ping` and `Pong` messages are ignored (handled at the WebSocket layer).
    /// Runs until the WebSocket stream closes, an error occurs, or the
    /// cancellation token is triggered. Ensures the token is cancelled and
    /// the TCP writer is shut down upon exit.
    async fn proxy_web_to_tcp(
        mut ws_receiver: WebSocketReceiver,
        mut tcp_writer: OwnedWriteHalf,
        token: tokio_util::sync::CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = token.cancelled() => {
                    tracing::debug!("Web->TCP cancelled, shutting down");
                    break;
                }
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Err(err) = tcp_writer.write_all(text.as_bytes()).await {
                                tracing::error!("Encountered an error while sending 'Text' to tcp: {}", err);
                                break;
                            }
                        }
                        Some(Ok(Message::Binary(bin))) => {
                            if let Err(err) = tcp_writer.write_all(&bin).await {
                                tracing::error!("Encountered an error while sending 'Binary' to tcp: {}", err);
                                break;
                            }
                        }
                        Some(Ok(Message::Pong(_))) |
                        Some(Ok(Message::Ping(_))) => { }
                        Some(Ok(Message::Frame(frame))) => {
                            if let Err(err) = tcp_writer.write_all(&frame.into_data()).await {
                                tracing::error!("Encountered an error while sending 'Frame' to tcp: {}", err);
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            tracing::debug!("WebSocket closed by client");
                            token.cancel();
                            break;
                        }
                        Some(Err(e)) => {
                            tracing::error!("Web->TCP Error: {}", e);
                            token.cancel();
                            break;
                        }
                        None => {
                            tracing::debug!("WebSocket stream ended");
                            token.cancel();
                            break;
                        }
                    }
                }
            }
        }
        // Ensure token is cancelled if out of loop
        if !token.is_cancelled() {
            token.cancel();
        }
        let _ = tcp_writer.shutdown().await;
        tracing::trace!("Web->TCP proxy ended");
    }
}

impl ProxyTarget for TcpConnection {
    /// Establishes a TCP connection to the configured target address and port.
    ///
    /// Returns an error if the connection cannot be established.
    async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = TcpStream::connect(format!("{}:{}", &self.ip_address, &self.port))
            .await
            .map_err(|error| format!("Failed to connect: {:?}", error))?;
        self.stream = Some(stream);
        return Ok(());
    }

    /// Splits the TCP stream and starts bidirectional proxying.
    ///
    /// Spawns [`proxy_tcp_to_web`](TcpConnection::proxy_tcp_to_web) on a separate task
    /// and runs [`proxy_web_to_tcp`](TcpConnection::proxy_web_to_tcp) on the current task.
    /// Waits for both directions to complete before returning.
    ///
    /// # Panics
    ///
    /// Logs an error and returns early if called before [`connect`](TcpConnection::connect).
    async fn attach_handles(&mut self, ws_sender: WebSocketSender, ws_receiver: WebSocketReceiver, cancel_token: CancellationToken) {
        let (tcp_reader, tcp_writer) = match self.stream.take() {
            Some(stream) => stream.into_split(),
            None => {
                tracing::error!("Tried to attach handle with no connected stream");
                return;
            }
        };
        tracing::debug!("Attaching proxy handles");

        let server_token = cancel_token.clone();
        let target_token = cancel_token.clone();

        let tcp_to_web = tokio::spawn(TcpConnection::proxy_tcp_to_web(
            tcp_reader,
            ws_sender,
            server_token,
        ));
        let _ = TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, target_token).await;
        let _ = tcp_to_web.await;
    }
}

impl ProtocolSchema for TcpConnection {
    /// Returns the handshake schema for the TCP protocol.
    ///
    /// Requires `target_ip` (the destination IP address) and
    /// `target_port` (the destination port number) from the client.
    fn schema() -> Schema {
        Schema {
            name: "tcp".into(),
            requirements: serde_json::json! ({
                "target_ip": "IP Address",
                "target_port": "Port for IP Address"
            }),
        }
    }
}
