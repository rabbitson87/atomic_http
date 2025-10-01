// 간단한 Connection Pool 테스트
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

async fn run_simple_keep_alive_server(port: u16, server_ready: Arc<Notify>) {
    println!("🚀 간단한 Keep-Alive 테스트 서버 시작 (포트: {})", port);

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
    server_ready.notify_one();
    println!("✅ 서버 시작됨 (Keep-Alive 지원)");

    let test_data = json!({
        "message": "connection pool test response",
        "features": ["arena", "simd", "vectored_io", "connection_pool"],
        "data": "x".repeat(2000),
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

                                // Connection 헤더 확인
                                if let Some(connection) = request.headers().get("connection") {
                                    println!("🔗 클라이언트 Connection 헤더: {:?}", connection);
                                }

                                match path {
                                    "/shutdown" => {
                                        let info = json!({ "status": "shutdown" });
                                        let _ = response.body_mut().set_arena_json(&info);
                                        *response.status_mut() = StatusCode::OK;
                                        let _ = response.responser_arena().await;
                                        return;
                                    }
                                    _ => {
                                        // 테스트 응답 전송 (Keep-alive 자동 설정됨)
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

async fn test_keep_alive_client(port: u16, num_requests: usize) -> Duration {
    let url = format!("http://127.0.0.1:{}/test", port);

    // Keep-alive를 지원하는 클라이언트 생성
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    println!(
        "📡 Keep-Alive 클라이언트 테스트 시작: {} 요청",
        num_requests
    );

    let start = Instant::now();

    for i in 0..num_requests {
        match client.get(&url).send().await {
            Ok(response) => {
                // Connection 헤더 확인
                if let Some(connection) = response.headers().get("connection") {
                    if i == 0 {
                        println!("🔗 서버 Connection 헤더: {:?}", connection);
                    }
                }

                // Keep-Alive 헤더 확인
                if let Some(keep_alive) = response.headers().get("keep-alive") {
                    if i == 0 {
                        println!("⏰ Keep-Alive 헤더: {:?}", keep_alive);
                    }
                }

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
    println!(
        "📊 Keep-Alive 성능: {:.2} req/s ({}ms total)",
        rps,
        duration.as_millis()
    );

    duration
}

async fn test_new_connection_client(port: u16, num_requests: usize) -> Duration {
    let url = format!("http://127.0.0.1:{}/test", port);

    println!("📡 새 연결 클라이언트 테스트 시작: {} 요청", num_requests);

    let start = Instant::now();

    for i in 0..num_requests {
        // 매번 새로운 클라이언트 생성 (연결 재사용 방지)
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(0) // 연결 풀 비활성화
            .build()
            .unwrap();

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
    println!(
        "📊 새 연결 성능: {:.2} req/s ({}ms total)",
        rps,
        duration.as_millis()
    );

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
    println!("🚀 간단한 Connection Pool & Keep-Alive 테스트");
    println!("{}", "=".repeat(60));

    #[cfg(feature = "connection_pool")]
    println!("🔥 Connection Pool 기능이 활성화됨");

    #[cfg(not(feature = "connection_pool"))]
    println!("⚠️  Connection Pool 기능이 비활성화됨");

    let port = 9999;
    let num_requests = 50;

    let server_ready = Arc::new(Notify::new());
    let server_ready_clone = server_ready.clone();

    // 서버 시작
    let server_handle = tokio::spawn(async move {
        run_simple_keep_alive_server(port, server_ready_clone).await;
    });

    // 서버 시작 대기
    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("\n1️⃣ Keep-Alive 연결 테스트:");
    let keep_alive_duration = test_keep_alive_client(port, num_requests).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("\n2️⃣ 새 연결 매번 생성 테스트:");
    let new_connection_duration = test_new_connection_client(port, num_requests).await;

    // 성능 비교
    let keep_alive_rps = num_requests as f64 / keep_alive_duration.as_secs_f64();
    let new_conn_rps = num_requests as f64 / new_connection_duration.as_secs_f64();
    let improvement = keep_alive_rps / new_conn_rps;

    println!("\n📊 성능 비교 결과:");
    println!("   Keep-Alive:   {:.2} req/s", keep_alive_rps);
    println!("   새 연결 생성:  {:.2} req/s", new_conn_rps);

    if improvement > 1.0 {
        println!(
            "   🏆 Keep-Alive 개선: {:.2}x 더 빠름 ({:.1}% 향상)",
            improvement,
            (improvement - 1.0) * 100.0
        );
    } else {
        println!("   📊 새 연결이 {:.2}x 더 빠름", 1.0 / improvement);
    }

    // 서버 종료
    shutdown_server(port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    server_handle.abort();

    println!("\n✅ 테스트 완료!");
    println!("💡 Keep-alive는 연결 설정 오버헤드를 제거하여 성능을 향상시킵니다.");
}
