// 간단한 성능 테스트 (기존 performance_test.rs 간소화)
use atomic_http::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

async fn run_simple_server() -> u16 {
    let port = 9999;
    let server_ready = Arc::new(Notify::new());
    let ready_clone = server_ready.clone();

    #[cfg(feature = "arena")]
    tokio::spawn(async move {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
        ready_clone.notify_one();

        loop {
            if let Ok(accept) = server.accept().await {
                tokio::spawn(async move {
                    if let Ok((request, mut response)) = accept.parse_request_arena_writer().await {
                        if let Ok(data) = request.get_json_arena::<TestData>() {
                            let result = serde_json::json!({
                                "status": "success",
                                "data_size": data.payload.len()
                            });
                            let _ = response.body_mut().set_arena_json(&result);
                        }
                        let _ = response.responser_arena().await;
                    }
                });
            }
        }
    });

    #[cfg(not(feature = "arena"))]
    tokio::spawn(async move {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
        ready_clone.notify_one();

        loop {
            if let Ok(accept) = server.accept().await {
                tokio::spawn(async move {
                    if let Ok((mut request, mut response)) = accept.parse_request().await {
                        if let Ok(data) = request.get_json::<TestData>() {
                            let result = serde_json::json!({
                                "status": "success",
                                "data_size": data.payload.len()
                            });
                            response.body_mut().body = result.to_string();
                        }
                        let _ = response.responser().await;
                    }
                });
            }
        }
    });

    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(100)).await; // 서버 안정화
    port
}

async fn test_performance(port: u16, size_kb: usize, requests: usize) -> (Duration, usize) {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/test", port);
    let test_data = TestData::generate(size_kb);

    let start = Instant::now();
    let mut successful = 0;

    for _ in 0..requests {
        if let Ok(response) = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&test_data)
            .send()
            .await
        {
            if response.status().is_success() {
                successful += 1;
            }
        }
    }

    (start.elapsed(), successful)
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    println!("🚀 간단한 성능 테스트");

    #[cfg(feature = "arena")]
    println!("모드: Arena");

    #[cfg(not(feature = "arena"))]
    println!("모드: 표준");

    let port = run_simple_server().await;
    println!("✅ 서버 시작됨 (포트: {})", port);

    let test_cases = vec![
        (1, 50),   // 1KB, 50 requests
        (10, 30),  // 10KB, 30 requests
        (100, 10), // 100KB, 10 requests
    ];

    println!("\n📊 성능 테스트 결과:");
    println!(
        "{:<8} {:<10} {:<12} {:<15}",
        "크기", "요청수", "총시간(ms)", "처리량(req/s)"
    );
    println!("{:-<50}", "");

    for (size_kb, requests) in test_cases {
        let (duration, successful) = test_performance(port, size_kb, requests).await;
        let rps = successful as f64 / duration.as_secs_f64();

        println!(
            "{:<8}KB {:<10} {:<12} {:<15.1}",
            size_kb,
            successful,
            duration.as_millis(),
            rps
        );
    }

    println!("\n✅ 간단한 성능 테스트 완료!");
    Ok(())
}
