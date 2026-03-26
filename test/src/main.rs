use atomic_http::{
    external::dotenv::dotenv, router::Router, ArenaBody, ArenaWriter, ResponseUtilArena,
    SendableError, Server, StreamResultArena,
};
use futures::{SinkExt, StreamExt};
use http::{Request, Response, StatusCode};
use tokio::fs::try_exists;

#[derive(Debug)]
enum Route {
    Index,
    WsEcho,
    Static,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let address = format!("0.0.0.0:{}", 9000);

    let router: &'static Router<Route> = Box::leak(Box::new(
        Router::new()
            .get("/", Route::Index)
            .get("/ws", Route::WsEcho)
            .get("/{*path}", Route::Static),
    ));

    let mut server = Server::new(&address).await.unwrap();
    println!("start server on: {}", address);

    loop {
        match server.accept().await {
            Ok(accept) => {
                tokio::spawn(async move {
                    let ip = accept.option.get_request_ip();
                    println!("ip: {:?}", ip);

                    match accept.stream_parse_arena().await {
                        Ok(StreamResultArena::WebSocket(ws_stream, request, peer)) => {
                            println!(
                                "WebSocket connected: {} from {}",
                                request.uri().path(),
                                peer
                            );

                            match router.find(&http::Method::GET, request.uri().path()) {
                                Some(m) => match m.value {
                                    Route::WsEcho => {
                                        let (mut tx, mut rx) = ws_stream.split();
                                        while let Some(Ok(msg)) = rx.next().await {
                                            if msg.is_text() || msg.is_binary() {
                                                if tx.send(msg).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        println!(
                                            "No WebSocket handler for: {}",
                                            request.uri().path()
                                        );
                                    }
                                },
                                None => {
                                    println!("Unknown WebSocket path: {}", request.uri().path());
                                }
                            }

                            println!("WebSocket disconnected: {}", peer);
                        }
                        Ok(StreamResultArena::Http(request, response)) => {
                            www_service(request, response, router)
                                .await
                                .unwrap_or_else(|e| {
                                    println!("an error occurred; error = {:?}", e);
                                });
                        }
                        Err(e) => {
                            println!("failed to parse stream: {e:?}");
                        }
                    }
                });
            }
            Err(e) => {
                println!("failed to accept connection: {e:?}");
                continue;
            }
        };
    }
}

async fn www_service(
    request: Request<ArenaBody>,
    mut response: Response<ArenaWriter>,
    router: &Router<Route>,
) -> Result<(), SendableError> {
    println!("ip: {:?}", request.body().ip);
    println!(
        "request: {:?}\n",
        String::from_utf8_lossy(request.body().get_raw_data())
    );

    match router.find(request.method(), request.uri().path()) {
        Some(m) => match m.value {
            Route::Index => {
                let dir = std::env::current_dir()?;
                let path = dir.join("app/index.html");
                response.body_mut().response_file(path)?;
            }
            Route::Static => {
                let path_str = m.params.get("path").unwrap_or("");
                let decoded: String = urlencoding::decode(path_str)?.into();
                let dir = std::env::current_dir()?;
                let file_path = dir.join(&decoded);

                if try_exists(&file_path).await? {
                    response.body_mut().response_file(file_path)?;
                } else {
                    let fallback = dir.join("app/index.html");
                    response.body_mut().response_file(fallback)?;
                }
            }
            Route::WsEcho => {
                *response.status_mut() = StatusCode::UPGRADE_REQUIRED;
            }
        },
        None => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    }

    response.responser_arena().await?;
    Ok(())
}
