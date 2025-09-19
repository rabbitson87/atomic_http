// 간단한 벤치마크 테스트 (문제 해결용)
use atomic_http::*;
use http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

// 간단한 서버 실행
async fn run_simple_benchmark_server(port: u16, server_ready: Arc<Notify>) {
    println!("🚀 간단한 벤치마크 서버 시작 (포트: {})", port);

    #[cfg(feature = "arena")]
    {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
        server_ready.notify_one();
        println!("✅ Arena 서버 시작됨");

        loop {
            match server.accept().await {
                Ok(accept) => {
                    tokio::spawn(async move {
                        match accept.parse_request_arena_writer().await {
                            Ok((request, mut response)) => {
                                let method = request.method().clone();
                                let path = request.uri().path().to_string();
                                println!("📨 Arena 요청: {} {}", method, path);

                                match path.as_str() {
                                    "/" => {
                                        let info = json!({
                                            "server": "arena",
                                            "status": "ready",
                                            "endpoints": ["/", "/test", "/benchmark"]
                                        });
                                        if let Err(e) = response.body_mut().set_arena_json(&info) {
                                            println!("❌ JSON 설정 실패: {}", e);
                                        }
                                        *response.status_mut() = StatusCode::OK;
                                    }

                                    "/test" | "/benchmark" => {
                                        if method == http::Method::GET {
                                            let info = json!({
                                                "server": "arena",
                                                "endpoint": path,
                                                "method": "GET",
                                                "status": "ready"
                                            });
                                            if let Err(e) =
                                                response.body_mut().set_arena_json(&info)
                                            {
                                                println!("❌ GET JSON 설정 실패: {}", e);
                                            }
                                            *response.status_mut() = StatusCode::OK;
                                            println!("✅ Arena GET {} 응답 완료", path);
                                        } else if method == http::Method::POST {
                                            let start_time = Instant::now();
                                            match request.get_json_arena::<TestData>() {
                                                Ok(data) => {
                                                    let processing_time = start_time.elapsed();
                                                    let result = json!({
                                                        "status": "success",
                                                        "server_type": "arena",
                                                        "endpoint": path,
                                                        "data_id": data.id,
                                                        "payload_size": data.payload.len(),
                                                        "description_len": data.description.len(),
                                                        "tags_count": data.tags.len(),
                                                        "processing_time_ms": processing_time.as_millis()
                                                    });
                                                    if let Err(e) =
                                                        response.body_mut().set_arena_json(&result)
                                                    {
                                                        println!("❌ 결과 JSON 설정 실패: {}", e);
                                                    }
                                                    *response.status_mut() = StatusCode::OK;
                                                    println!("✅ Arena POST {} 성공: ID={}, 페이로드={}KB, {:.1}ms", 
                                                            path, data.id, data.payload.len() / 1024, processing_time.as_millis());
                                                }
                                                Err(e) => {
                                                    println!("❌ Arena JSON 파싱 실패: {}", e);
                                                    let error = json!({
                                                        "status": "error",
                                                        "message": e.to_string()
                                                    });
                                                    if let Err(e) =
                                                        response.body_mut().set_arena_json(&error)
                                                    {
                                                        println!("❌ 에러 JSON 설정 실패: {}", e);
                                                    }
                                                    *response.status_mut() =
                                                        StatusCode::BAD_REQUEST;
                                                }
                                            }
                                        } else {
                                            *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
                                            if let Err(e) = response
                                                .body_mut()
                                                .set_arena_response("Method not allowed")
                                            {
                                                println!("❌ 메서드 에러 응답 설정 실패: {}", e);
                                            }
                                        }
                                    }

                                    _ => {
                                        *response.status_mut() = StatusCode::NOT_FOUND;
                                        if let Err(e) = response
                                            .body_mut()
                                            .set_arena_response(&format!("Not found: {}", path))
                                        {
                                            println!("❌ 404 응답 설정 실패: {}", e);
                                        }
                                    }
                                }

                                if let Err(e) = response.responser_arena().await {
                                    println!("❌ Arena 응답 전송 실패: {}", e);
                                } else {
                                    println!("✅ Arena 응답 전송 완료: {} {}", method, path);
                                }
                            }
                            Err(e) => {
                                println!("❌ Arena 요청 파싱 실패: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    println!("❌ Arena 연결 수락 실패: {}", e);
                }
            }
        }
    }

    #[cfg(not(feature = "arena"))]
    {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await.unwrap();
        server_ready.notify_one();
        println!("✅ 표준 서버 시작됨");

        loop {
            match server.accept().await {
                Ok(accept) => {
                    tokio::spawn(async move {
                        match accept.parse_request().await {
                            Ok((mut request, mut response)) => {
                                let method = request.method().clone();
                                let path = request.uri().path().to_string();
                                println!("📨 표준 요청: {} {}", method, path);

                                match path.as_str() {
                                    "/" => {
                                        let info = json!({
                                            "server": "standard",
                                            "status": "ready",
                                            "endpoints": ["/", "/test", "/benchmark"]
                                        });
                                        response.body_mut().body = info.to_string();
                                        *response.status_mut() = StatusCode::OK;
                                    }

                                    "/test" | "/benchmark" => {
                                        if method == http::Method::GET {
                                            let info = json!({
                                                "server": "standard",
                                                "endpoint": path,
                                                "method": "GET",
                                                "status": "ready"
                                            });
                                            response.body_mut().body = info.to_string();
                                            *response.status_mut() = StatusCode::OK;
                                            println!("✅ 표준 GET {} 응답 완료", path);
                                        } else if method == http::Method::POST {
                                            let start_time = Instant::now();
                                            match request.get_json::<TestData>() {
                                                Ok(data) => {
                                                    let processing_time = start_time.elapsed();
                                                    let result = json!({
                                                        "status": "success",
                                                        "server_type": "standard",
                                                        "endpoint": path,
                                                        "data_id": data.id,
                                                        "payload_size": data.payload.len(),
                                                        "description_len": data.description.len(),
                                                        "tags_count": data.tags.len(),
                                                        "processing_time_ms": processing_time.as_millis()
                                                    });
                                                    response.body_mut().body = result.to_string();
                                                    *response.status_mut() = StatusCode::OK;
                                                    println!("✅ 표준 POST {} 성공: ID={}, 페이로드={}KB, {:.1}ms", 
                                                            path, data.id, data.payload.len() / 1024, processing_time.as_millis());
                                                }
                                                Err(e) => {
                                                    println!("❌ 표준 JSON 파싱 실패: {}", e);
                                                    let error = json!({
                                                        "status": "error",
                                                        "message": e.to_string()
                                                    });
                                                    response.body_mut().body = error.to_string();
                                                    *response.status_mut() =
                                                        StatusCode::BAD_REQUEST;
                                                }
                                            }
                                        } else {
                                            *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
                                            response.body_mut().body =
                                                "Method not allowed".to_string();
                                        }
                                    }

                                    _ => {
                                        *response.status_mut() = StatusCode::NOT_FOUND;
                                        response.body_mut().body = format!("Not found: {}", path);
                                    }
                                }

                                if let Err(e) = response.responser().await {
                                    println!("❌ 표준 응답 전송 실패: {}", e);
                                } else {
                                    println!("✅ 표준 응답 전송 완료: {} {}", method, path);
                                }
                            }
                            Err(e) => {
                                println!("❌ 표준 요청 파싱 실패: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    println!("❌ 표준 연결 수락 실패: {}", e);
                }
            }
        }
    }
}

// 간단한 클라이언트 테스트
async fn run_simple_benchmark_client(port: u16) -> Result<(), SendableError> {
    println!("🧪 간단한 벤치마크 클라이언트 시작");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let base_url = format!("http://127.0.0.1:{}", port);

    // 1. 연결 확인
    println!("\n1️⃣ 기본 연결 확인");
    let start = Instant::now();
    match client.get(&base_url).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await?;
            println!(
                "✅ 루트 연결: {} ({:.1}ms)",
                status,
                start.elapsed().as_millis()
            );
            println!("📄 응답: {}", body);
        }
        Err(e) => {
            println!("❌ 루트 연결 실패: {}", e);
            return Err(e.into());
        }
    }

    // 2. GET /benchmark 테스트
    println!("\n2️⃣ GET /benchmark 테스트");
    let start = Instant::now();
    match client.get(&format!("{}/benchmark", base_url)).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await?;
            println!(
                "✅ GET /benchmark: {} ({:.1}ms)",
                status,
                start.elapsed().as_millis()
            );
            println!("📄 응답: {}", body);
        }
        Err(e) => {
            println!("❌ GET /benchmark 실패: {}", e);
            return Err(e.into());
        }
    }

    // 3. POST 벤치마크 테스트
    println!("\n3️⃣ POST 벤치마크 테스트");
    for size_kb in [1, 10, 100] {
        let mut test_data = TestData::generate(size_kb);
        test_data.id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let start = Instant::now();
        match client
            .post(&format!("{}/benchmark", base_url))
            .header("Content-Type", "application/json")
            .json(&test_data)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await?;
                println!(
                    "✅ POST {}KB: {} ({:.1}ms)",
                    size_kb,
                    status,
                    start.elapsed().as_millis()
                );
                if let Ok(json_response) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(server_time) = json_response.get("processing_time_ms") {
                        println!("   서버 처리시간: {}ms", server_time);
                    }
                    if let Some(received_id) = json_response.get("data_id") {
                        println!("   수신된 ID: {}", received_id);
                    }
                    if let Some(payload_size) = json_response.get("payload_size") {
                        println!("   페이로드 크기: {}바이트", payload_size);
                    }
                } else {
                    println!("📄 응답: {}", body);
                }
            }
            Err(e) => {
                println!("❌ POST {}KB 실패: {}", size_kb, e);
            }
        }
    }

    // 4. 간단한 성능 테스트
    println!("\n4️⃣ 간단한 성능 테스트 (10회 요청)");
    let mut times = Vec::new();

    for i in 1..=10 {
        // 매번 새로운 ID로 테스트 데이터 생성
        let mut test_data = TestData::generate(10); // 10KB
        test_data.id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            + i as u64;

        let start = Instant::now();
        match client
            .post(&format!("{}/benchmark", base_url))
            .header("Content-Type", "application/json")
            .json(&test_data)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let duration = start.elapsed();
                    times.push(duration);
                    println!("  요청 #{}: {:.1}ms", i, duration.as_millis());
                } else {
                    println!("  요청 #{}: HTTP {}", i, response.status());
                }
            }
            Err(e) => {
                println!("  요청 #{}: 실패 - {}", i, e);
            }
        }
    }

    if !times.is_empty() {
        let avg = times.iter().sum::<Duration>() / times.len() as u32;
        let min = times.iter().min().unwrap();
        let max = times.iter().max().unwrap();

        println!("\n📊 성능 요약:");
        println!("  평균: {:.1}ms", avg.as_millis());
        println!("  최소: {:.1}ms", min.as_millis());
        println!("  최대: {:.1}ms", max.as_millis());
        println!("  성공률: {}/10", times.len());
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(9998);

    println!("🚀 간단한 벤치마크 테스트 (포트: {})", port);

    #[cfg(feature = "arena")]
    println!("🏗️ Arena 모드");

    #[cfg(not(feature = "arena"))]
    println!("📝 표준 모드");

    // 서버 시작
    let server_ready = Arc::new(Notify::new());
    let ready_clone = server_ready.clone();

    tokio::spawn(async move {
        run_simple_benchmark_server(port, ready_clone).await;
    });

    // 서버 준비 대기
    println!("⏳ 서버 시작 대기...");
    server_ready.notified().await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    println!("✅ 서버 준비 완료!");

    // 클라이언트 테스트 실행
    run_simple_benchmark_client(port).await?;

    println!("\n✅ 간단한 벤치마크 테스트 완료!");
    println!("\n💡 다음 단계:");
    println!("   1. 이 테스트가 성공하면 comparative_benchmark 실행");
    println!("   2. cargo run --example simple_benchmark_test --features arena");
    println!("   3. cargo run --example comparative_benchmark --features arena");

    Ok(())
}
