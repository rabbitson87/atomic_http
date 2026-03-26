use atomic_http::router::Router;
use atomic_http::*;
use http::StatusCode;

#[derive(Debug)]
enum Route {
    Home,
    GetUser,
    CreateUser,
    ServeFile,
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9080);

    // Leak into &'static so spawned tasks can reference it without Arc overhead.
    let router: &'static Router<Route> = Box::leak(Box::new(
        Router::new()
            .get("/", Route::Home)
            .get("/users/{id}", Route::GetUser)
            .post("/users", Route::CreateUser)
            .get("/files/{*path}", Route::ServeFile),
    ));

    println!("Router server on http://127.0.0.1:{}", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

    loop {
        let accept = server.accept().await?;

        tokio::spawn(async move {
            match accept.parse_request().await {
                Ok((request, mut response)) => {
                    match router.find(request.method(), request.uri().path()) {
                        Some(m) => match m.value {
                            Route::Home => {
                                response.body_mut().body =
                                    r#"{"message":"Hello from Router!"}"#.into();
                                *response.status_mut() = StatusCode::OK;
                            }
                            Route::GetUser => {
                                let id = m.params.get("id").unwrap_or("unknown");
                                response.body_mut().body = format!(r#"{{"user_id":"{}"}}"#, id);
                                *response.status_mut() = StatusCode::OK;
                            }
                            Route::CreateUser => {
                                response.body_mut().body = r#"{"status":"created"}"#.into();
                                *response.status_mut() = StatusCode::CREATED;
                            }
                            Route::ServeFile => {
                                let path = m.params.get("path").unwrap_or("");
                                response.body_mut().body = format!(r#"{{"file":"{}"}}"#, path);
                                *response.status_mut() = StatusCode::OK;
                            }
                        },
                        None => {
                            response.body_mut().body = r#"{"error":"not found"}"#.into();
                            *response.status_mut() = StatusCode::NOT_FOUND;
                        }
                    }

                    let _ = response.responser().await;
                }
                Err(e) => {
                    eprintln!("Parse error: {}", e);
                }
            }
        });
    }
}
