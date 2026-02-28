use std::io;

use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info};

use crate::protocols::{ProxyTarget, WebSocketReceiver, WebSocketSender};

pub struct TcpConnection {
    ip_address: String,
    port: u16,
    stream: Option<TcpStream>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl TcpConnection {
    pub fn new(ip_address: String, port: u16) -> Self {
        Self {
            ip_address: ip_address,
            port: port,
            stream: None,
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }

    async fn proxy_tcp_to_web(
        mut tcp_reader: OwnedReadHalf,
        mut ws_sender: WebSocketSender,
        token: tokio_util::sync::CancellationToken,
    ) {
        info!("TCP->Web started!");
        let mut buffer = [0; 1024];
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    break;
                }
                rx = tcp_reader.read(&mut buffer) => {
                    match rx {
                        Ok(0) => {
                            info!("Received EOF");
                            break;
                        }
                        Ok(rx_n) => {
                            if let Err(tx_err) = ws_sender
                            .send(Message::Binary(buffer[..rx_n].to_vec()))
                            .await
                            {
                                error!("Encountered an error while transmitting to web: {}", tx_err);
                                break;
                            }
                        }
                        Err(err) => {
                            if err.kind() != io::ErrorKind::WouldBlock {
                                error!("Failed to read from socket");
                                token.cancel();
                                break;
                            }
                        }
                    }
                }
            }
        }

        let _ = ws_sender.close();
        info!("Ended tcp->web");
    }

    async fn proxy_web_to_tcp(
        mut ws_receiver: WebSocketReceiver,
        mut tcp_writer: OwnedWriteHalf,
        token: &tokio_util::sync::CancellationToken,
    ) {
        info!("Web->TCP started!");
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    let _ = tcp_writer.shutdown().await;
                    info!("Web->TCP token was cancelled, shutting down");
                    break;
                }
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let _ = tcp_writer.write(text.as_bytes()).await;
                        }
                        Some(Ok(Message::Binary(bin))) => {
                            let _ = tcp_writer.write(&bin).await;
                        }
                        Some(Ok(Message::Pong(pong))) => {
                            let _ = tcp_writer.write(&pong).await;
                        }
                        Some(Ok(Message::Ping(ping))) => {
                            let _ = tcp_writer.write(&ping).await;
                        }
                        Some(Ok(Message::Frame(frame))) => {
                            let _ = tcp_writer.write(&frame.into_data()).await;
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Websocket was closed by client!");
                            let _ = tcp_writer.shutdown().await;
                            token.cancel();
                            break;
                        }
                        Some(Err(e)) => {
                            error!("Web->TCP Error: {}", e);
                            token.cancel();
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended");
                            let _ = tcp_writer.shutdown().await;
                            token.cancel();
                            break;
                        }
                    }
                }
            }
        }
        info!("Ended web->tcp");
    }
}

impl ProxyTarget for TcpConnection {
    async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stream = TcpStream::connect(format!("{}:{}", &self.ip_address, &self.port))
            .await
            .map_err(|error| format!("Failed to connect: {:?}", error))?;
        self.stream = Some(stream);
        return Ok(());
    }

    async fn attach_handles(&mut self, ws_sender: WebSocketSender, ws_receiver: WebSocketReceiver) {
        let (tcp_reader, tcp_writer) = self
            .stream
            .take()
            .expect("stream not connected")
            .into_split();
        info!("Attaching handles!");

        let cloned_token = self.cancel_token.clone();

        tokio::spawn(TcpConnection::proxy_tcp_to_web(
            tcp_reader,
            ws_sender,
            cloned_token,
        ));
        let _ = TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer, &self.cancel_token).await;
        info!("Done awating proxy_we_to_tcp");
    }
}
