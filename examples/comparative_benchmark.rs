use atomic_http::*;
use clap::{Arg, Command};
use http::StatusCode;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Notify, Semaphore};

// 벤치마크 결과 구조체
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub test_name: String,
    pub server_type: String,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub total_duration: Duration,
    pub average_latency: Duration,
    pub p95_latency: Duration,
    pub requests_per_second: f64,
    pub throughput_mbps: f64,
}

// 비교 벤치마크 매니저
pub struct ComparativeBenchmark {
    #[cfg(feature = "arena")]
    arena_port: u16,
    standard_port: u16,
    #[cfg(feature = "arena")]
    arena_ready: Arc<Notify>,
    standard_ready: Arc<Notify>,
    results: Vec<BenchmarkResult>,
}

impl ComparativeBenchmark {
    pub fn new(#[cfg(feature = "arena")] arena_port: u16, standard_port: u16) -> Self {
        Self {
            #[cfg(feature = "arena")]
            arena_port,
            standard_port,
            #[cfg(feature = "arena")]
            arena_ready: Arc::new(Notify::new()),
            standard_ready: Arc::new(Notify::new()),
            results: Vec::new(),
        }
    }

    // 비교 벤치마크 실행
    pub async fn run_comparative_benchmark(&mut self) -> Result<(), SendableError> {
        println!("🏁 비교 성능 벤치마크 시작");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 서버 시작
        self.start_servers().await?;

        // 벤치마크 실행
        self.run_benchmarks().await;

        // 결과 분석 및 출력
        self.analyze_and_print_results();

        Ok(())
    }

    // 두 서버 시작
    async fn start_servers(&self) -> Result<(), SendableError> {
        println!("🚀 서버 시작 중...");

        #[cfg(feature = "arena")]
        let (_arena_shutdown_tx, arena_shutdown_rx) = broadcast::channel(1);
        let (_standard_shutdown_tx, standard_shutdown_rx) = broadcast::channel(1);

        // Arena 서버 시작
        #[cfg(feature = "arena")]
        {
            let arena_ready = self.arena_ready.clone();
            let arena_port = self.arena_port;
            tokio::spawn(async move {
                if let Err(e) =
                    Self::run_arena_server(arena_port, arena_ready, arena_shutdown_rx).await
                {
                    eprintln!("Arena 서버 오류: {}", e);
                }
            });
        }

        // 표준 서버 시작
        let standard_ready = self.standard_ready.clone();
        let standard_port = self.standard_port;
        tokio::spawn(async move {
            if let Err(e) =
                Self::run_standard_server(standard_port, standard_ready, standard_shutdown_rx).await
            {
                eprintln!("표준 서버 오류: {}", e);
            }
        });

        // 서버 준비 대기
        println!("⏳ 서버 준비 대기 중...");

        #[cfg(feature = "arena")]
        {
            self.arena_ready.notified().await;
            println!("✅ Arena 서버 준비됨");
        }

        self.standard_ready.notified().await;
        println!("✅ 표준 서버 준비됨");

        println!("⏳ 추가 안정화 대기...");
        tokio::time::sleep(Duration::from_millis(2000)).await;

        println!("✅ 모든 서버 준비 완료!");

        // 연결 테스트
        self.verify_servers().await?;

        Ok(())
    }

    // 서버 연결 확인
    async fn verify_servers(&self) -> Result<(), SendableError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;

        // 더 많은 재시도와 대기 시간
        let max_retries = 10;
        let retry_delay = Duration::from_millis(500);

