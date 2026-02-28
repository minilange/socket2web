use argh::FromArgs;

#[derive(FromArgs)]
/// Arguments that's used to configure the Socket2Web proxy
pub struct WebSocketArguments {
    /// the ip address to listen on             (default 127.0.0.1)
    #[argh(option, default = "String::from(\"127.0.0.1\")")]
    pub ip: String,

    /// the port to listen on                   (default: 433)
    #[argh(option, default = "8080")]
    pub port: u16,
}
