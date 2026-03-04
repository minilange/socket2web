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
    async fn attach_handles(
        &mut self,
        ws_sender: WebSocketSender,
        ws_receiver: WebSocketReceiver,
        cancel_token: CancellationToken,
    ) {
        let (tcp_reader, tcp_writer) = match self.stream.take() {
            Some(stream) => stream.into_split(),
            None => {
                tracing::error!("Tried to attach handle with no connected stream");
                return;
            }
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    /// Creates a connected WebSocket pair (client-side stream, server-side sender+receiver)
    /// and a connected TCP pair (target-side stream, proxy-side reader+writer).
    ///
    /// Returns:
    /// - `client_ws`: the WebSocket stream from the client's perspective
    /// - `ws_sender` / `ws_receiver`: the server-side split WebSocket (what the proxy uses)
    /// - `target_stream`: the TCP stream from the target's perspective
    /// - `tcp_reader` / `tcp_writer`: the proxy-side split TCP halves
    async fn setup_proxy_pair() -> (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WebSocketSender,
        WebSocketReceiver,
        tokio::net::TcpStream,
        OwnedReadHalf,
        OwnedWriteHalf,
    ) {
        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let ws_accept = tokio::spawn(async move {
            let (stream, _) = ws_listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });

        let (client_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ws_addr.port()))
                .await
                .unwrap();

        let server_ws = ws_accept.await.unwrap();
        let (ws_sender, ws_receiver) = server_ws.split();

        let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_addr = tcp_listener.local_addr().unwrap();

        let tcp_accept = tokio::spawn(async move {
            let (stream, _) = tcp_listener.accept().await.unwrap();
            stream
        });

        let proxy_tcp = tokio::net::TcpStream::connect(tcp_addr).await.unwrap();
        let target_stream = tcp_accept.await.unwrap();
        let (tcp_reader, tcp_writer) = proxy_tcp.into_split();

        (
            client_ws,
            ws_sender,
            ws_receiver,
            target_stream,
            tcp_reader,
            tcp_writer,
        )
    }

    #[test]
    fn tcp_connection_new_does_not_init_stream() {
        let conn = TcpConnection::new("ip".into(), 1234);
        assert_eq!(conn.ip_address, "ip");
        assert_eq!(conn.port, 1234);
        assert!(conn.stream.is_none())
    }

    #[tokio::test]
    async fn tcp_connection_connects_to_stream() {
        let target_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_server.local_addr().unwrap();

        let accept_handle = tokio::spawn(async move { target_server.accept().await });

        let mut conn = TcpConnection::new("127.0.0.1".into(), target_addr.port());
        let result = conn.connect().await;

        assert!(result.is_ok(), "connect() should succeed");
        assert!(conn.stream.is_some(), "stream should be set after connect");

        let accepted = accept_handle.await.unwrap();
        assert!(
            accepted.is_ok(),
            "listener should have accepted a connection"
        );
    }

    #[tokio::test]
    async fn tcp_connection_fails_with_no_listener() {
        let mut conn = TcpConnection::new("127.0.0.1".into(), 1);
        let result = conn.connect().await;

        assert!(result.is_err(), "connect() should fail with no listener");
        assert!(
            conn.stream.is_none(),
            "stream should remain None on failure"
        );
    }

    #[tokio::test]
    async fn tcp_connection_attaches_handles() {
        let target_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_server.local_addr().unwrap();

        let echo_handle = tokio::spawn(async move {
            let (mut stream, _) = target_server.accept().await.unwrap();
            let mut buf = vec![0, 255];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                stream.write_all(&buf[..n]).await.unwrap();
            }
        });

        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let cancel_token = CancellationToken::new();
        let server_token = cancel_token.clone();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = ws_listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();

            let (ws_sender, ws_receiver) = futures_util::StreamExt::split(ws_stream);

            let mut conn = TcpConnection::new("127.0.0.1".into(), target_addr.port());
            conn.connect().await.unwrap();
            conn.attach_handles(ws_sender, ws_receiver, server_token)
                .await;
        });

        let (mut client_stream, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ws_addr.port()))
                .await
                .unwrap();

        client_stream
            .send(Message::binary(b"Greetings target!".to_vec()))
            .await
            .unwrap();

        let response =
            tokio::time::timeout(std::time::Duration::from_secs(1), client_stream.next())
                .await
                .expect("Timed out waiting for echo response")
                .expect("Stream ended unexpectedly")
                .expect("Error reading message");

        match response {
            Message::Binary(data) => {
                assert_eq!(data, b"Greetings target!", "Data from target should match")
            }
            other => panic!("Expected Binary message, got {:?}", other),
        }

        client_stream.close(None).await.unwrap();
        cancel_token.cancel();

        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), server_handle).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), echo_handle).await;
    }

    #[tokio::test]
    async fn tcp_connection_attach_handles_with_not_connected_stream() {
        let target_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_server.local_addr().unwrap();

        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let cancel_token = CancellationToken::new();
        let server_token = cancel_token.clone();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = ws_listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (ws_sender, ws_receiver) = futures_util::StreamExt::split(ws_stream);

            let mut conn = TcpConnection::new("127.0.0.1".into(), target_addr.port());
            conn.attach_handles(ws_sender, ws_receiver, server_token)
                .await;
        });

        let (mut client_stream, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ws_addr.port()))
                .await
                .unwrap();

        client_stream
            .send(Message::binary(b"this should never arrive".to_vec()))
            .await
            .unwrap();

        let server_result =
            tokio::time::timeout(std::time::Duration::from_secs(1), server_handle).await;
        assert!(
            server_result.is_ok(),
            "Server should exit quickly when stream is None"
        );

        let target_accept = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            target_server.accept(),
        )
        .await;
        assert!(
            target_accept.is_err(),
            "Target should never receive a connection"
        );

        cancel_token.cancel();
    }

    #[tokio::test]
    async fn proxy_web_to_tcp_forwards_binary_data() {
        let (mut client_ws, _ws_sender, ws_receiver, mut target_stream, _tcp_reader, tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, proxy_token).await;
        });

        client_ws
            .send(Message::Binary(b"hello tcp".to_vec()))
            .await
            .unwrap();

        let mut buf = vec![0u8; 1024];
        let n = tokio::time::timeout(std::time::Duration::from_secs(1), target_stream.read(&mut buf))
            .await
            .expect("Timed out waiting for data on target")
            .expect("Failed to read from target");

        assert_eq!(&buf[..n], b"hello tcp");

        token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle).await;
    }

    #[tokio::test]
    async fn proxy_web_to_tcp_forwards_text_data() {
        let (mut client_ws, _ws_sender, ws_receiver, mut target_stream, _tcp_reader, tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, proxy_token).await;
        });

        client_ws
            .send(Message::Text("hello as text".into()))
            .await
            .unwrap();

        let mut buf = vec![0u8; 1024];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), target_stream.read(&mut buf))
            .await
            .expect("Timed out waiting for data on target")
            .expect("Failed to read from target");

        assert_eq!(&buf[..n], b"hello as text");

        token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle).await;
    }

    #[tokio::test]
    async fn proxy_web_to_tcp_cancels_token_on_ws_close() {
        let (mut client_ws, _ws_sender, ws_receiver, _target_stream, _tcp_reader, tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, proxy_token).await;
        });

        client_ws.close(None).await.unwrap();

        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle)
            .await
            .expect("proxy_web_to_tcp should exit after WS close");

        assert!(
            token.is_cancelled(),
            "Token should be cancelled after WS close"
        );
    }

    #[tokio::test]
    async fn proxy_web_to_tcp_stops_on_token_cancel() {
        let (_client_ws, _ws_sender, ws_receiver, _target_stream, _tcp_reader, tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, proxy_token).await;
        });

        token.cancel();

        let result = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle).await;
        assert!(
            result.is_ok(),
            "proxy_web_to_tcp should exit after token cancel"
        );
    }

    #[tokio::test]
    async fn proxy_tcp_to_web_forwards_data() {
        let (mut client_ws, ws_sender, _ws_receiver, mut target_stream, tcp_reader, _tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_tcp_to_web(tcp_reader, ws_sender, proxy_token).await;
        });

        target_stream.write_all(b"hello websocket").await.unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), client_ws.next())
            .await
            .expect("Timed out waiting for WS message")
            .expect("Stream ended unexpectedly")
            .expect("Error reading message");

        match msg {
            Message::Binary(data) => assert_eq!(data, b"hello websocket"),
            other => panic!("Expected Binary message, got {:?}", other),
        }

        token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle).await;
    }

    #[tokio::test]
    async fn proxy_tcp_to_web_cancels_token_on_eof() {
        let (_client_ws, ws_sender, _ws_receiver, target_stream, tcp_reader, _tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_tcp_to_web(tcp_reader, ws_sender, proxy_token).await;
        });

        drop(target_stream);

        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle)
            .await
            .expect("proxy_tcp_to_web should exit after TCP EOF");

        assert!(
            token.is_cancelled(),
            "Token should be cancelled after TCP EOF"
        );
    }

    #[tokio::test]
    async fn proxy_tcp_to_web_stops_on_token_cancel() {
        let (_client_ws, ws_sender, _ws_receiver, _target_stream, tcp_reader, _tcp_writer) =
            setup_proxy_pair().await;

        let token = CancellationToken::new();
        let proxy_token = token.clone();

        let proxy_handle = tokio::spawn(async move {
            TcpConnection::proxy_tcp_to_web(tcp_reader, ws_sender, proxy_token).await;
        });

        // Cancel externally
        token.cancel();

        // Proxy should exit promptly
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), proxy_handle).await;
        assert!(
            result.is_ok(),
            "proxy_tcp_to_web should exit after token cancel"
        );
    }

    #[test]
    fn tcp_schema_returns_ip_and_port() {
        let tcp_schema: Schema = TcpConnection::schema();
        assert_eq!(tcp_schema.name, "tcp");

        let reqs = tcp_schema.requirements.as_object().unwrap();
        assert_eq!(reqs.keys().len(), 2);
        assert!(
            reqs.keys().any(|k| k == "target_ip"),
            "Tcp protocol requirements should contain 'taret_ip'"
        );
        assert!(
            reqs.keys().any(|k| k == "target_port"),
            "Tcp protocol requirements should contain 'taret_port'"
        );
    }
}