        #[cfg(feature = "arena")]
        {
            println!("🔍 Arena 서버 연결 확인 중... (포트: {})", self.arena_port);
            let mut arena_success = false;

            for attempt in 1..=max_retries {
                match client
                    .get(&format!("http://127.0.0.1:{}/", self.arena_port))
                    .send()
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            println!("✅ Arena 서버 연결 성공 (시도 {})", attempt);
                            arena_success = true;
                            break;
                        } else {
                            println!(
                                "⚠️ Arena 서버 응답 오류: {} (시도 {})",
                                response.status(),
                                attempt
                            );
                        }
                    }
                    Err(e) => {
                        println!("❌ Arena 서버 연결 실패 (시도 {}): {}", attempt, e);
                        if attempt < max_retries {
                            tokio::time::sleep(retry_delay).await;
                        }
                    }
                }
            }

            if !arena_success {
                return Err("Arena 서버 연결 실패".into());
            }
        }

        println!(
            "🔍 표준 서버 연결 확인 중... (포트: {})",
            self.standard_port
        );
        let mut standard_success = false;

        for attempt in 1..=max_retries {
            match client
                .get(&format!("http://127.0.0.1:{}/", self.standard_port))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        println!("✅ 표준 서버 연결 성공 (시도 {})", attempt);
                        standard_success = true;
                        break;
                    } else {
                        println!(
                            "⚠️ 표준 서버 응답 오류: {} (시도 {})",
                            response.status(),
                            attempt
                        );
                    }
                }
                Err(e) => {
                    println!("❌ 표준 서버 연결 실패 (시도 {}): {}", attempt, e);
                    if attempt < max_retries {
                        tokio::time::sleep(retry_delay).await;
                    }
                }
            }
        }

        if !standard_success {
            return Err("표준 서버 연결 실패".into());
        }

        // 추가 안정화 시간
        println!("⏳ 서버 안정화 대기...");
        tokio::time::sleep(Duration::from_millis(1000)).await;

        Ok(())
    }

    // Arena 서버 실행
    #[cfg(feature = "arena")]
    async fn run_arena_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();
        println!("🏗️ Arena 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                accept_result = server.accept() => {
                    match accept_result {
                        Ok(accept) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_arena_benchmark_request(accept).await {
                                    eprintln!("Arena 요청 처리 오류: {}", e);
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        Ok(())
    }

    // 표준 서버 실행
    async fn run_standard_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();
        println!("📝 표준 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                accept_result = server.accept() => {
                    match accept_result {
                        Ok(accept) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_standard_benchmark_request(accept).await {
                                    eprintln!("표준 요청 처리 오류: {}", e);
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        Ok(())
    }

    // Arena 벤치마크 요청 처리
    #[cfg(feature = "arena")]
    async fn handle_arena_benchmark_request(accept: Accept) -> Result<(), SendableError> {
        match accept.parse_request_arena_writer().await {
            Ok((request, mut response)) => {
                let method = request.method().clone();
                let path = request.uri().path().to_string();
                println!("🏗️ Arena 서버 요청: {} {}", method, path);

                // Content-Type 헤더 설정
                response
                    .headers_mut()
                    .insert("Content-Type", "application/json".parse().unwrap());

                match path.as_str() {
                    "/" => {
                        let info = json!({
                            "server": "arena",
                            "status": "ready",
                            "endpoints": ["/", "/benchmark"]
                        });

                        // JSON 문자열로 변환하여 길이 확인
                        let json_str = serde_json::to_string(&info)?;
                        response.headers_mut().insert(
                            "Content-Length",
                            json_str.len().to_string().parse().unwrap(),
                        );

                        response.body_mut().set_arena_response(&json_str)?;
                        *response.status_mut() = StatusCode::OK;
                        println!("✅ Arena 루트 응답 완료");
                    }

                    "/benchmark" => {
                        println!("🧪 Arena /benchmark 엔드포인트 호출됨 ({})", method);

                        if method == http::Method::GET {
                            let info = json!({
                                "server": "arena",
                                "endpoint": "/benchmark",
                                "status": "ready",
                                "method": "GET"
                            });

                            let json_str = serde_json::to_string(&info)?;
                            response.headers_mut().insert(
                                "Content-Length",
                                json_str.len().to_string().parse().unwrap(),
                            );

                            response.body_mut().set_arena_response(&json_str)?;
                            *response.status_mut() = StatusCode::OK;
                            println!("✅ Arena /benchmark GET 응답 완료");
                        } else if method == http::Method::POST {
                            let start_time = Instant::now();
                            println!("🧪 Arena JSON 파싱 시작...");

                            match request.get_json_arena::<TestData>() {
                                Ok(data) => {
                                    let processing_time = start_time.elapsed();
                                    let result = json!({
                                        "status": "success",
                                        "server_type": "arena",
                                        "data_id": data.id,
                                        "data_size": data.payload.len(),
                                        "processing_time_ms": processing_time.as_millis(),
                                        "memory_model": "arena_zero_copy"
                                    });

                                    let json_str = serde_json::to_string(&result)?;
                                    response.headers_mut().insert(
                                        "Content-Length",
                                        json_str.len().to_string().parse().unwrap(),
                                    );

                                    response.body_mut().set_arena_response(&json_str)?;
                                    *response.status_mut() = StatusCode::OK;

                                    println!(
                                        "✅ Arena 벤치마크 응답 완료: {}KB, {:.2}ms",
                                        data.payload.len() / 1024,
                                        processing_time.as_millis()
                                    );
                                }
                                Err(e) => {
                                    println!("❌ Arena JSON 파싱 실패: {}", e);
                                    let error = json!({
                                        "status": "error",
                                        "message": e.to_string()
                                    });

                                    let json_str = serde_json::to_string(&error)?;
                                    response.headers_mut().insert(
                                        "Content-Length",
                                        json_str.len().to_string().parse().unwrap(),
                                    );

                                    response.body_mut().set_arena_response(&json_str)?;
                                    *response.status_mut() = StatusCode::BAD_REQUEST;
                                }
                            }
                        } else {
                            println!("❓ Arena 지원하지 않는 메서드: {}", method);
                            *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;

                            let error_msg = "Method not allowed";
                            response.headers_mut().insert(
                                "Content-Length",
                                error_msg.len().to_string().parse().unwrap(),
                            );

                            response.body_mut().set_arena_response(error_msg)?;
                        }
                    }

                    _ => {
                        println!("❌ Arena 알 수 없는 경로: {}", path);
                        *response.status_mut() = StatusCode::NOT_FOUND;

                        let error_msg = format!("Not found: {}", path);
                        response.headers_mut().insert(
                            "Content-Length",
                            error_msg.len().to_string().parse().unwrap(),
                        );

                        response.body_mut().set_arena_response(&error_msg)?;
                    }
                }

                if let Err(e) = response.responser_arena().await {
                    println!("❌ Arena 응답 전송 실패: {}", e);
                } else {
                    println!("✅ Arena 응답 전송 완료: {} {}", method, path);
                }
            }
            Err(e) => {
                eprintln!("❌ Arena 요청 파싱 실패: {}", e);
            }
        }

        Ok(())
    }

    // 표준 벤치마크 요청 처리
    async fn handle_standard_benchmark_request(accept: Accept) -> Result<(), SendableError> {
        match accept.parse_request().await {
            Ok((mut request, mut response)) => {
                let method = request.method().clone();
                let path = request.uri().path().to_string();
                println!("📝 표준 서버 요청: {} {}", method, path);

                // Content-Type 헤더 설정
                response
                    .headers_mut()
                    .insert("Content-Type", "application/json".parse().unwrap());

                match path.as_str() {
                    "/" => {
                        let info = json!({
                            "server": "standard",
                            "status": "ready",
                            "endpoints": ["/", "/benchmark"]
                        });

                        let json_str = info.to_string();
                        response.headers_mut().insert(
                            "Content-Length",
                            json_str.len().to_string().parse().unwrap(),
                        );

                        response.body_mut().body = json_str;
                        *response.status_mut() = StatusCode::OK;
                        println!("✅ 표준 루트 응답 완료");
                    }

                    "/benchmark" => {
                        println!("🧪 표준 /benchmark 엔드포인트 호출됨 ({})", method);

                        if method == http::Method::GET {
                            let info = json!({
                                "server": "standard",
                                "endpoint": "/benchmark",
                                "status": "ready",
                                "method": "GET"
                            });

                            let json_str = info.to_string();
                            response.headers_mut().insert(
                                "Content-Length",
                                json_str.len().to_string().parse().unwrap(),
                            );

                            response.body_mut().body = json_str;
                            *response.status_mut() = StatusCode::OK;
                            println!("✅ 표준 /benchmark GET 응답 완료");
                        } else if method == http::Method::POST {
                            let start_time = Instant::now();
                            println!("🧪 표준 JSON 파싱 시작...");

                            match request.get_json::<TestData>() {
                                Ok(data) => {
                                    let processing_time = start_time.elapsed();
                                    let result = json!({
                                        "status": "success",
                                        "server_type": "standard",
                                        "data_id": data.id,
                                        "data_size": data.payload.len(),
                                        "processing_time_ms": processing_time.as_millis(),
                                        "memory_model": "heap_allocated"
                                    });

                                    let json_str = result.to_string();
                                    response.headers_mut().insert(
                                        "Content-Length",
                                        json_str.len().to_string().parse().unwrap(),
                                    );

                                    response.body_mut().body = json_str;
                                    *response.status_mut() = StatusCode::OK;

                                    println!(
                                        "✅ 표준 벤치마크 응답 완료: {}KB, {:.2}ms",
                                        data.payload.len() / 1024,
                                        processing_time.as_millis()
                                    );
                                }
                                Err(e) => {
                                    println!("❌ 표준 JSON 파싱 실패: {}", e);
                                    let error = json!({
                                        "status": "error",
                                        "message": e.to_string()
                                    });

                                    let json_str = error.to_string();
                                    response.headers_mut().insert(
                                        "Content-Length",
                                        json_str.len().to_string().parse().unwrap(),
                                    );

                                    response.body_mut().body = json_str;
                                    *response.status_mut() = StatusCode::BAD_REQUEST;
                                }
                            }
                        } else {
                            println!("❓ 표준 지원하지 않는 메서드: {}", method);
                            *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;

                            let error_msg = "Method not allowed";
                            response.headers_mut().insert(
                                "Content-Length",
                                error_msg.len().to_string().parse().unwrap(),
                            );

                            response.body_mut().body = error_msg.to_string();
                        }
                    }

                    _ => {
                        println!("❌ 표준 알 수 없는 경로: {}", path);
                        *response.status_mut() = StatusCode::NOT_FOUND;

                        let error_msg = format!("Not found: {}", path);
                        response.headers_mut().insert(
                            "Content-Length",
                            error_msg.len().to_string().parse().unwrap(),
                        );

                        response.body_mut().body = error_msg;
                    }
                }

                if let Err(e) = response.responser().await {
                    println!("❌ 표준 응답 전송 실패: {}", e);
                } else {
                    println!("✅ 표준 응답 전송 완료: {} {}", method, path);
                }
            }
            Err(e) => {
                eprintln!("❌ 표준 요청 파싱 실패: {}", e);
            }
        }

        Ok(())
    }

    // 벤치마크 실행
    async fn run_benchmarks(&mut self) {
        println!("\n📊 벤치마크 테스트 실행");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let test_configs = vec![
            ("소용량 JSON (1KB)", 1, 1000, 50),
            ("중용량 JSON (10KB)", 10, 500, 30),
            ("대용량 JSON (100KB)", 100, 200, 20),
            ("초대용량 JSON (1MB)", 1000, 50, 10),
        ];

        for (test_name, size_kb, total_requests, concurrency) in test_configs {
            println!(
                "\n🧪 {} 테스트 ({}개 요청, 동시성 {})",
                test_name, total_requests, concurrency
            );

            // Arena 서버 테스트
            #[cfg(feature = "arena")]
            {
                println!("  🏗️ Arena 서버 테스트 중...");
                match self
                    .run_single_benchmark(
                        "arena",
                        self.arena_port,
                        size_kb,
                        total_requests,
                        concurrency,
                        test_name,
                    )
                    .await
                {
                    Ok(result) => {
                        println!(
                            "    ✅ 완료: {:.1} req/sec, 평균 {:.1}ms",
                            result.requests_per_second,
                            result.average_latency.as_millis()
                        );
                        self.results.push(result);
                    }
                    Err(e) => {
                        println!("    ❌ 실패: {}", e);
                    }
                }
            }

            // 표준 서버 테스트
            println!("  📝 표준 서버 테스트 중...");
            match self
                .run_single_benchmark(
                    "standard",
                    self.standard_port,
                    size_kb,
                    total_requests,
                    concurrency,
                    test_name,
                )
                .await
            {
                Ok(result) => {
                    println!(
                        "    ✅ 완료: {:.1} req/sec, 평균 {:.1}ms",
                        result.requests_per_second,
                        result.average_latency.as_millis()
                    );
                    self.results.push(result);
                }
                Err(e) => {
                    println!("    ❌ 실패: {}", e);
                }
            }

            // 잠시 대기 (서버 안정화)
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // 단일 벤치마크 실행
    async fn run_single_benchmark(
        &self,
        server_type: &str,
        port: u16,
        size_kb: usize,
        total_requests: usize,
        concurrency: usize,
        test_name: &str,
    ) -> Result<BenchmarkResult, SendableError> {
        println!(
            "🔧 {} 벤치마크 시작: {}KB 데이터, {}개 요청, 동시성 {}",
            server_type, size_kb, total_requests, concurrency
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let url = format!("http://127.0.0.1:{}/benchmark", port);

        // TestData 생성 및 검증
        let mut test_data = TestData::generate(size_kb);
        // ID가 겹치지 않도록 더 확실한 방법으로 설정
        test_data.id = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64)
            + (port as u64 * 1000000)
            + (size_kb as u64 * 1000);

        println!(
            "🔧 테스트 데이터 생성됨: ID={}, 페이로드={}KB",
            test_data.id,
            test_data.payload.len() / 1024
        );

        // JSON 직렬화 테스트
        match serde_json::to_string(&test_data) {
            Ok(json_str) => {
                println!("✅ JSON 직렬화 성공: {}바이트", json_str.len());
            }
            Err(e) => {
                return Err(format!("JSON 직렬화 실패: {}", e).into());
            }
        }

        // 연결 테스트
        println!("🔍 {} 서버 연결 테스트 중... ({})", server_type, url);
        match client
            .get(&format!("http://127.0.0.1:{}/", port))
            .send()
            .await
        {
            Ok(response) => {
                println!("✅ {} 서버 연결 성공: {}", server_type, response.status());
            }
            Err(e) => {
                println!("❌ {} 서버 연결 실패: {}", server_type, e);
                return Err(format!("{} 서버 연결 실패: {}", server_type, e).into());
            }
        }

        // /benchmark 엔드포인트 연결 테스트 (GET 요청으로)
        println!("🔍 {} /benchmark 엔드포인트 테스트 중...", server_type);
        match client.get(&url).send().await {
            Ok(response) => {
                println!(
                    "✅ {} /benchmark 연결 성공: {}",
                    server_type,
                    response.status()
                );
            }
            Err(e) => {
                println!(
                    "❌ {} /benchmark 연결 실패: {} - URL: {}",
                    server_type, e, url
                );
                return Err(format!("{} /benchmark 연결 실패: {}", server_type, e).into());
            }
        }

        let semaphore = Arc::new(Semaphore::new(concurrency));
        let successful_count = Arc::new(AtomicUsize::new(0));
        let failed_count = Arc::new(AtomicUsize::new(0));
        let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let total_bytes = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));

        let start_time = Instant::now();
        let mut handles = Vec::new();

        for i in 0..total_requests {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = client.clone();
            let url = url.clone();
            let data = test_data.clone();
            let successful = successful_count.clone();
            let failed = failed_count.clone();
            let latencies_clone = latencies.clone();
            let bytes_counter = total_bytes.clone();
            let errors = error_count.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;

                let request_start = Instant::now();
                let result = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&data)
                    .send()
                    .await;

                match result {
                    Ok(response) => {
                        let latency = request_start.elapsed();
                        if response.status().is_success() {
                            match response.bytes().await {
                                Ok(body) => {
                                    bytes_counter.fetch_add(body.len(), Ordering::Relaxed);
                                    successful.fetch_add(1, Ordering::Relaxed);

                                    let mut latencies_guard = latencies_clone.lock().await;
                                    latencies_guard.push(latency);

                                    if i < 5 {
                                        // 처음 5개만 로그
                                        println!(
                                            "📊 요청 #{} 성공: {:.1}ms",
                                            i + 1,
                                            latency.as_millis()
                                        );
                                    }
                                }
                                Err(e) => {
                                    println!("❌ 요청 #{} 응답 읽기 실패: {}", i + 1, e);
                                    failed.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        } else {
                            println!("❌ 요청 #{} HTTP 오류: {}", i + 1, response.status());
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        println!("❌ 요청 #{} 네트워크 오류: {}", i + 1, e);
                        errors.fetch_add(1, Ordering::Relaxed);
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });

            handles.push(handle);
        }

        println!("⏳ {}개 요청 처리 대기 중...", total_requests);
        for handle in handles {
            handle.await.unwrap();
        }

        let total_duration = start_time.elapsed();
        let successful = successful_count.load(Ordering::Relaxed);
        let failed = failed_count.load(Ordering::Relaxed);
        let network_errors = error_count.load(Ordering::Relaxed);
        let latencies_vec = latencies.lock().await.clone();
        let bytes_transferred = total_bytes.load(Ordering::Relaxed);

        println!(
            "📊 {} 결과: 성공={}, 실패={}, 네트워크오류={}, 총시간={:.1}s",
            server_type,
            successful,
            failed,
            network_errors,
            total_duration.as_secs_f64()
        );

        // 통계 계산
        let requests_per_second = successful as f64 / total_duration.as_secs_f64();
        let throughput_mbps =
            (bytes_transferred as f64 * 8.0) / (total_duration.as_secs_f64() * 1_000_000.0);

        let (average_latency, p95_latency) = if !latencies_vec.is_empty() {
            let mut sorted = latencies_vec.clone();
            sorted.sort();
            let avg = sorted.iter().sum::<Duration>() / sorted.len() as u32;
            let p95_index = (sorted.len() as f64 * 0.95) as usize;
            let p95 = sorted[p95_index.min(sorted.len() - 1)];
            (avg, p95)
        } else {
            (Duration::from_millis(0), Duration::from_millis(0))
        };

        Ok(BenchmarkResult {
            test_name: test_name.to_string(),
            server_type: server_type.to_string(),
            total_requests,
            successful_requests: successful,
            failed_requests: failed,
            total_duration,
            average_latency,
            p95_latency,
            requests_per_second,
            throughput_mbps,
        })
    }

    // 결과 분석 및 출력
    fn analyze_and_print_results(&self) {
        println!("\n📈 벤치마크 결과 분석");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 결과 테이블 출력
        println!("\n📊 상세 결과:");
        println!(
            "{:<20} {:<10} {:<12} {:<12} {:<12} {:<12}",
            "테스트", "서버", "성공률(%)", "평균(ms)", "95th(ms)", "처리량(req/s)"
        );
        println!("{:-<80}", "");

        for result in &self.results {
            let success_rate =
                (result.successful_requests as f64 / result.total_requests as f64) * 100.0;
            println!(
                "{:<20} {:<10} {:<12.1} {:<12.1} {:<12.1} {:<12.1}",
                result.test_name,
                result.server_type,
                success_rate,
                result.average_latency.as_millis(),
                result.p95_latency.as_millis(),
                result.requests_per_second
            );
        }

        // 성능 비교
        #[cfg(feature = "arena")]
        self.print_performance_comparison();

        // 요약
        self.print_summary();
    }

    // 성능 비교 출력
    #[cfg(feature = "arena")]
    fn print_performance_comparison(&self) {
        println!("\n🔍 Arena vs 표준 서버 비교:");
        println!("{:-<60}", "");

        // 테스트별 비교
        let test_names: std::collections::HashSet<_> =
            self.results.iter().map(|r| &r.test_name).collect();

        for test_name in test_names {
            let arena_result = self
                .results
                .iter()
                .find(|r| r.test_name == *test_name && r.server_type == "arena");
            let standard_result = self
                .results
                .iter()
                .find(|r| r.test_name == *test_name && r.server_type == "standard");

            if let (Some(arena), Some(standard)) = (arena_result, standard_result) {
                let rps_improvement =
                    (arena.requests_per_second / standard.requests_per_second - 1.0) * 100.0;
                let latency_improvement = (standard.average_latency.as_millis() as f64
                    / arena.average_latency.as_millis() as f64
                    - 1.0)
                    * 100.0;

                println!("\n📋 {}:", test_name);
                println!(
                    "  처리량 개선: {:.1}% ({:.1} → {:.1} req/s)",
                    rps_improvement, standard.requests_per_second, arena.requests_per_second
                );
                println!(
                    "  지연시간 개선: {:.1}% ({:.1}ms → {:.1}ms)",
                    latency_improvement,
                    standard.average_latency.as_millis(),
                    arena.average_latency.as_millis()
                );
            }
        }
    }

    // 요약 출력
    fn print_summary(&self) {
        println!("\n🎯 요약:");

        #[cfg(feature = "arena")]
        {
            let arena_results: Vec<_> = self
                .results
                .iter()
                .filter(|r| r.server_type == "arena")
                .collect();
            let standard_results: Vec<_> = self
                .results
                .iter()
                .filter(|r| r.server_type == "standard")
                .collect();

            if !arena_results.is_empty() && !standard_results.is_empty() {
                let arena_avg_rps: f64 = arena_results
                    .iter()
                    .map(|r| r.requests_per_second)
                    .sum::<f64>()
                    / arena_results.len() as f64;
                let standard_avg_rps: f64 = standard_results
                    .iter()
                    .map(|r| r.requests_per_second)
                    .sum::<f64>()
                    / standard_results.len() as f64;

                let overall_improvement = (arena_avg_rps / standard_avg_rps - 1.0) * 100.0;

                println!("🏆 Arena 서버 전체 성능 개선: {:.1}%", overall_improvement);
                println!(
                    "📊 평균 처리량 - Arena: {:.1} req/s, 표준: {:.1} req/s",
                    arena_avg_rps, standard_avg_rps
                );

                println!("\n💡 Arena 서버의 장점:");
                println!("  ✅ 제로카피 메모리 관리");
                println!("  ✅ 낮은 메모리 사용량");
                println!("  ✅ 예측 가능한 성능");
                println!("  ✅ GC 압박 없음");
            }
        }

        #[cfg(not(feature = "arena"))]
        {
            println!("📝 표준 서버로 실행됨");
            println!("🔧 Arena 서버와 비교하려면 --features arena로 컴파일하세요");
        }

        println!("\n✨ 벤치마크 완료!");
    }
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let matches = Command::new("Comparative HTTP Benchmark")
        .version("1.0")
        .about("Arena vs 표준 HTTP 서버 성능 비교")
        .arg(
            Arg::new("arena_port")
                .long("arena-port")
                .value_name("PORT")
                .help("Arena 서버 포트")
                .default_value("9001"),
        )
        .arg(
            Arg::new("standard_port")
                .long("standard-port")
                .value_name("PORT")
                .help("표준 서버 포트")
                .default_value("9002"),
        )
        .get_matches();

    #[cfg(feature = "arena")]
    let arena_port: u16 = matches.get_one::<String>("arena_port").unwrap().parse()?;
    let standard_port: u16 = matches
        .get_one::<String>("standard_port")
        .unwrap()
        .parse()?;

    println!("🚀 HTTP 서버 성능 비교 벤치마크");
    #[cfg(feature = "arena")]
    println!("Arena 서버 포트: {}", arena_port);
    println!("표준 서버 포트: {}", standard_port);

    #[cfg(feature = "arena")]
    println!("🏗️ Arena 기능 활성화됨");

    #[cfg(not(feature = "arena"))]
    println!("📝 표준 모드로 실행");

    let mut benchmark = ComparativeBenchmark::new(
        #[cfg(feature = "arena")]
        arena_port,
        standard_port,
    );
    benchmark.run_comparative_benchmark().await?;

    Ok(())
}
