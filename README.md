# Socket2Web: Bridging Traditional Sockets and WebSockets

## Introduction

Modern web applications increasingly rely on WebSockets for real-time, bidirectional communication. However, many legacy systems and applications still use traditional TCP sockets, creating a gap between new and old communication paradigms. Socket2Web is designed to bridge this gap by providing a transparent proxy that translates between standard socket connections and WebSocket connections.

## Motivation

Most existing WebSocket-to-TCP proxies require a statically configured target — the destination is fixed at startup and every client connects to the same backend. Socket2Web takes a different approach: the target is chosen **dynamically** by the client at connection time. Each WebSocket client specifies the protocol, IP address, and port it wants to reach during the handshake, allowing a single proxy instance to route to any TCP endpoint in real time. Additionally, the architecture is built around a trait-based protocol system, making it straightforward to add support for any other bidirectional communication protocol beyond TCP.

## Architecture Overview

The core of Socket2Web is a proxy server that listens for incoming WebSocket connections. Upon receiving a connection, the server expects the client to specify the target protocol, IP address, and port for the desired socket endpoint. The proxy then establishes a TCP connection to the specified target and transparently relays data between the WebSocket client and the traditional socket server.

**Key Components:**
- **WebSocket Server:** Accepts connections from web clients, enforces connection limits, and handles graceful shutdown.
- **Protocol Handler:** Interprets the initial handshake and target specification from the client. Currently supports TCP, with an extensible trait-based architecture for adding new protocols.
- **Socket Proxy:** Forwards data bidirectionally between the WebSocket and the target TCP socket using cancellation tokens for coordinated shutdown.

## Protocol

Upon connecting to the WebSocket server, the client receives a greeting message describing the supported protocols and their required fields. The client must then respond with a JSON handshake specifying the target protocol, IP, and port.

**Example greeting (sent by server):**
```json
{
  "supported protocols": {
    "tcp": {
      "target_ip": "IP Address",
      "target_port": "Port for IP Address"
    }
  }
}
```

**Example handshake (sent by client):**
```json
{
  "protocol": "tcp",
  "target_ip": "192.168.1.100",
  "target_port": 12345
}
```

After a successful handshake, the server responds with a status message and all subsequent data is proxied transparently.

## Configuration

Socket2Web is configured via command-line arguments:

| Argument | Default | Description |
|----------|---------|-------------|
| `--ip` | `127.0.0.1` | IP address the WebSocket server binds to |
| `--port` | `433` | Port the WebSocket server listens on |
| `--log-level` | `info` | Log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `--timeout` | `10` | Handshake timeout in seconds |
| `--max-connections` | `1000` | Maximum number of concurrent connections |
| `--max-lifetime` | `3600` | Maximum lifetime of a connection in seconds (0 to disable) |

## Use Cases

- **Legacy Application Modernization:** Connect legacy TCP-based applications to web clients without rewriting server code.
- **IoT Device Integration:** Bridge devices that use raw sockets with modern dashboards or control panels.
- **Testing and Debugging:** Intercept and analyze socket traffic using web-based tools.

## Getting Started

1. Build the application using Cargo:
   ```
   cargo build --release
   ```
2. Run the proxy server:
   ```
   cargo run --release
   ```
3. Run with custom options:
   ```
   cargo run --release -- --ip 0.0.0.0 --port 8080 --log-level debug --max-connections 500
   ```
4. Connect a WebSocket client, receive the greeting, send the handshake JSON, and begin proxying data.

## Conclusion

Socket2Web provides a simple, robust, and extensible solution for bridging the gap between traditional socket-based systems and modern web applications. Contributions and feedback are welcome!