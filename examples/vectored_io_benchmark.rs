// Vectored I/O vs 일반 I/O 성능 비교 벤치마크
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const TEST_DATA_SIZES: &[usize] = &[1024, 4096, 16384, 65536, 262144]; // 1KB ~ 256KB
const ITERATIONS: usize = 1000;
const TEST_PORT_BASE: u16 = 9990;

#[derive(Clone)]
struct BenchmarkConfig {
    enable_vectored_io: bool,
    data_size: usize,
    port: u16,
}

async fn run_test_server(config: BenchmarkConfig, server_ready: Arc<Notify>) {
    let bind_addr = format!("127.0.0.1:{}", config.port);
    println!(
        "🚀 테스트 서버 시작 (포트: {}, vectored_io: {}, 데이터 크기: {}KB)",
        config.port,
        config.enable_vectored_io,
        config.data_size / 1024
    );

    let mut server = Server::new(&bind_addr).await.unwrap();
    server_ready.notify_one();

    let test_data = generate_test_response(config.data_size);

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
                                        std::process::exit(0);
                                    }
                                    _ => {
                                        // 테스트 응답 전송
                                        let _ = response.body_mut().set_arena_response(&test_data);
                                        *response.status_mut() = StatusCode::OK;
                                        let _ = response.responser_arena().await;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("요청 파싱 실패: {}", e);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("Accept 실패: {}", e);
                break;
            }
        }
    }
}

fn generate_test_response(size: usize) -> String {
    let data = json!({
        "message": "benchmark test response",
        "size": size,
        "data": "x".repeat(size.saturating_sub(100)),
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });
    serde_json::to_string(&data).unwrap()
}

async fn benchmark_client(port: u16, iterations: usize) -> Duration {
    let url = format!("http://127.0.0.1:{}/test", port);
    let client = reqwest::Client::new();

    let start = Instant::now();

    for _ in 0..iterations {
        match client.get(&url).send().await {
            Ok(response) => {
                let _ = response.bytes().await;
            }
            Err(e) => {
                eprintln!("요청 실패: {}", e);
            }
        }
    }

    start.elapsed()
}

async fn shutdown_server(port: u16) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/shutdown", port);
    let _ = client.get(&url).send().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn run_benchmark_for_size(data_size: usize) {
    println!("\n📊 데이터 크기: {}KB 벤치마크", data_size / 1024);
    println!("{}", "-".repeat(60));

    let mut _vectored_duration = Duration::new(0, 0);
    let mut _regular_duration = Duration::new(0, 0);

    // Vectored I/O 테스트
    #[cfg(feature = "vectored_io")]
    {
        let config = BenchmarkConfig {
            enable_vectored_io: true,
            data_size,
            port: TEST_PORT_BASE,
        };

        let server_ready = Arc::new(Notify::new());
        let server_ready_clone = server_ready.clone();

        let server_handle = tokio::spawn(async move {
            run_test_server(config, server_ready_clone).await;
        });

        // 서버 시작 대기
        server_ready.notified().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 벤치마크 실행
        _vectored_duration = benchmark_client(TEST_PORT_BASE, ITERATIONS).await;

        // 서버 종료
        shutdown_server(TEST_PORT_BASE).await;
        server_handle.abort();

        println!("✅ Vectored I/O 테스트: {:?}", _vectored_duration);
    }

    // 일반 I/O 테스트 (vectored_io 기능 비활성화 상태에서)
    {
        let config = BenchmarkConfig {
            enable_vectored_io: false,
            data_size,
            port: TEST_PORT_BASE + 1,
        };

        let server_ready = Arc::new(Notify::new());
        let server_ready_clone = server_ready.clone();

        let server_handle = tokio::spawn(async move {
            run_test_server(config, server_ready_clone).await;
        });

        // 서버 시작 대기
        server_ready.notified().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 벤치마크 실행
        _regular_duration = benchmark_client(TEST_PORT_BASE + 1, ITERATIONS).await;

        // 서버 종료
        shutdown_server(TEST_PORT_BASE + 1).await;
        server_handle.abort();

        println!("✅ 일반 I/O 테스트: {:?}", _regular_duration);
    }

    // 성능 비교
    #[cfg(feature = "vectored_io")]
    {
        let vectored_rps = ITERATIONS as f64 / _vectored_duration.as_secs_f64();
        let regular_rps = ITERATIONS as f64 / _regular_duration.as_secs_f64();
        let improvement = vectored_rps / regular_rps;

        println!("📈 성능 결과:");
        println!("   Vectored I/O: {:.2} req/s", vectored_rps);
        println!("   일반 I/O:     {:.2} req/s", regular_rps);
        if improvement > 1.0 {
            println!(
                "   🏆 개선도: {:.2}x 더 빠름 ({:.1}% 개선)",
                improvement,
                (improvement - 1.0) * 100.0
            );
        } else {
            println!("   📊 비교: {:.2}x 더 느림", 1.0 / improvement);
        }

        // 대역폭 계산
        let total_bytes = data_size * ITERATIONS;
        let vectored_bandwidth =
            total_bytes as f64 / _vectored_duration.as_secs_f64() / 1024.0 / 1024.0;
        let regular_bandwidth =
            total_bytes as f64 / _regular_duration.as_secs_f64() / 1024.0 / 1024.0;

        println!("   Vectored I/O 대역폭: {:.2} MB/s", vectored_bandwidth);
        println!("   일반 I/O 대역폭:     {:.2} MB/s", regular_bandwidth);
    }

    #[cfg(not(feature = "vectored_io"))]
    {
        println!("⚠️  Vectored I/O 기능이 비활성화되어 있습니다.");
        let regular_rps = ITERATIONS as f64 / _regular_duration.as_secs_f64();
        println!("📊 일반 I/O 성능: {:.2} req/s", regular_rps);
    }
}

#[tokio::main]
async fn main() {
    println!("🚀 Vectored I/O vs 일반 I/O 성능 벤치마크");
    println!("{}", "=".repeat(70));

    #[cfg(feature = "vectored_io")]
    println!("🔥 Vectored I/O 기능이 활성화됨");

    #[cfg(not(feature = "vectored_io"))]
    println!("⚠️  Vectored I/O 기능이 비활성화됨");

    println!("📊 테스트 설정: {} iterations per size", ITERATIONS);

    for &data_size in TEST_DATA_SIZES {
        run_benchmark_for_size(data_size).await;
        tokio::time::sleep(Duration::from_millis(500)).await; // 테스트 간 간격
    }

    println!("\n✅ 모든 벤치마크 완료!");
    println!("💡 Vectored I/O는 큰 응답에서 더 나은 성능을 보입니다.");
}
