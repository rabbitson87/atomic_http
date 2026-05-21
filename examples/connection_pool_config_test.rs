// Connection Pool 설정 테스트 - nginx 스타일 설정 옵션
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

async fn run_configurable_server(port: u16, server_ready: Arc<Notify>, keep_alive_enabled: bool) {
    println!(
        "🚀 설정 가능한 서버 시작 (포트: {}, Keep-Alive: {})",
        port, keep_alive_enabled
    );

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();

    // 커스텀 Connection Pool 설정 (nginx 스타일)
    #[cfg(feature = "connection_pool")]
    {
        if keep_alive_enabled {
            let config = ConnectionPoolConfig::new()
                .idle_timeout(60) // 60초 (nginx 기본값: 75)
                .max_connections_per_host(16) // 16개 연결
                .keep_alive(true);
            server.options_mut().set_connection_option(config);
        } else {
            server.options_mut().disable_connection_pool();
        }

        let connection_config = server.options.get_connection_option();
        println!(
            "📊 Connection Pool 설정: enabled={}, timeout={}s, max_connections={}",
            connection_config.enable_keep_alive,
            connection_config.max_idle_time.as_secs(),
            connection_config.max_connections_per_host
        );

        // 서버 재시작 필요 (connection pool 설정 변경을 위해)
        if let Err(e) = server.enable_connection_pool() {
            println!("⚠️  Connection pool 활성화 실패: {}", e);
        }
    }

    server_ready.notify_one();

    let test_data = json!({
        "message": "configurable connection pool test",
        "keep_alive_enabled": keep_alive_enabled,
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

                                match path {
                                    "/shutdown" => {
                                        let info = json!({ "status": "shutdown" });
                                        let _ = response.body_mut().set_arena_json(&info);
                                        *response.status_mut() = StatusCode::OK;
                                        let _ = response.responser_arena().await;
                                        return;
                                    }
                                    _ => {
                                        // 설정에 따른 응답 전송
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

async fn test_client_with_connection_reuse(port: u16, num_requests: usize, name: &str) -> Duration {
    let url = format!("http://127.0.0.1:{}/test", port);

    // Keep-alive 클라이언트 설정
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    println!("📡 {} 테스트 시작: {} 요청", name, num_requests);

    let start = Instant::now();

    for i in 0..num_requests {
        match client.get(&url).send().await {
            Ok(response) => {
                // 응답 헤더 확인
                if i == 0 {
                    if let Some(connection) = response.headers().get("connection") {
                        println!("🔗 {} Connection 헤더: {:?}", name, connection);
                    }
                    if let Some(keep_alive) = response.headers().get("keep-alive") {
                        println!("⏰ {} Keep-Alive 헤더: {:?}", name, keep_alive);
                    }
                }

                let _body = response.text().await.unwrap();
            }
            Err(e) => {
                eprintln!("❌ {} 요청 {} 실패: {}", name, i, e);
            }
        }
    }

    let duration = start.elapsed();
    let rps = num_requests as f64 / duration.as_secs_f64();
    println!(
        "✅ {}: {:.2} req/s ({}ms total)",
        name,
        rps,
        duration.as_millis()
    );

    duration
}

async fn shutdown_server(port: u16) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/shutdown", port);
    let _ = client.get(&url).send().await;
    println!("🛑 서버 {} 종료 신호 전송", port);
}

#[tokio::main]
async fn main() {
    println!("🚀 Connection Pool 설정 테스트 (nginx 스타일)");
    println!("{}", "=".repeat(60));

    let num_requests = 30;

    println!("💡 이 테스트는 Keep-Alive 설정이 실제로 적용되는지 확인합니다.");
    println!("   - Keep-Alive 활성화: Connection: keep-alive, Keep-Alive: timeout=60, max=50");
    println!("   - Keep-Alive 비활성화: Connection: close");
    println!();

    // 테스트 1: Keep-Alive 활성화
    let port1 = 9991;
    let server_ready1 = Arc::new(Notify::new());
    let server_ready1_clone = server_ready1.clone();

    let server1_handle = tokio::spawn(async move {
        run_configurable_server(port1, server_ready1_clone, true).await;
    });

    server_ready1.notified().await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let keep_alive_duration =
        test_client_with_connection_reuse(port1, num_requests, "Keep-Alive 활성화").await;

    shutdown_server(port1).await;
    server1_handle.abort();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 테스트 2: Keep-Alive 비활성화
    let port2 = 9992;
    let server_ready2 = Arc::new(Notify::new());
    let server_ready2_clone = server_ready2.clone();

    let server2_handle = tokio::spawn(async move {
        run_configurable_server(port2, server_ready2_clone, false).await;
    });

    server_ready2.notified().await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let close_duration =
        test_client_with_connection_reuse(port2, num_requests, "Keep-Alive 비활성화").await;

    shutdown_server(port2).await;
    server2_handle.abort();

    // 성능 비교
    let keep_alive_rps = num_requests as f64 / keep_alive_duration.as_secs_f64();
    let close_rps = num_requests as f64 / close_duration.as_secs_f64();
    let improvement = keep_alive_rps / close_rps;

    println!("\n📊 설정별 성능 비교:");
    println!("   Keep-Alive 활성화:   {:.2} req/s", keep_alive_rps);
    println!("   Keep-Alive 비활성화:  {:.2} req/s", close_rps);

    if improvement > 1.0 {
        println!(
            "   🏆 Keep-Alive 개선: {:.2}x 더 빠름 ({:.1}% 향상)",
            improvement,
            (improvement - 1.0) * 100.0
        );
    } else {
        println!("   📊 Connection close가 {:.2}x 더 빠름", 1.0 / improvement);
    }

    println!("\n✅ 설정 테스트 완료!");
    println!("💡 서버 관리자는 Options를 통해 Keep-Alive 동작을 세밀하게 제어할 수 있습니다.");

    #[cfg(feature = "connection_pool")]
    println!("🔧 새로운 설정 방법:");
    #[cfg(feature = "connection_pool")]
    println!("   let config = ConnectionPoolConfig::new()");
    #[cfg(feature = "connection_pool")]
    println!("       .keep_alive(true)");
    #[cfg(feature = "connection_pool")]
    println!("       .idle_timeout(60)");
    #[cfg(feature = "connection_pool")]
    println!("       .max_connections_per_host(16);");
    #[cfg(feature = "connection_pool")]
    println!("   server.options.set_connection_option(config);");
}
