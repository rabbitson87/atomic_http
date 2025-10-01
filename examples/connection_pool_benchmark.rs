// Connection Pool vs 일반 연결 성능 비교 벤치마크
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const TEST_SCENARIOS: &[(usize, &str)] = &[
    (10, "저부하"),
    (50, "중부하"),
    (100, "고부하"),
    (200, "초고부하"),
];
const REQUESTS_PER_CONNECTION: usize = 50;
const TEST_PORT_BASE: u16 = 9990;

async fn run_keep_alive_server(port: u16, server_ready: Arc<Notify>) {
    println!("🚀 Keep-Alive 테스트 서버 시작 (포트: {})", port);

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
    server_ready.notify_one();

    let test_response = json!({
        "message": "connection pool benchmark response",
        "features": ["keep-alive", "connection-pool"],
        "data": "x".repeat(1000), // 1KB response
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });

    loop {
        match server.accept().await {
            Ok(accept) => {
                let test_response = test_response.clone();
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
                                        // Keep-alive 응답 전송
                                        let _ = response.body_mut().set_arena_json(&test_response);
                                        *response.status_mut() = StatusCode::OK;

                                        #[cfg(feature = "connection_pool")]
                                        {
                                            // Keep-alive 헤더 명시적으로 설정
                                            use http::header::{HeaderValue, CONNECTION};
                                            let _ = response.headers_mut().insert(
                                                CONNECTION,
                                                HeaderValue::from_static("keep-alive"),
                                            );
                                        }

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

async fn run_close_connection_server(port: u16, server_ready: Arc<Notify>) {
    println!("🚀 Connection-Close 테스트 서버 시작 (포트: {})", port);

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
    server_ready.notify_one();

    let test_response = json!({
        "message": "no connection pool benchmark response",
        "features": ["connection-close"],
        "data": "x".repeat(1000), // 1KB response
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });

    loop {
        match server.accept().await {
            Ok(accept) => {
                let test_response = test_response.clone();
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
                                        // Connection close 응답 전송
                                        let _ = response.body_mut().set_arena_json(&test_response);
                                        *response.status_mut() = StatusCode::OK;

                                        // Connection close 헤더 명시적으로 설정
                                        use http::header::{HeaderValue, CONNECTION};
                                        let _ = response
                                            .headers_mut()
                                            .insert(CONNECTION, HeaderValue::from_static("close"));

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

async fn benchmark_keep_alive_client(port: u16, concurrent_connections: usize) -> Duration {
    println!(
        "📡 Keep-Alive 클라이언트 벤치마크 시작: {} 동시 연결",
        concurrent_connections
    );

    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(concurrent_connections)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let url = format!("http://127.0.0.1:{}/test", port);
    let start = Instant::now();

    let mut tasks = Vec::new();

    for _ in 0..concurrent_connections {
        let client = client.clone();
        let url = url.clone();

        let task = tokio::spawn(async move {
            for _ in 0..REQUESTS_PER_CONNECTION {
                match client.get(&url).send().await {
                    Ok(response) => {
                        let _ = response.bytes().await;
                    }
                    Err(e) => {
                        eprintln!("❌ Keep-alive 요청 실패: {}", e);
                    }
                }
            }
        });

        tasks.push(task);
    }

    // 모든 태스크 완료 대기
    for task in tasks {
        let _ = task.await;
    }

    let duration = start.elapsed();
    let total_requests = concurrent_connections * REQUESTS_PER_CONNECTION;
    let rps = total_requests as f64 / duration.as_secs_f64();

    println!(
        "✅ Keep-Alive: {} req/s ({} 총 요청, {}ms)",
        rps,
        total_requests,
        duration.as_millis()
    );

    duration
}

async fn benchmark_close_connection_client(port: u16, concurrent_connections: usize) -> Duration {
    println!(
        "📡 Connection-Close 클라이언트 벤치마크 시작: {} 동시 연결",
        concurrent_connections
    );

    let start = Instant::now();
    let mut tasks = Vec::new();

    for _ in 0..concurrent_connections {
        let task = tokio::spawn(async move {
            for _ in 0..REQUESTS_PER_CONNECTION {
                // 매번 새로운 클라이언트 생성 (연결 재사용 방지)
                let client = reqwest::Client::builder()
                    .pool_max_idle_per_host(0) // 연결 풀 비활성화
                    .build()
                    .unwrap();

                let url = format!("http://127.0.0.1:{}/test", port);

                match client.get(&url).send().await {
                    Ok(response) => {
                        let _ = response.bytes().await;
                    }
                    Err(e) => {
                        eprintln!("❌ Connection-close 요청 실패: {}", e);
                    }
                }
            }
        });

        tasks.push(task);
    }

    // 모든 태스크 완료 대기
    for task in tasks {
        let _ = task.await;
    }

    let duration = start.elapsed();
    let total_requests = concurrent_connections * REQUESTS_PER_CONNECTION;
    let rps = total_requests as f64 / duration.as_secs_f64();

    println!(
        "✅ Connection-Close: {} req/s ({} 총 요청, {}ms)",
        rps,
        total_requests,
        duration.as_millis()
    );

    duration
}

async fn shutdown_server(port: u16) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/shutdown", port);
    let _ = client.get(&url).send().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn run_benchmark_scenario(concurrent_connections: usize, scenario_name: &str) {
    println!(
        "\n🎯 벤치마크 시나리오: {} ({} 동시 연결)",
        scenario_name, concurrent_connections
    );
    println!("{}", "=".repeat(70));

    // Keep-Alive 서버 테스트
    let keep_alive_port = TEST_PORT_BASE;
    let server_ready = Arc::new(Notify::new());
    let server_ready_clone = server_ready.clone();

    let ka_server_handle = tokio::spawn(async move {
        run_keep_alive_server(keep_alive_port, server_ready_clone).await;
    });

    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let keep_alive_duration =
        benchmark_keep_alive_client(keep_alive_port, concurrent_connections).await;

    shutdown_server(keep_alive_port).await;
    ka_server_handle.abort();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connection-Close 서버 테스트
    let close_port = TEST_PORT_BASE + 1;
    let server_ready = Arc::new(Notify::new());
    let server_ready_clone = server_ready.clone();

    let close_server_handle = tokio::spawn(async move {
        run_close_connection_server(close_port, server_ready_clone).await;
    });

    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let close_duration =
        benchmark_close_connection_client(close_port, concurrent_connections).await;

    shutdown_server(close_port).await;
    close_server_handle.abort();

    // 성능 비교
    let total_requests = concurrent_connections * REQUESTS_PER_CONNECTION;
    let keep_alive_rps = total_requests as f64 / keep_alive_duration.as_secs_f64();
    let close_rps = total_requests as f64 / close_duration.as_secs_f64();
    let improvement = keep_alive_rps / close_rps;

    println!("\n📊 성능 비교 결과:");
    println!("   Keep-Alive:     {:.2} req/s", keep_alive_rps);
    println!("   Connection-Close: {:.2} req/s", close_rps);

    if improvement > 1.0 {
        println!(
            "   🏆 Keep-Alive 개선: {:.2}x 더 빠름 ({:.1}% 향상)",
            improvement,
            (improvement - 1.0) * 100.0
        );
    } else {
        println!("   📊 Connection-Close가 {:.2}x 더 빠름", 1.0 / improvement);
    }

    let latency_improvement =
        close_duration.as_millis() as f64 / keep_alive_duration.as_millis() as f64;
    println!("   ⚡ 지연시간 개선: {:.2}x 더 빠름", latency_improvement);
}

#[tokio::main]
async fn main() {
    println!("🚀 Connection Pool & Keep-Alive 성능 벤치마크");
    println!("{}", "=".repeat(70));

    #[cfg(feature = "connection_pool")]
    println!("🔥 Connection Pool 기능이 활성화됨");

    #[cfg(not(feature = "connection_pool"))]
    println!("⚠️  Connection Pool 기능이 비활성화됨");

    println!("📊 테스트 설정:");
    println!("   - 연결당 요청 수: {}", REQUESTS_PER_CONNECTION);
    println!("   - 응답 크기: ~1KB");

    for &(concurrent_connections, scenario_name) in TEST_SCENARIOS {
        run_benchmark_scenario(concurrent_connections, scenario_name).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    println!("\n✅ 모든 벤치마크 완료!");
    println!(
        "💡 Connection pooling과 keep-alive는 동시 연결이 많을수록 더 큰 성능 향상을 보입니다."
    );
}
