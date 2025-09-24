// 간단한 vectored I/O 테스트
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

async fn run_simple_server(port: u16, server_ready: Arc<Notify>) {
    println!("🚀 간단한 테스트 서버 시작 (포트: {})", port);

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
    server_ready.notify_one();
    println!("✅ 서버 시작됨");

    let test_data = json!({
        "message": "vectored I/O test response",
        "data": "x".repeat(10000),
        "features": ["arena", "simd", "vectored_io"],
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });

    loop {
        match server.accept().await {
            Ok(accept) => {
                let test_data = test_data.clone();
                tokio::spawn(async move {
                    #[cfg(feature = "arena")]
                    {
                        match accept.parse_request_arena_writer().await {
                            Ok((request, mut response)) => {
                                let path = request.uri().path();
                                println!("📨 요청: {}", path);

                                match path {
                                    "/shutdown" => {
                                        let info = json!({ "status": "shutdown" });
                                        let _ = response.body_mut().set_arena_json(&info);
                                        *response.status_mut() = StatusCode::OK;
                                        let _ = response.responser_arena().await;
                                        return;
                                    }
                                    _ => {
                                        // 테스트 응답 전송
                                        let _ = response.body_mut().set_arena_json(&test_data);
                                        *response.status_mut() = StatusCode::OK;
                                        let _ = response.responser_arena().await;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("❌ 요청 파싱 실패: {}", e);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("❌ Accept 실패: {}", e);
                break;
            }
        }
    }
}

async fn test_client(port: u16, num_requests: usize) -> Duration {
    let url = format!("http://127.0.0.1:{}/test", port);
    let client = reqwest::Client::new();

    println!("📡 클라이언트 테스트 시작: {} 요청", num_requests);

    let start = Instant::now();

    for i in 0..num_requests {
        match client.get(&url).send().await {
            Ok(response) => {
                let body = response.text().await.unwrap();
                if i == 0 {
                    println!("✅ 첫 번째 응답 크기: {} bytes", body.len());
                }
            }
            Err(e) => {
                eprintln!("❌ 요청 {} 실패: {}", i, e);
            }
        }
    }

    let duration = start.elapsed();
    let rps = num_requests as f64 / duration.as_secs_f64();
    println!("📊 성능: {:.2} req/s ({}ms total)", rps, duration.as_millis());

    duration
}

async fn shutdown_server(port: u16) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/shutdown", port);
    let _ = client.get(&url).send().await;
    println!("🛑 서버 종료 신호 전송");
}

#[tokio::main]
async fn main() {
    println!("🚀 간단한 Vectored I/O 테스트");
    println!("{}", "=".repeat(50));

    #[cfg(feature = "vectored_io")]
    println!("🔥 Vectored I/O 기능이 활성화됨");

    #[cfg(not(feature = "vectored_io"))]
    println!("⚠️  Vectored I/O 기능이 비활성화됨");

    #[cfg(feature = "simd")]
    println!("🔥 SIMD 기능이 활성화됨");

    #[cfg(feature = "arena")]
    println!("🔥 Arena 기능이 활성화됨");

    let port = 9999;
    let num_requests = 100;

    let server_ready = Arc::new(Notify::new());
    let server_ready_clone = server_ready.clone();

    // 서버 시작
    let server_handle = tokio::spawn(async move {
        run_simple_server(port, server_ready_clone).await;
    });

    // 서버 시작 대기
    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 테스트 실행
    let duration = test_client(port, num_requests).await;

    // 서버 종료
    shutdown_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    server_handle.abort();

    println!("\n✅ 테스트 완료!");
    println!("📈 최종 결과: {}ms (평균 {:.2}ms/req)",
             duration.as_millis(),
             duration.as_millis() as f64 / num_requests as f64);
}