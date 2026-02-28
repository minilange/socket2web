mod protocols;
mod web_socket;
mod arguments;

use crate::web_socket::WebSocket;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config: arguments::WebSocketArguments = argh::from_env();

    let ws = WebSocket::new(&config);

    let _ = ws.start_listening().await;

    return Ok(());
}