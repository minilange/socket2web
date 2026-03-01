use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use std::time::{self, Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use crate::arguments;
use crate::protocols::tcp::TcpConnection;
use crate::protocols::{Protocol, ProtocolSchema, ProxyTarget, WebSocketReceiver, WebSocketSender};

/// The WebSocket server that accepts client connections and proxies them
/// to target endpoints based on the handshake protocol.
pub struct WebSocket {
    address: String,
    timeout: u64,
    max_lifetime: u64,
    max_connections: u64,
}

/// Tracks an active proxied connection along with its start time.
///
/// Used to enforce connection limits and maximum lifetime policies.
/// Connections that exceed [`max_lifetime`](WebSocket) or have finished
/// are cleaned up during the accept loop.
pub struct ClientConnection<T> {
    conn: tokio::task::JoinHandle<T>,
    cancel_token: CancellationToken,
    start: time::Instant,
    addr: IpAddr,
}

impl WebSocket {
    /// Creates a new `WebSocket` server from the provided CLI configuration.
    pub fn new(config: &arguments::WebSocketArguments) -> Self {
        Self {
            address: format!("{}:{}", config.ip, config.port),
            timeout: config.timeout,
            max_lifetime: config.max_lifetime,
            max_connections: config.max_connections,
        }
    }

    /// Starts the WebSocket server and begins accepting connections.
    ///
    /// Listens for incoming TCP connections, upgrades them to WebSocket,
    /// and spawns a task per connection. clients graceful shutdown via
    /// `Ctrl+C`, cleaning up completed connection clients periodically
    /// and aborting remaining connections on exit.
    pub async fn start_listening(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(&self.address)
            .await
            .map_err(|e| format!("Failed to bind to address {}: {}", &self.address, e))?;
        tracing::info!("Listening on {}", &self.address);

        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        let mut clients = vec![];
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!("Shutdown signal received, draining {} active connections", clients.len());
                    break;
                }
                _ = cleanup_interval.tick() => {
                    clients.retain(|h: &ClientConnection<_>| self.connection_cleanup(h));
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            clients.retain(|h: &ClientConnection<_>| self.connection_cleanup(h));

                            if clients.len() < self.max_connections as usize {
                                let token = CancellationToken::new();
                                let conn = ClientConnection {
                                    addr: addr.ip().clone(),
                                    conn: tokio::spawn(WebSocket::handle_connection(stream, addr, self.timeout, token.clone())),
                                    cancel_token: token,
                                    start: time::Instant::now()
                                };
                                clients.push(conn);
                            }
                            else {
                                tracing::warn!("Max connections ({}) reached, rejecting {}", self.max_connections, addr.ip());
                                tokio::spawn(async move {
                                    if let Ok(ws) = accept_async(stream).await {
                                        let (mut sender, _) = ws.split();
                                        let _ = sender.send(Message::Close(Some(CloseFrame {
                                            code: CloseCode::Again,
                                            reason: "Server at capacity, try again later".into(),
                                        }))).await;
                                    }
                                });
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to accept connection: {}", e)
                        }
                    }
                }
            }
        }

        for mut c in clients {
            c.cancel_token.cancel();
            if timeout(Duration::from_secs(5), &mut c.conn).await.is_err() {
                tracing::warn!(
                    "Connection '{}' did not finish within shutdown timeout, aborting",
                    c.addr
                );
                c.conn.abort();
            }
        }

        return Ok(());
    }

    /// clients a single WebSocket connection from accept to close.
    ///
    /// Upgrades the raw TCP stream to a WebSocket, performs the handshake
    /// to determine the target protocol, connects to the target, and begins
    /// proxying data. Logs and cleans up on handshake or connection failure.
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        timeout: u64,
        cancel_token: CancellationToken,
    ) {
        tracing::info!("New connection from {}", addr.ip());

        let ws_stream = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!("WebSocket upgrade failed for {}: {}", e, addr.ip());
                return;
            }
        };
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        match WebSocket::handle_handshake(&mut ws_sender, &mut ws_receiver, &timeout).await {
            Ok(Protocol::Tcp(mut proxy)) => {
                if let Err(e) = proxy.connect().await {
                    WebSocket::proxy_failed(&e.to_string(), &mut ws_sender).await;
                } else {
                    WebSocket::proxy_success(&mut ws_sender).await;
                    proxy
                        .attach_handles(ws_sender, ws_receiver, cancel_token)
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!("Handshake failed for {}: {}", addr.ip(), e);
                let _ = ws_sender.close().await;
            }
        }
        tracing::info!("Connection closed for {}", addr.ip());
        return;
    }

    /// Performs the initial handshake with a WebSocket client.
    ///
    /// Sends a greeting message describing the expected format, then waits
    /// up to 10 seconds for the client to respond with a JSON handshake
    /// specifying the target protocol, IP, and port.
    ///
    /// Returns the parsed [`Protocol`] on success, or an error if the
    /// handshake times out, contains invalid data, or fails.
    async fn handle_handshake(
        ws_sender: &mut WebSocketSender,
        ws_receiver: &mut WebSocketReceiver,
        timeout_seconds: &u64,
    ) -> Result<Protocol, Box<dyn std::error::Error + Send + Sync>> {
        let _ = ws_sender.send(WebSocket::get_greeting()).await;

        let msg = timeout(
            Duration::from_secs(timeout_seconds.clone()),
            ws_receiver.next(),
        )
        .await
        .map_err(|_| "Handshake timed out")?;

        if let Some(inc_msg) = msg {
            match inc_msg {
                Ok(Message::Text(json_str)) => {
                    return WebSocket::read_greeting(&json_str);
                }
                Err(e) => return Err(Box::new(e)), // propagate error
                _ => return Err("Received unexpected, closing connection!".into()), // propagate as custom error
            }
        } else {
            return Err("No handshake was received".into());
        }
    }

    /// Builds the greeting message sent to clients upon connection.
    ///
    /// The greeting is a JSON message that describes the expected handshake
    /// format, including the protocol, target IP, and target port fields.
    fn get_greeting() -> Message {
        let greeting = serde_json::json!({
            "supported protocols": {
                TcpConnection::schema().name: TcpConnection::schema().requirements
            }
        });
        return Message::Text(greeting.to_string());
    }

    /// Parses a client's handshake JSON and returns the corresponding [`Protocol`].
    ///
    /// Expects a JSON object with `protocol`, `target_ip`, and `target_port` fields.
    /// Currently only the `"tcp"` protocol is supported.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed, required fields are missing,
    /// or the protocol is unsupported.
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

                // if target_ip.starts_with("127.") || target_ip == "localhost" {
                //     return Err("Proxying to loopback is not allowed".into())
                // }

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

    fn connection_cleanup<T>(&self, client: &ClientConnection<T>) -> bool {
        if client.conn.is_finished() {
            return false;
        }
        if self.max_lifetime > 0 && client.start.elapsed().as_secs() > self.max_lifetime {
            tracing::info!(
                "Closing connection '{}' that exceeded max lifetime",
                client.addr
            );
            client.cancel_token.cancel();
            return false;
        }
        true
    }

    /// Sends a success status message to the WebSocket client indicating
    /// the proxy connection has been established.
    async fn proxy_success(ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Successfully connected, starting to proxy",
        });
        let _ = ws_sender.send(Message::text(json_str.to_string())).await;
    }

    /// Sends a failure status message to the WebSocket client with the
    /// reason the proxy connection could not be established.
    async fn proxy_failed(reason: &str, ws_sender: &mut WebSocketSender) {
        let json_str = serde_json::json!({
            "status": "Connection failed, cancelling proxy",
            "reason": reason
        });
        let _ = ws_sender.send(Message::Text(json_str.to_string())).await;
    }
}
