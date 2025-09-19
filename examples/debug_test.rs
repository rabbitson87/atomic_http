// examples/debug_test.rs
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

// 간단한 디버그 서버
async fn run_debug_server(port: u16, server_ready: Arc<Notify>) -> Result<(), SendableError> {
    println!("🚀 디버그 서버 시작 (포트: {})", port);

    #[cfg(feature = "arena")]
    {
        println!("🏗️ Arena 모드로 실행");
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();

        loop {
            match server.accept().await {
                Ok(accept) => {
                    tokio::spawn(async move {
                        match accept.parse_request_arena_writer().await {
                            Ok((request, mut response)) => {
                                let method = request.method().clone();
                                let path = request.uri().path().to_string();
                                println!("📨 요청 수신: {} {}", method, path);

                                // HTTP 응답 라인과 헤더 구성
                                let json_response = json!({
                                    "server": "arena_debug",
                                    "method": method.to_string(),
                                    "path": path,
                                    "status": "success",
                                    "timestamp": chrono::Utc::now().to_rfc3339()
                                });

                                let json_str = serde_json::to_string(&json_response).unwrap();

                                // 명시적으로 헤더 설정
                                response
                                    .headers_mut()
                                    .insert("Content-Type", "application/json".parse().unwrap());
                                response.headers_mut().insert(
                                    "Content-Length",
                                    json_str.len().to_string().parse().unwrap(),
                                );
                                response.headers_mut().insert(
                                    "Connection",
                                    "close".parse().unwrap(), // 연결 명시적 종료
                                );

                                response.body_mut().set_arena_response(&json_str).unwrap();
                                *response.status_mut() = StatusCode::OK;

                                if let Err(e) = response.responser_arena().await {
                                    println!("❌ 응답 전송 실패: {}", e);
                                } else {
                                    println!("✅ 응답 전송 성공: {} bytes", json_str.len());
                                }
                            }
                            Err(e) => {
                                println!("❌ 요청 파싱 실패: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    println!("❌ 연결 수락 실패: {}", e);
                }
            }
        }
    }

    #[cfg(not(feature = "arena"))]
    {
        println!("📝 표준 모드로 실행");
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();

        loop {
            match server.accept().await {
                Ok(accpet) => {
                    tokio::spawn(async move {
                        match accpet.parse_request().await {
                            Ok((request, mut response)) => {
                                let method = request.method().clone();
                                let path = request.uri().path().to_string();
                                println!("📨 요청 수신: {} {}", method, path);

                                let json_response = json!({
                                    "server": "standard_debug",
                                    "method": method.to_string(),
                                    "path": path,
                                    "status": "success",
                                    "timestamp": chrono::Utc::now().to_rfc3339()
                                });

                                let json_str = json_response.to_string();

                                // 명시적으로 헤더 설정
                                response
                                    .headers_mut()
                                    .insert("Content-Type", "application/json".parse().unwrap());
                                response.headers_mut().insert(
                                    "Content-Length",
                                    json_str.len().to_string().parse().unwrap(),
                                );
                                response
                                    .headers_mut()
                                    .insert("Connection", "close".parse().unwrap());

                                response.body_mut().body = json_str.clone();
                                *response.status_mut() = StatusCode::OK;

                                if let Err(e) = response.responser().await {
                                    println!("❌ 응답 전송 실패: {}", e);
                                } else {
                                    println!("✅ 응답 전송 성공: {} bytes", json_str.len());
                                }
                            }
                            Err(e) => {
                                println!("❌ 요청 파싱 실패: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    println!("❌ 연결 수락 실패: {}", e);
                }
            }
        }
    }
}

// 간단한 클라이언트 테스트
async fn test_debug_client(port: u16) -> Result<(), SendableError> {
    println!("🧪 디버그 클라이언트 시작");

    // 서버 안정화 대기
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("debug-client/1.0")
        .build()?;

    let base_url = format!("http://127.0.0.1:{}", port);

    // 기본 연결 테스트
    println!("\n1️⃣ 기본 연결 테스트");
    let start = Instant::now();

    match client.get(&base_url).send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.text().await?;
            let duration = start.elapsed();

            println!("✅ 연결 성공!");
            println!("   상태: {}", status);
            println!("   소요시간: {:.1}ms", duration.as_millis());
            println!("   Content-Length: {:?}", headers.get("content-length"));
            println!("   Content-Type: {:?}", headers.get("content-type"));
            println!("   응답 본문: {}", body);
        }
        Err(e) => {
            println!("❌ 연결 실패: {}", e);
            println!("   오류 종류: {:?}", e.to_string());

            // 상세 오류 분석
            if e.to_string().contains("Connection refused") {
                println!("   → 서버가 해당 포트에서 실행되지 않음");
            } else if e.to_string().contains("timeout") {
                println!("   → 연결 타임아웃 발생");
            } else if e.to_string().contains("error sending request") {
                println!("   → HTTP 요청 전송 중 오류");
            }

            return Err(e.into());
        }
    }

    // POST 테스트
    println!("\n2️⃣ POST 테스트");
    let test_data = json!({
        "test": "debug_post",
        "size": 1024,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    let start = Instant::now();
    match client
        .post(&base_url)
        .header("Content-Type", "application/json")
        .json(&test_data)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await?;
            let duration = start.elapsed();

            println!("✅ POST 요청 성공!");
            println!("   상태: {}", status);
            println!("   소요시간: {:.1}ms", duration.as_millis());
            println!("   응답 본문: {}", body);
        }
        Err(e) => {
            println!("❌ POST 요청 실패: {}", e);
        }
    }

    println!("\n✅ 디버그 테스트 완료");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9999);

    println!("🐛 HTTP 서버 디버그 테스트");
    println!("포트: {}", port);

    #[cfg(feature = "arena")]
    println!("모드: Arena");

    #[cfg(not(feature = "arena"))]
    println!("모드: 표준");

    // 서버 시작
    let server_ready = Arc::new(Notify::new());
    let ready_clone = server_ready.clone();

    let server_handle = tokio::spawn(async move {
        if let Err(e) = run_debug_server(port, ready_clone).await {
            eprintln!("서버 오류: {}", e);
        }
    });

    // 서버 준비 대기
    println!("⏳ 서버 시작 대기...");
    server_ready.notified().await;
    println!("✅ 서버 준비 완료!");

    // 클라이언트 테스트 실행
    if let Err(e) = test_debug_client(port).await {
        println!("❌ 클라이언트 테스트 실패: {}", e);
    }

    println!("\n🛑 서버 종료 중...");
    server_handle.abort();

    Ok(())
}
