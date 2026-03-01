mod arguments;
mod protocols;
mod web_socket;

use crate::web_socket::WebSocket;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config: arguments::WebSocketArguments = argh::from_env();

    let filter = EnvFilter::try_new(&config.log_level).unwrap_or_else(|_| {
        eprintln!(
            "Invalid log level '{}', defaulting to 'info'",
            config.log_level
        );
        EnvFilter::new("info")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let ws = WebSocket::new(&config);

    if let Err(err) = ws.start_listening().await {
        tracing::error!(
            "An error was encountered while running websocket server: {}",
            err
        );
    }

    return Ok(());
}
