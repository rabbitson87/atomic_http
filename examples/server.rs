#[cfg(feature = "arena")]
use atomic_http::SendableError;
use atomic_http::*;

#[cfg(feature = "arena")]
async fn run_arena_server(port: u16) -> Result<(), SendableError> {
    use atomic_http::Server;
    use http::StatusCode;

    println!("🚀 Arena HTTP 서버 시작 중... (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
    println!("✅ Arena HTTP 서버가 포트 {}에서 실행 중입니다!", port);
    println!("📊 테스트 준비 완료. 다른 터미널에서 load_test_client를 실행하세요.");
    println!("⚡ 중단하려면 Ctrl+C를 누르세요.\n");

    loop {
        let (stream, options, herd) = server.accept().await?;

        tokio::spawn(async move {
            match Server::parse_request_arena_writer(stream, options, herd).await {
                Ok((request, mut response)) => {
                    match request.get_json_arena::<TestData>() {
                        Ok(data) => {
                            let response_data = serde_json::json!({
                                "status": "success",
                                "received_id": data.id,
                                "data_size": data.description.len() + data.payload.len(),
                                "tags_count": data.tags.len(),
                                "metadata_count": data.metadata.len(),
                                "server_type": "arena"
                            });
                            if let Err(e) = response.body_mut().set_arena_json(&response_data) {
                                eprintln!("Arena JSON 설정 실패: {}", e);
                                *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            } else {
                                *response.status_mut() = StatusCode::OK;
                            }
                        }
                        Err(e) => {
                            eprintln!("JSON 파싱 실패: {}", e);
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    if let Err(e) = response.responser_arena().await {
                        eprintln!("응답 전송 실패: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("요청 파싱 실패: {}", e);
                }
            }
        });
    }
}

// 표준 서버
#[cfg(not(feature = "arena"))]
async fn run_standard_server(port: u16) -> Result<(), SendableError> {
    use http::StatusCode;

    println!("🚀 표준 HTTP 서버 시작 중... (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
    println!("✅ 표준 HTTP 서버가 포트 {}에서 실행 중입니다!", port);
    println!("📊 테스트 준비 완료. 다른 터미널에서 load_test_client를 실행하세요.");
    println!("⚡ 중단하려면 Ctrl+C를 누르세요.\n");

    loop {
        let (stream, options) = server.accept().await?;

        tokio::spawn(async move {
            match Server::parse_request(stream, options).await {
                Ok((mut request, mut response)) => {
                    match request.get_json::<TestData>() {
                        Ok(data) => {
                            let response_data = serde_json::json!({
                                "status": "success",
                                "received_id": data.id,
                                "data_size": data.description.len() + data.payload.len(),
                                "tags_count": data.tags.len(),
                                "metadata_count": data.metadata.len(),
                                "server_type": "standard"
                            });
                            response.body_mut().body = response_data.to_string();
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(e) => {
                            eprintln!("JSON 파싱 실패: {}", e);
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    if let Err(e) = response.responser().await {
                        eprintln!("응답 전송 실패: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("요청 파싱 실패: {}", e);
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let args: Vec<String> = std::env::args().collect();
    let port = if args.len() > 1 {
        args[1].parse().unwrap_or(9080)
    } else {
        9080
    };

    #[cfg(feature = "arena")]
    {
        println!("🏗️  Arena 피쳐가 활성화되었습니다.");
        run_arena_server(port).await
    }

    #[cfg(not(feature = "arena"))]
    {
        println!("📝 표준 HTTP 모드입니다.");
        run_standard_server(port).await
    }
}
