// use std::{io::{Read, Write}, net::TcpStream, net::TcpListener};

// fn test_connect() {
//     if let Ok(mut stream) = TcpStream::connect("127.0.0.1:7878") {
//         println!("Connected to socket");
//         let mut buffer = [0; 512];
//         let _ = stream.write(&[1]);
//     } else {
//         println!("Didn't connect to socket");
//     }
// }

// fn main() {
//     println!("Connecting to socket");
//     test_connect();
// }

// use core::error;
// use std::collections::HashMap;
// use std::{net::SocketAddr, sync::Arc};

// use tokio::net::TcpStream;
// use tokio::{net::TcpListener, sync::RwLock};
// use tokio_tungstenite::accept_async;
// use tokio_tungstenite::tungstenite::Message;

mod web_socket;
mod protocols;

use crate::web_socket::WebSocket;
// use crate::protocols::Protocol;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let listen_addr = "127.0.0.1:8080";
    let ws = WebSocket::new(listen_addr.to_string());

    let _ = ws.start_listening().await;

    return Ok(());
}

// use tokio_tungstenite::{connect_async, tungstenite::Message};
// use futures_util::{SinkExt, StreamExt};
// use tracing::info;

// use tokio_tungstenite::{connect_async, tungstenite::Message};
// use futures_util::{SinkExt, StreamExt};
// use std::time::Instant;
// use tracing::info;

// #[tokio::main]
// async fn res() -> Result<(), Box<dyn std::error::Error>> {
//     tracing_subscriber::fmt::init();

//     let url = "ws://127.0.0.1:8080";
//     let (ws_stream, _) = connect_async(url).await?;
//     info!("Connected to WebSocket server");

//     let (mut write, mut read) = ws_stream.split();

//     // Spawn a task to handle incoming messages
//     let read_handle = tokio::spawn(async move {
//         let mut time = Instant::now();
//         let mut received_data = false;
//         let mut counter: u128 = 0;
//         let mut print_counter = 0 ;

//         while let Some(msg) = read.next().await {
//             match msg {
//                 Ok(Message::Text(text)) => {
//                     info!("Received: {}", text);
//                 }
//                 Ok(Message::Binary(mut bin)) => {
//                     if !received_data {
//                         received_data = true;
//                         time = Instant::now();
//                     }
//                     counter += bin.len() as u128;
                    
//                     print_counter += 1;

//                     if print_counter % 10 == 0 {
//                         print_counter = 0;
//                         info!("Received {} bytes - avg. {} bps", &counter, &(counter / time.elapsed().as_millis()))
//                     }

//                     // info!("Received binary: {} bytes", bin.len());
//                 }
//                 Ok(Message::Close(_)) => {
//                     info!("Server closed connection");
//                     break;
//                 }
//                 _ => {}
//             }
//         }
//     });

//     // Read greeting
    
//     // Respond to greeting
//     let json_msg = serde_json::json!({
//         "protocol": "tcp",
//         "target_ip": "127.0.0.1",
//         "target_port": 7878
//     });
//     write.send(Message::Text(json_msg.to_string())).await?;
    
//     // Keep connection alive
//     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
//     write.send(Message::Text("This is a string that's being proxied to".to_string())).await;
    
//     // let mut buffer = [0; 1024];

//     tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    
//     read_handle.await?;
//     // Close connection
//     // write.send(Message::Close(None)).await?;

//     Ok(())
// }