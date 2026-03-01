use argh::FromArgs;

#[derive(FromArgs)]
/// Socket2Web - A WebSocket to TCP proxy server.
///
/// Accepts incoming WebSocket connections and proxies them to
/// traditional TCP socket endpoints based on a client handshake.
pub struct WebSocketArguments {
    /// the IP address the WebSocket server will bind to (default: 127.0.0.1)
    #[argh(option, default = "String::from(\"127.0.0.1\")")]
    pub ip: String,

    /// the port the WebSocket server will listen on (default: 433)
    #[argh(option, default = "433")]
    pub port: u16,

    /// the log-level for what's being logged (default: info)
    #[argh(option, default = "String::from(\"info\")")]
    pub log_level: String,

    /// the timeout for establishing a greeting, in seconds (default: 10)
    #[argh(option, default = "10")]
    pub timeout: u64,

    /// the number of concurrent connections maximum allowed (default: 1000)
    #[argh(option, default = "1000")]
    pub max_connections: u64,

    /// the maxmimum lifetime of a single connection in seconds (default: 3600)
    /// if max_lifetime is set to 0, it's ignored
    #[argh(option, default = "3600")]
    pub max_lifetime: u64
}
