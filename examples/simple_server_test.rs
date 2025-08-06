// 간단한 서버 테스트 (기존 server.rs를 간소화)
use atomic_http::*;
use http::StatusCode;

#[cfg(feature = "arena")]
async fn run_arena_server(port: u16) -> Result<(), SendableError> {
    println!("🏗️ 간단한 Arena 서버 (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

    loop {
        let (stream, options, herd) = server.accept().await?;

        tokio::spawn(async move {
            match Server::parse_request_arena_writer(stream, options, herd).await {
                Ok((request, mut response)) => {
                    match request.get_json_arena::<TestData>() {
                        Ok(data) => {
                            let response_data = serde_json::json!({
                                "status": "success",
                                "server_type": "arena",
                                "data_id": data.id,
                                "data_size": data.payload.len()
                            });
                            let _ = response.body_mut().set_arena_json(&response_data);
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(_) => {
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    let _ = response.responser_arena().await;
                }
                Err(_) => {}
            }
        });
    }
}

#[cfg(not(feature = "arena"))]
async fn run_standard_server(port: u16) -> Result<(), SendableError> {
    println!("📝 간단한 표준 서버 (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

    loop {
        let (stream, options) = server.accept().await?;

        tokio::spawn(async move {
            match Server::parse_request(stream, options).await {
                Ok((mut request, mut response)) => {
                    match request.get_json::<TestData>() {
                        Ok(data) => {
                            let response_data = serde_json::json!({
                                "status": "success",
                                "server_type": "standard",
                                "data_id": data.id,
                                "data_size": data.payload.len()
                            });
                            response.body_mut().body = response_data.to_string();
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(_) => {
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    let _ = response.responser().await;
                }
                Err(_) => {}
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9080);

    println!("🚀 간단한 HTTP 서버 테스트");

    #[cfg(feature = "arena")]
    {
        println!("모드: Arena");
        run_arena_server(port).await
    }

    #[cfg(not(feature = "arena"))]
    {
        println!("모드: 표준");
        run_standard_server(port).await
    }
}
