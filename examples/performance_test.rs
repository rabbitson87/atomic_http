use atomic_http::*;
use std::time::{Duration, Instant};

// 표준 HTTP 서버 (arena 피쳐 없이)
#[cfg(not(feature = "arena"))]
async fn run_standard_server(port: u16) {
    use http::StatusCode;

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();

    loop {
        let (stream, options) = server.accept().await.unwrap();

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
                                "metadata_count": data.metadata.len()
                            });
                            response.body_mut().body = response_data.to_string();
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(_) => {
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    response.responser().await.ok();
                }
                Err(_) => {}
            }
        });
    }
}

// 아레나 HTTP 서버 (arena 피쳐 포함)
#[cfg(feature = "arena")]
async fn run_arena_server(port: u16) {
    use http::StatusCode;

    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();

    loop {
        let (stream, options, herd) = server.accept().await.unwrap();

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
                                "metadata_count": data.metadata.len()
                            });
                            response.body_mut().set_arena_json(&response_data).unwrap();
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(_) => {
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    }
                    response
                        .responser_arena()
                        .await
                        .expect("Failed to send response");
                }
                Err(_) => {}
            }
        });
    }
}

// HTTP 클라이언트 테스트
async fn send_request(port: u16, data: &TestData) -> Result<Duration, SendableError> {
    let client = reqwest::Client::new();
    let json_data = serde_json::to_string(data)?;

    let start = Instant::now();

    let response = client
        .post(&format!("http://127.0.0.1:{}/test", port))
        .header("Content-Type", "application/json")
        .body(json_data)
        .send()
        .await?;

    let _body = response.text().await?;

    Ok(start.elapsed())
}

// 성능 테스트 함수
async fn performance_test(
    port: u16,
    label: &str,
    data: &TestData,
    iterations: usize,
) -> (Duration, Duration, Duration, Duration) {
    println!("🧪 {} 테스트 중... ({}회)", label, iterations);

    let mut times = Vec::new();

    for i in 0..iterations {
        if i % 10 == 0 {
            print!(".");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        }

        if let Ok(duration) = send_request(port, data).await {
            times.push(duration);
        }

        // 약간의 간격으로 더 현실적인 테스트
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    println!(" 완료!");

    if times.is_empty() {
        return (
            Duration::from_millis(0),
            Duration::from_millis(0),
            Duration::from_millis(0),
            Duration::from_millis(0),
        );
    }

    times.sort();
    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    let min = times[0];
    let max = times[times.len() - 1];

    // 95th percentile 추가
    let p95_index = (times.len() as f64 * 0.95) as usize;
    let p95 = times[p95_index.min(times.len() - 1)];

    (avg, min, max, p95)
}

#[tokio::main]
async fn main() {
    println!("🚀 고부하 HTTP 성능 테스트 시작");

    #[cfg(feature = "arena")]
    println!("✅ Arena 피쳐 활성화됨");

    #[cfg(not(feature = "arena"))]
    println!("📝 표준 HTTP 모드");

    // 서버 시작
    #[cfg(not(feature = "arena"))]
    {
        tokio::spawn(async { run_standard_server(9080).await });
        println!("🖥️  표준 HTTP 서버 시작됨 (포트: 9080)");
    }

    #[cfg(feature = "arena")]
    {
        tokio::spawn(async { run_arena_server(9081).await });
        println!("🖥️  아레나 HTTP 서버 시작됨 (포트: 9081)");
    }

    // 서버 시작 대기
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 더 큰 테스트 크기와 더 많은 반복
    let test_sizes = vec![100, 500, 1000, 2000]; // KB
    let iterations = 100; // 더 많은 반복

    println!("\n📊 고부하 성능 테스트 결과");
    println!(
        "{:<10} {:<15} {:<15} {:<15} {:<15}",
        "크기(KB)", "평균(ms)", "최소(ms)", "최대(ms)", "95th(ms)"
    );
    println!("{:-<75}", "");

    for size in &test_sizes {
        let data = TestData::generate(*size);

        // 실제 JSON 크기 확인
        let json_size = serde_json::to_string(&data).unwrap().len();
        println!(
            "실제 JSON 크기: {} bytes ({:.1} KB)",
            json_size,
            json_size as f64 / 1024.0
        );

        #[cfg(not(feature = "arena"))]
        let (avg, min, max, p95) = performance_test(9080, "표준 HTTP", &data, iterations).await;

        #[cfg(feature = "arena")]
        let (avg, min, max, p95) = performance_test(9081, "아레나 HTTP", &data, iterations).await;

        println!(
            "{:<10} {:<15.2} {:<15.2} {:<15.2} {:<15.2}",
            size,
            avg.as_millis(),
            min.as_millis(),
            max.as_millis(),
            p95.as_millis()
        );
    }

    #[cfg(feature = "arena")]
    println!("\n✨ 아레나 HTTP 고부하 측정 완료!");

    #[cfg(not(feature = "arena"))]
    println!("\n✨ 표준 HTTP 고부하 측정 완료!");

    println!("\n💡 다음 단계:");
    println!("   부하 테스트: cargo run --example load_test_client -- -n 5000 -c 200");
    println!("   릴리즈 모드: cargo run --release --example performance_test --features arena");
}
