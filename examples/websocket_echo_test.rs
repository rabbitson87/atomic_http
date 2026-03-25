use atomic_http::*;
use futures::stream::StreamExt;
use futures::SinkExt;
use http::StatusCode;

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9080);

    println!("WebSocket Echo Server on port {}", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

    loop {
        let accept = server.accept().await?;

        tokio::spawn(async move {
            match accept.stream_parse().await {
                Ok(StreamResult::WebSocket(ws_stream, request, peer)) => {
                    println!(
                        "WebSocket connected: {} (path: {}, peer: {})",
                        request.uri(),
                        request.uri().path(),
                        peer
                    );

                    // Route by path
                    match request.uri().path() {
                        "/echo" | "/" => {
                            let (mut write, mut read) = ws_stream.split();
                            while let Some(Ok(msg)) = read.next().await {
                                if msg.is_text() || msg.is_binary() {
                                    if write.send(msg).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            println!("Unknown WebSocket path: {}", request.uri().path());
                        }
                    }
                    println!("WebSocket disconnected: {}", peer);
                }
                Ok(StreamResult::Http(_request, mut response)) => {
                    response.body_mut().body = "Hello from HTTP".to_string();
                    *response.status_mut() = StatusCode::OK;
                    let _ = response.responser().await;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                }
            }
        });
    }
}
