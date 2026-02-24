use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};

use crate::protocols::{ProxyTarget, WebSocketReceiver, WebSocketSender};

pub struct TcpConnection {
    ip_address: String,
    port: u16,
    stream: Option<TcpStream>,
}

impl TcpConnection {
    pub fn new(ip_address: String, port: u16) -> Self {
        Self {
            ip_address: ip_address,
            port: port,
            stream: None,
        }
    }

    async fn proxy_tcp_to_web(mut tcp_reader: OwnedReadHalf, mut ws_sender: WebSocketSender) {
        let mut buffer = [0; 1024];
        loop {
            match tcp_reader.read(&mut buffer).await {
                Ok(0) => {
                    debug!("Received EOF");
                    let _ = ws_sender.close();
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
                Err(_) => {
                    error!("Failed to read from socket");
                    let _ = ws_sender.close();
                    break;
                }
            }
        }
        info!("Ended tcp->web");
    }

    async fn proxy_web_to_tcp(mut ws_receiver: WebSocketReceiver, mut tcp_writer: OwnedWriteHalf) {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let _ = tcp_writer.write(text.as_bytes()).await;
                }
                Ok(Message::Binary(bin)) => {
                    let _ = tcp_writer.write(&bin).await;
                }
                Ok(Message::Pong(_)) | Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => {}
                Ok(Message::Close(_)) => {
                    info!("Websocket was closed by client!");
                    let _ = tcp_writer.shutdown();
                }
                Err(e) => {
                    error!("WebSocket Error: {}", e);
                }
            };
        }
        debug!("Ended web->tcp");
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
        debug!("Attaching handles!");

        tokio::spawn(TcpConnection::proxy_tcp_to_web(tcp_reader, ws_sender));
        let _ = TcpConnection::proxy_web_to_tcp(ws_receiver, tcp_writer).await;
    }
}
