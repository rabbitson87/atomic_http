use atomic_http::*;
use clap::{Arg, Command};
use http::StatusCode;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify};

// 테스트 결과 구조체
#[derive(Debug, Clone)]
pub struct TestResult {
    pub test_name: String,
    pub success: bool,
    pub duration: Duration,
    pub details: HashMap<String, String>,
    pub error: Option<String>,
}

// 통합 테스트 매니저
pub struct IntegratedTestManager {
    port: u16,
    server_ready: Arc<Notify>,
    shutdown_tx: Option<broadcast::Sender<()>>,
    test_results: Vec<TestResult>,
}

impl IntegratedTestManager {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            server_ready: Arc::new(Notify::new()),
            shutdown_tx: None,
            test_results: Vec::new(),
        }
    }

    // 통합 테스트 실행
    pub async fn run_integrated_tests(&mut self) -> Result<(), SendableError> {
        println!("🚀 통합 테스트 시작");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 1. 테스트 준비
        self.prepare_test_environment().await?;

        // 2. 서버 시작
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        let server_ready = self.server_ready.clone();
        let server_port = self.port;

        // 서버를 백그라운드에서 시작
        let server_handle = tokio::spawn(async move {
            if let Err(e) = Self::run_test_server(server_port, server_ready, shutdown_rx).await {
                eprintln!("❌ 서버 오류: {}", e);
            }
        });

        // 3. 서버 준비 대기
        println!("⏳ 서버 시작 대기 중...");
        self.server_ready.notified().await;
        println!("✅ 서버 준비 완료!");

        // 4. 테스트 실행
        self.run_all_tests().await;

        // 5. 결과 출력
        self.print_test_results();

        // 6. 정리
        println!("\n🧹 테스트 정리 중...");
        let _ = shutdown_tx.send(());

        // 서버 종료 대기 (타임아웃 적용)
        match tokio::time::timeout(Duration::from_secs(5), server_handle).await {
            Ok(_) => println!("✅ 서버 정상 종료"),
            Err(_) => println!("⚠️ 서버 종료 타임아웃"),
        }

        Ok(())
    }

    // 테스트 환경 준비
    async fn prepare_test_environment(&self) -> Result<(), SendableError> {
        println!("🔧 테스트 환경 준비 중...");

        // 테스트 디렉토리 생성
        for dir in &["test_files", "test_json_files", "uploads"] {
            tokio::fs::create_dir_all(dir).await.ok();
        }

        // 테스트 파일 생성
        self.create_test_files().await?;

        println!("✅ 테스트 환경 준비 완료");
        Ok(())
    }

    // 테스트 파일 생성
    async fn create_test_files(&self) -> Result<(), SendableError> {
        // JSON 테스트 파일들
        let json_files = vec![
            ("small_test.json", TestData::generate(1)),
            ("medium_test.json", TestData::generate(10)),
            ("large_test.json", TestData::generate(100)),
        ];

        for (filename, data) in json_files {
            let filepath = format!("test_json_files/{}", filename);
            let json_str = serde_json::to_string_pretty(&data)?;
            tokio::fs::write(&filepath, json_str).await?;
        }

        // 바이너리 테스트 파일들
        let binary_files = vec![
            ("test_1kb.bin", 1024),
            ("test_10kb.bin", 10240),
            ("test_100kb.bin", 102400),
        ];

        for (filename, size) in binary_files {
            let filepath = format!("test_files/{}", filename);
            let data = vec![0u8; size];
            tokio::fs::write(&filepath, data).await?;
        }

        Ok(())
    }

    // 테스트 서버 실행
    async fn run_test_server(
        port: u16,
        server_ready: Arc<Notify>,
        shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        println!("🖥️ 테스트 서버 시작 중... (포트: {})", port);

        #[cfg(feature = "arena")]
        {
            Self::run_arena_test_server(port, server_ready, shutdown_rx).await
        }

        #[cfg(not(feature = "arena"))]
        {
            Self::run_standard_test_server(port, server_ready, shutdown_rx).await
        }
    }

    // Arena 테스트 서버
    #[cfg(feature = "arena")]
    async fn run_arena_test_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

        // 서버 준비 신호
        server_ready.notify_one();
        println!("✅ Arena 테스트 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                // 종료 신호 확인
                _ = shutdown_rx.recv() => {
                    println!("🛑 서버 종료 신호 수신");
                    break;
                }

                // 연결 처리
                accept_result = server.accept() => {
                    match accept_result {
                        Ok(accept) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_arena_request(accept).await {
                                    eprintln!("요청 처리 오류: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("연결 수락 오류: {}", e);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // 표준 테스트 서버
    #[cfg(not(feature = "arena"))]
    async fn run_standard_test_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;

        // 서버 준비 신호
        server_ready.notify_one();
        println!("✅ 표준 테스트 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                // 종료 신호 확인
                _ = shutdown_rx.recv() => {
                    println!("🛑 서버 종료 신호 수신");
                    break;
                }

                // 연결 처리
                accept_result = server.accept() => {
                    match accept_result {
                        Ok((stream, options)) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_standard_request(stream, options).await {
                                    eprintln!("요청 처리 오류: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("연결 수락 오류: {}", e);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // Arena 요청 처리
    #[cfg(feature = "arena")]
    async fn handle_arena_request(accept: Accept) -> Result<(), SendableError> {
        match accept.parse_request_arena_writer().await {
            Ok((request, mut response)) => {
                let path = request.uri().path();

                match path {
                    "/" => {
                        let info = json!({
                            "message": "🏗️ Arena 테스트 서버",
                            "version": "1.0.0",
                            "features": ["arena"],
                            "endpoints": ["/test/json", "/test/performance", "/files/*"]
                        });
                        response.body_mut().set_arena_json(&info)?;
                        *response.status_mut() = StatusCode::OK;
                    }

                    "/test/json" => match request.get_json_arena::<TestData>() {
                        Ok(data) => {
                            let result = json!({
                                "status": "success",
                                "server_type": "arena",
                                "data_id": data.id,
                                "data_size": data.payload.len(),
                                "memory_model": "zero_copy_arena"
                            });
                            response.body_mut().set_arena_json(&result)?;
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(e) => {
                            let error = json!({
                                "status": "error",
                                "message": e.to_string()
                            });
                            response.body_mut().set_arena_json(&error)?;
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    },

                    "/test/performance" => {
                        let perf_data = json!({
                            "server_type": "arena",
                            "memory_efficiency": "high",
                            "allocation_strategy": "arena_based",
                        });
                        response.body_mut().set_arena_json(&perf_data)?;
                        *response.status_mut() = StatusCode::OK;
                    }

                    path if path.starts_with("/files/") => {
                        let filename = &path[7..];
                        let filepath = format!("test_files/{}", filename);

                        if Path::new(&filepath).exists() {
                            #[cfg(feature = "response_file")]
                            {
                                response.body_mut().response_file(&filepath)?;
                                *response.status_mut() = StatusCode::OK;
                            }
                            #[cfg(not(feature = "response_file"))]
                            {
                                let data = tokio::fs::read(&filepath).await?;
                                response
                                    .body_mut()
                                    .set_arena_response(&String::from_utf8_lossy(&data))?;
                                *response.status_mut() = StatusCode::OK;
                            }
                        } else {
                            *response.status_mut() = StatusCode::NOT_FOUND;
                            response.body_mut().set_arena_response("File not found")?;
                        }
                    }

                    _ => {
                        *response.status_mut() = StatusCode::NOT_FOUND;
                        response.body_mut().set_arena_response("Not found")?;
                    }
                }

                response.responser_arena().await?;
            }
            Err(e) => {
                eprintln!("Arena 요청 파싱 실패: {}", e);
            }
        }

        Ok(())
    }

    // 표준 요청 처리
    #[cfg(not(feature = "arena"))]
    async fn handle_standard_request(
        stream: tokio::net::TcpStream,
        options: Options,
    ) -> Result<(), SendableError> {
        match Server::parse_request(stream, options).await {
            Ok((mut request, mut response)) => {
                let path = request.uri().path();

                match path {
                    "/" => {
                        let info = json!({
                            "message": "📝 표준 테스트 서버",
                            "version": "1.0.0",
                            "features": ["standard"],
                            "endpoints": ["/test/json", "/test/performance"]
                        });
                        response.body_mut().body = info.to_string();
                        *response.status_mut() = StatusCode::OK;
                    }

                    "/test/json" => match request.get_json::<TestData>() {
                        Ok(data) => {
                            let result = json!({
                                "status": "success",
                                "server_type": "standard",
                                "data_id": data.id,
                                "data_size": data.payload.len(),
                                "memory_model": "heap_allocated"
                            });
                            response.body_mut().body = result.to_string();
                            *response.status_mut() = StatusCode::OK;
                        }
                        Err(e) => {
                            let error = json!({
                                "status": "error",
                                "message": e.to_string()
                            });
                            response.body_mut().body = error.to_string();
                            *response.status_mut() = StatusCode::BAD_REQUEST;
                        }
                    },

                    "/test/performance" => {
                        let perf_data = json!({
                            "server_type": "standard",
                            "memory_efficiency": "normal",
                            "allocation_strategy": "heap_based",
                        });
                        response.body_mut().body = perf_data.to_string();
                        *response.status_mut() = StatusCode::OK;
                    }

                    _ => {
                        *response.status_mut() = StatusCode::NOT_FOUND;
                        response.body_mut().body = "Not found".to_string();
                    }
                }

                response.responser().await?;
            }
            Err(e) => {
                eprintln!("표준 요청 파싱 실패: {}", e);
            }
        }

        Ok(())
    }

    // 모든 테스트 실행
    async fn run_all_tests(&mut self) {
        println!("\n🧪 테스트 실행 시작");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 1. 기본 연결 테스트
        self.test_basic_connection().await;

        // 2. JSON 파싱 테스트
        self.test_json_parsing().await;

        // 3. 성능 테스트
        self.test_performance().await;

        // 4. 파일 서빙 테스트
        self.test_file_serving().await;

        // 5. 부하 테스트
        self.test_load_performance().await;
    }

    // 기본 연결 테스트
    async fn test_basic_connection(&mut self) {
        let test_name = "basic_connection";
        let start = Instant::now();

        match self.execute_basic_connection_test().await {
            Ok(details) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: true,
                    duration: start.elapsed(),
                    details,
                    error: None,
                });
            }
            Err(e) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: false,
                    duration: start.elapsed(),
                    details: HashMap::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    async fn execute_basic_connection_test(
        &self,
    ) -> Result<HashMap<String, String>, SendableError> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/", self.port);

        let response = client.get(&url).send().await?;
        let status = response.status();
        let body: serde_json::Value = response.json().await?;

        let mut details = HashMap::new();
        details.insert("status".to_string(), status.to_string());
        details.insert(
            "server_type".to_string(),
            body.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        );

        Ok(details)
    }

    // JSON 파싱 테스트
    async fn test_json_parsing(&mut self) {
        let test_name = "json_parsing";
        let start = Instant::now();

        match self.execute_json_parsing_test().await {
            Ok(details) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: true,
                    duration: start.elapsed(),
                    details,
                    error: None,
                });
            }
            Err(e) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: false,
                    duration: start.elapsed(),
                    details: HashMap::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    async fn execute_json_parsing_test(&self) -> Result<HashMap<String, String>, SendableError> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/test/json", self.port);

        let test_sizes = vec![1, 10, 100]; // KB
        let mut details = HashMap::new();

        for size_kb in test_sizes {
            let test_data = TestData::generate(size_kb);
            let start = Instant::now();

            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&test_data)
                .send()
                .await?;

            let duration = start.elapsed();

            details.insert(
                format!("json_{}kb_status", size_kb),
                response.status().to_string(),
            );
            let body: serde_json::Value = response.json().await?;

            details.insert(
                format!("json_{}kb_time_ms", size_kb),
                duration.as_millis().to_string(),
            );
            details.insert(
                format!("json_{}kb_server_type", size_kb),
                body.get("server_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            );
        }

        Ok(details)
    }

    // 성능 테스트
    async fn test_performance(&mut self) {
        let test_name = "performance";
        let start = Instant::now();

        match self.execute_performance_test().await {
            Ok(details) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: true,
                    duration: start.elapsed(),
                    details,
                    error: None,
                });
            }
            Err(e) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: false,
                    duration: start.elapsed(),
                    details: HashMap::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    async fn execute_performance_test(&self) -> Result<HashMap<String, String>, SendableError> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/test/performance", self.port);

        let response = client.get(&url).send().await?;
        let mut details = HashMap::new();
        details.insert(
            "performance_status".to_string(),
            response.status().to_string(),
        );
        let body: serde_json::Value = response.json().await?;

        details.insert(
            "server_type".to_string(),
            body.get("server_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        );
        details.insert(
            "memory_efficiency".to_string(),
            body.get("memory_efficiency")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        );

        Ok(details)
    }

    // 파일 서빙 테스트
    async fn test_file_serving(&mut self) {
        let test_name = "file_serving";
        let start = Instant::now();

        match self.execute_file_serving_test().await {
            Ok(details) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: true,
                    duration: start.elapsed(),
                    details,
                    error: None,
                });
            }
            Err(e) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: false,
                    duration: start.elapsed(),
                    details: HashMap::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    async fn execute_file_serving_test(&self) -> Result<HashMap<String, String>, SendableError> {
        let client = reqwest::Client::new();
        let mut details = HashMap::new();

        let test_files = vec!["test_1kb.bin", "test_10kb.bin", "test_100kb.bin"];

        for filename in test_files {
            let url = format!("http://127.0.0.1:{}/files/{}", self.port, filename);
            let start = Instant::now();

            match client.get(&url).send().await {
                Ok(response) => {
                    let duration = start.elapsed();
                    let size = response.content_length().unwrap_or(0);

                    details.insert(
                        format!("file_{}_status", filename),
                        response.status().to_string(),
                    );
                    details.insert(
                        format!("file_{}_time_ms", filename),
                        duration.as_millis().to_string(),
                    );
                    details.insert(format!("file_{}_size", filename), size.to_string());
                }
                Err(e) => {
                    details.insert(format!("file_{}_error", filename), e.to_string());
                }
            }
        }

        Ok(details)
    }

    // 부하 테스트
    async fn test_load_performance(&mut self) {
        let test_name = "load_performance";
        let start = Instant::now();

        match self.execute_load_test().await {
            Ok(details) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: true,
                    duration: start.elapsed(),
                    details,
                    error: None,
                });
            }
            Err(e) => {
                self.test_results.push(TestResult {
                    test_name: test_name.to_string(),
                    success: false,
                    duration: start.elapsed(),
                    details: HashMap::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    async fn execute_load_test(&self) -> Result<HashMap<String, String>, SendableError> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/test/json", self.port);

        let concurrent_requests = 20;
        let total_requests = 100;
        let test_data = TestData::generate(10); // 10KB

        let start = Instant::now();
        let mut handles = Vec::new();

        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent_requests));
        let success_count = Arc::new(AtomicUsize::new(0));

        for _ in 0..total_requests {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = client.clone();
            let url = url.clone();
            let data = test_data.clone();
            let success_count = success_count.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;

                match client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&data)
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        success_count.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let total_duration = start.elapsed();
        let successful = success_count.load(Ordering::Relaxed);
        let rps = successful as f64 / total_duration.as_secs_f64();

        let mut details = HashMap::new();
        details.insert("total_requests".to_string(), total_requests.to_string());
        details.insert("successful_requests".to_string(), successful.to_string());
        details.insert(
            "total_time_ms".to_string(),
            total_duration.as_millis().to_string(),
        );
        details.insert("requests_per_second".to_string(), format!("{:.1}", rps));
        details.insert(
            "concurrent_connections".to_string(),
            concurrent_requests.to_string(),
        );

        Ok(details)
    }

    // 테스트 결과 출력
    fn print_test_results(&self) {
        println!("\n📊 테스트 결과 요약");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let successful = self.test_results.iter().filter(|r| r.success).count();
        let total = self.test_results.len();

        println!(
            "전체 테스트: {} / 성공: {} / 실패: {}",
            total,
            successful,
            total - successful
        );

        for result in &self.test_results {
            let status = if result.success { "✅" } else { "❌" };
            println!(
                "\n{} {} ({:.2}ms)",
                status,
                result.test_name,
                result.duration.as_millis()
            );

            if result.success {
                for (key, value) in &result.details {
                    println!("   {}: {}", key, value);
                }
            } else if let Some(error) = &result.error {
                println!("   오류: {}", error);
            }
        }

        println!("\n🎯 결과:");
        if successful == total {
            println!("🏆 모든 테스트 통과!");
        } else {
            println!("⚠️ 일부 테스트 실패 ({}/{})", total - successful, total);
        }

        // 성능 요약
        if let Some(load_test) = self
            .test_results
            .iter()
            .find(|r| r.test_name == "load_performance")
        {
            if let Some(rps) = load_test.details.get("requests_per_second") {
                println!("📈 처리량: {} req/sec", rps);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let matches = Command::new("Integrated HTTP Test")
        .version("1.0")
        .about("통합 HTTP 서버 테스트 도구")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("서버 포트")
                .default_value("9090"),
        )
        .get_matches();

    let port: u16 = matches.get_one::<String>("port").unwrap().parse()?;

    println!("🚀 통합 HTTP 테스트 도구");
    println!("포트: {}", port);

    #[cfg(feature = "arena")]
    println!("모드: Arena + Zero-copy");

    #[cfg(not(feature = "arena"))]
    println!("모드: 표준 HTTP");

    let mut test_manager = IntegratedTestManager::new(port);
    test_manager.run_integrated_tests().await?;

    println!("\n✨ 통합 테스트 완료!");

    Ok(())
}
