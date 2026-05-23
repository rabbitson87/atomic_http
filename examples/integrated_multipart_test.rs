use atomic_http::*;
use http::StatusCode;
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MultipartTestData {
    pub id: u64,
    pub description: String,
    pub payload: Vec<u8>,
    pub tags: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl MultipartTestData {
    pub fn generate(size_kb: usize) -> Self {
        let payload_size = size_kb * 1024;
        let mut metadata = HashMap::new();
        metadata.insert("test".to_string(), "value".to_string());
        metadata.insert("size".to_string(), size_kb.to_string());
        metadata.insert("created_at".to_string(), chrono::Utc::now().to_rfc3339());

        Self {
            id: rand::random(),
            description: format!("Test multipart data with {} KB", size_kb),
            payload: vec![0u8; payload_size],
            tags: vec![
                "test".to_string(),
                "multipart".to_string(),
                "performance".to_string(),
            ],
            metadata,
        }
    }
}

// 멀티파트 테스트 결과
#[derive(Debug, Clone)]
pub struct MultipartTestResult {
    pub test_name: String,
    pub server_type: String,
    pub file_count: usize,
    pub total_size_mb: f64,
    pub upload_time: Duration,
    pub processing_time: Duration,
    pub throughput_mbps: f64,
    pub success: bool,
    pub error: Option<String>,
}

// 통합 멀티파트 테스트 매니저
pub struct IntegratedMultipartTest {
    port: u16,
    server_ready: Arc<Notify>,
    test_results: Vec<MultipartTestResult>,
}

impl IntegratedMultipartTest {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            server_ready: Arc::new(Notify::new()),
            test_results: Vec::new(),
        }
    }

    // 통합 멀티파트 테스트 실행
    pub async fn run_integrated_multipart_test(&mut self) -> Result<(), SendableError> {
        println!("🚀 통합 멀티파트 테스트 시작");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 1. 테스트 환경 준비
        self.prepare_test_environment().await?;

        // 2. 서버 시작
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let server_ready = self.server_ready.clone();
        let server_port = self.port;

        let server_handle = tokio::spawn(async move {
            if let Err(e) = Self::run_multipart_server(server_port, server_ready, shutdown_rx).await
            {
                eprintln!("❌ 멀티파트 서버 오류: {}", e);
            }
        });

        // 3. 서버 준비 대기
        println!("⏳ 멀티파트 서버 시작 대기 중...");
        self.server_ready.notified().await;
        println!("✅ 멀티파트 서버 준비 완료!");

        // 4. 멀티파트 테스트 실행
        self.run_all_multipart_tests().await;

        // 5. 결과 출력
        self.print_multipart_test_results();

        // 6. 정리
        println!("\n🧹 테스트 정리 중...");
        let _ = shutdown_tx.send(());

        match tokio::time::timeout(Duration::from_secs(5), server_handle).await {
            Ok(_) => println!("✅ 서버 정상 종료"),
            Err(_) => println!("⚠️ 서버 종료 타임아웃"),
        }

        Ok(())
    }

    // 테스트 환경 준비
    async fn prepare_test_environment(&self) -> Result<(), SendableError> {
        println!("🔧 멀티파트 테스트 환경 준비 중...");

        // uploads 디렉토리 생성
        tokio::fs::create_dir_all("uploads").await.ok();

        println!("✅ 멀티파트 테스트 환경 준비 완료");
        Ok(())
    }

    // 멀티파트 서버 실행
    async fn run_multipart_server(
        port: u16,
        server_ready: Arc<Notify>,
        shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        println!("🖥️ 멀티파트 서버 시작 중... (포트: {})", port);

        #[cfg(feature = "arena")]
        {
            Self::run_arena_multipart_server(port, server_ready, shutdown_rx).await
        }

        #[cfg(not(feature = "arena"))]
        {
            Self::run_standard_multipart_server(port, server_ready, shutdown_rx).await
        }
    }

    // Arena 멀티파트 서버
    #[cfg(feature = "arena")]
    async fn run_arena_multipart_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();
        println!("✅ Arena 멀티파트 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    println!("🛑 멀티파트 서버 종료 신호 수신");
                    break;
                }

                accept_result = server.accept() => {
                    match accept_result {
                        Ok(accept) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_arena_multipart_request(accept).await {
                                    eprintln!("Arena 멀티파트 요청 처리 오류: {}", e);
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

    // 표준 멀티파트 서버
    #[cfg(not(feature = "arena"))]
    async fn run_standard_multipart_server(
        port: u16,
        server_ready: Arc<Notify>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), SendableError> {
        let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
        server_ready.notify_one();
        println!("✅ 표준 멀티파트 서버 실행 중 (포트: {})", port);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    println!("🛑 멀티파트 서버 종료 신호 수신");
                    break;
                }

                accept_result = server.accept() => {
                    match accept_result {
                        Ok(accept) => {
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_standard_multipart_request(accept).await {
                                    eprintln!("표준 멀티파트 요청 처리 오류: {}", e);
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

    // Arena 멀티파트 요청 처리
    #[cfg(feature = "arena")]
    async fn handle_arena_multipart_request(accept: Accept) -> Result<(), SendableError> {
        match accept.parse_request_arena_writer().await {
            Ok((request, mut response)) => {
                let start_time = Instant::now();
                let path = request.uri().path();

                match path {
                    "/" => {
                        let info = serde_json::json!({
                            "message": "🏗️ Arena 멀티파트 서버",
                            "version": "1.0.0",
                            "features": ["arena", "zero_copy_multipart"],
                            "endpoints": ["/upload", "/test/json"]
                        });
                        response.body_mut().set_arena_json(&info)?;
                        *response.status_mut() = StatusCode::OK;
                    }

                    "/upload" => {
                        match request.get_multi_part_arena() {
                            Ok(Some(form)) => {
                                let process_time = start_time.elapsed();

                                // 텍스트 필드 수집
                                let mut text_fields = HashMap::new();
                                for i in 0..form.text_fields.len() {
                                    if let (Some(name), Some(value)) =
                                        (form.get_text_field_name(i), form.get_text_field_value(i))
                                    {
                                        text_fields.insert(name.to_string(), value.to_string());
                                    }
                                }

                                // 파일 정보 수집
                                let files_info: Vec<serde_json::Value> = form.parts.iter().map(|part| {
                                    serde_json::json!({
                                        "name": part.get_name().unwrap_or("unknown"),
                                        "filename": part.get_file_name().unwrap_or(""),
                                        "size": part.get_body().len(),
                                        "content_type": part.get_content_type().unwrap_or("application/octet-stream")
                                    })
                                }).collect();

                                let total_size: usize =
                                    form.parts.iter().map(|p| p.get_body().len()).sum();

                                let response_data = serde_json::json!({
                                    "status": "success",
                                    "server_type": "arena",
                                    "text_fields": text_fields,
                                    "file_count": form.parts.len(),
                                    "files": files_info,
                                    "total_size_bytes": total_size,
                                    "processing_time_ms": process_time.as_millis(),
                                    "memory_info": "zero_copy_arena_allocated",
                                    "performance": {
                                        "memory_copies": 0,
                                        "string_allocations": 0,
                                        "direct_byte_access": true
                                    }
                                });

                                response.body_mut().set_arena_json(&response_data)?;
                                *response.status_mut() = StatusCode::OK;

                                // 파일 저장 (옵션)
                                for part in form.parts.iter() {
                                    if let Some(filename) = part.get_file_name() {
                                        if !filename.is_empty() {
                                            let save_path = format!("uploads/arena_{}", filename);
                                            if let Err(e) =
                                                tokio::fs::write(&save_path, part.get_body()).await
                                            {
                                                eprintln!("파일 저장 실패 {}: {}", save_path, e);
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                // JSON 데이터 처리
                                match request.get_json_arena::<MultipartTestData>() {
                                    Ok(data) => {
                                        let process_time = start_time.elapsed();
                                        let response_data = serde_json::json!({
                                            "status": "success",
                                            "server_type": "arena",
                                            "data_type": "json",
                                            "received_id": data.id,
                                            "data_size": data.description.len() + data.payload.len(),
                                            "processing_time_ms": process_time.as_millis(),
                                            "memory_info": "zero_copy_json_parsing"
                                        });
                                        response.body_mut().set_arena_json(&response_data)?;
                                        *response.status_mut() = StatusCode::OK;
                                    }
                                    Err(e) => {
                                        eprintln!("JSON 파싱 실패: {}", e);
                                        *response.status_mut() = StatusCode::BAD_REQUEST;
                                        response
                                            .body_mut()
                                            .set_arena_response("Invalid JSON data")?;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("멀티파트 파싱 실패: {}", e);
                                *response.status_mut() = StatusCode::BAD_REQUEST;
                                response
                                    .body_mut()
                                    .set_arena_response("Multipart parsing failed")?;
                            }
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

    // 표준 멀티파트 요청 처리
    #[cfg(not(feature = "arena"))]
    async fn handle_standard_multipart_request(accept: Accept) -> Result<(), SendableError> {
        match accept.parse_request().await {
            Ok((mut request, mut response)) => {
                let start_time = Instant::now();
                let path = request.uri().path();

                match path {
                    "/" => {
                        let info = serde_json::json!({
                            "message": "📝 표준 멀티파트 서버",
                            "version": "1.0.0",
                            "features": ["standard"],
                            "endpoints": ["/upload", "/test/json"]
                        });
                        response.body_mut().body = info.to_string();
                        *response.status_mut() = StatusCode::OK;
                    }

                    "/upload" => {
                        match request.get_multi_part().await {
                            Ok(Some(form)) => {
                                let process_time = start_time.elapsed();
                                let total_size: usize =
                                    form.parts.iter().map(|p| p.body.len()).sum();

                                let text_fields_map: serde_json::Map<String, serde_json::Value> =
                                    form.text_fields
                                        .iter()
                                        .map(|(k, v)| {
                                            (k.clone(), serde_json::Value::String(v.clone()))
                                        })
                                        .collect();
                                let response_data = serde_json::json!({
                                    "status": "success",
                                    "server_type": "standard",
                                    "text_fields": text_fields_map,
                                    "file_count": form.parts.len(),
                                    "files": form.parts.iter().map(|part| {
                                        serde_json::json!({
                                            "name": part.name,
                                            "filename": part.file_name,
                                            "size": part.body.len(),
                                            "content_type": part.headers.get("content-type")
                                                .map(|v| v.to_str().unwrap_or("unknown"))
                                                .unwrap_or("unknown")
                                        })
                                    }).collect::<Vec<_>>(),
                                    "total_size_bytes": total_size,
                                    "processing_time_ms": process_time.as_millis(),
                                    "memory_info": "heap_allocated_with_copies",
                                    "performance": {
                                        "memory_copies": "multiple",
                                        "string_allocations": "many",
                                        "direct_byte_access": false
                                    }
                                });

                                response.body_mut().body = response_data.to_string();
                                *response.status_mut() = StatusCode::OK;

                                // 파일 저장
                                for part in &form.parts {
                                    if !part.file_name.is_empty() {
                                        let save_path =
                                            format!("uploads/standard_{}", part.file_name);
                                        if let Err(e) =
                                            tokio::fs::write(&save_path, &part.body).await
                                        {
                                            eprintln!("파일 저장 실패 {}: {}", save_path, e);
                                        }
                                    }
                                }
                            }
                            Ok(None) => match request.get_json::<MultipartTestData>() {
                                Ok(data) => {
                                    let process_time = start_time.elapsed();
                                    let response_data = serde_json::json!({
                                        "status": "success",
                                        "server_type": "standard",
                                        "data_type": "json",
                                        "received_id": data.id,
                                        "data_size": data.description.len() + data.payload.len(),
                                        "processing_time_ms": process_time.as_millis(),
                                        "memory_info": "heap_allocated_json"
                                    });
                                    response.body_mut().body = response_data.to_string();
                                    *response.status_mut() = StatusCode::OK;
                                }
                                Err(e) => {
                                    eprintln!("JSON 파싱 실패: {}", e);
                                    *response.status_mut() = StatusCode::BAD_REQUEST;
                                }
                            },
                            Err(e) => {
                                eprintln!("멀티파트 파싱 실패: {}", e);
                                *response.status_mut() = StatusCode::BAD_REQUEST;
                            }
                        }
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

    // 모든 멀티파트 테스트 실행
    async fn run_all_multipart_tests(&mut self) {
        println!("\n🧪 멀티파트 테스트 실행 시작");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // 1. JSON 업로드 테스트
        self.test_json_upload().await;

        // 2. 소용량 파일 업로드
        self.test_small_file_upload().await;

        // 3. 중용량 파일 업로드
        self.test_medium_file_upload().await;

        // 4. 대용량 파일 업로드
        self.test_large_file_upload().await;

        // 5. 다중 파일 업로드
        self.test_multiple_file_upload().await;

        // 6. 극한 테스트
        self.test_extreme_upload().await;
    }

    // JSON 업로드 테스트
    async fn test_json_upload(&mut self) {
        println!("\n📄 JSON 업로드 테스트");

        let sizes = vec![1, 10, 100]; // KB
        for size_kb in sizes {
            match self.execute_json_upload_test(size_kb).await {
                Ok(result) => {
                    println!(
                        "  ✅ {}KB JSON: {:.1}ms",
                        size_kb,
                        result.processing_time.as_millis()
                    );
                    self.test_results.push(result);
                }
                Err(e) => {
                    println!("  ❌ {}KB JSON 실패: {}", size_kb, e);
                    self.test_results.push(MultipartTestResult {
                        test_name: format!("JSON {}KB", size_kb),
                        server_type: "unknown".to_string(),
                        file_count: 0,
                        total_size_mb: 0.0,
                        upload_time: Duration::from_millis(0),
                        processing_time: Duration::from_millis(0),
                        throughput_mbps: 0.0,
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    }

    async fn execute_json_upload_test(
        &self,
        size_kb: usize,
    ) -> Result<MultipartTestResult, SendableError> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/upload", self.port);
        let test_data = MultipartTestData::generate(size_kb);

        let start = Instant::now();
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&test_data)
            .send()
            .await?;

        let upload_time = start.elapsed();
        let body: serde_json::Value = response.json().await?;

        let processing_time = Duration::from_millis(
            body.get("processing_time_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        );

        let server_type = body
            .get("server_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let size_mb = (size_kb as f64) / 1024.0;
        let throughput_mbps = (size_mb * 8.0) / upload_time.as_secs_f64();

        Ok(MultipartTestResult {
            test_name: format!("JSON {}KB", size_kb),
            server_type,
            file_count: 0,
            total_size_mb: size_mb,
            upload_time,
            processing_time,
            throughput_mbps,
            success: true,
            error: None,
        })
    }

    // 소용량 파일 업로드 테스트
    async fn test_small_file_upload(&mut self) {
        println!("\n📎 소용량 파일 업로드 테스트");

        let file_sizes = vec![1024, 5120, 10240]; // 1KB, 5KB, 10KB

        match self
            .execute_multipart_upload_test("소용량 파일들", file_sizes)
            .await
        {
            Ok(result) => {
                println!("  ✅ 완료: {:.1}MB/s", result.throughput_mbps);
                self.test_results.push(result);
            }
            Err(e) => {
                println!("  ❌ 실패: {}", e);
            }
        }
    }

    // 중용량 파일 업로드 테스트
    async fn test_medium_file_upload(&mut self) {
        println!("\n📁 중용량 파일 업로드 테스트");

        let file_sizes = vec![102400, 204800, 512000]; // 100KB, 200KB, 500KB

        match self
            .execute_multipart_upload_test("중용량 파일들", file_sizes)
            .await
        {
            Ok(result) => {
                println!("  ✅ 완료: {:.1}MB/s", result.throughput_mbps);
                self.test_results.push(result);
            }
            Err(e) => {
                println!("  ❌ 실패: {}", e);
            }
        }
    }

    // 대용량 파일 업로드 테스트
    async fn test_large_file_upload(&mut self) {
        println!("\n🗂️ 대용량 파일 업로드 테스트");

        let file_sizes = vec![1048576, 2097152]; // 1MB, 2MB

        match self
            .execute_multipart_upload_test("대용량 파일들", file_sizes)
            .await
        {
            Ok(result) => {
                println!("  ✅ 완료: {:.1}MB/s", result.throughput_mbps);
                self.test_results.push(result);
            }
            Err(e) => {
                println!("  ❌ 실패: {}", e);
            }
        }
    }

    // 다중 파일 업로드 테스트
    async fn test_multiple_file_upload(&mut self) {
        println!("\n📚 다중 파일 업로드 테스트");

        let file_sizes = vec![524288; 5]; // 5x 512KB

        match self
            .execute_multipart_upload_test("다중 파일", file_sizes)
            .await
        {
            Ok(result) => {
                println!(
                    "  ✅ 완료: {}개 파일, {:.1}MB/s",
                    result.file_count, result.throughput_mbps
                );
                self.test_results.push(result);
            }
            Err(e) => {
                println!("  ❌ 실패: {}", e);
            }
        }
    }

    // 극한 테스트
    async fn test_extreme_upload(&mut self) {
        println!("\n🚀 극한 업로드 테스트");

        let file_sizes = vec![10485760]; // 10MB

        match self
            .execute_multipart_upload_test("극한 대용량", file_sizes)
            .await
        {
            Ok(result) => {
                println!(
                    "  ✅ 완료: {:.1}MB 파일, {:.1}MB/s",
                    result.total_size_mb, result.throughput_mbps
                );
                self.test_results.push(result);
            }
            Err(e) => {
                println!("  ❌ 실패: {}", e);
            }
        }
    }

    // 멀티파트 업로드 테스트 실행
    async fn execute_multipart_upload_test(
        &self,
        test_name: &str,
        file_sizes: Vec<usize>,
    ) -> Result<MultipartTestResult, SendableError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;

        let url = format!("http://127.0.0.1:{}/upload", self.port);

        let mut form = multipart::Form::new();

        // 텍스트 필드 추가
        form = form.text("description", format!("테스트: {}", test_name));
        form = form.text("user_id", "test_user_12345");
        form = form.text("timestamp", chrono::Utc::now().to_rfc3339());

        // 파일들 추가
        let total_size: usize = file_sizes.iter().sum();
        for (i, size) in file_sizes.iter().enumerate() {
            let file_data = vec![(i % 256) as u8; *size];
            let part = multipart::Part::bytes(file_data)
                .file_name(format!(
                    "test_file_{}_{}.bin",
                    test_name.replace(" ", "_"),
                    i
                ))
                .mime_str("application/octet-stream")?;
            form = form.part(format!("file_{}", i), part);
        }

        let start = Instant::now();
        let response = client.post(&url).multipart(form).send().await?;

        let upload_time = start.elapsed();
        let body: serde_json::Value = response.json().await?;

        let processing_time = Duration::from_millis(
            body.get("processing_time_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        );

        let server_type = body
            .get("server_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let file_count = file_sizes.len();
        let total_size_mb = (total_size as f64) / (1024.0 * 1024.0);
        let throughput_mbps = (total_size_mb * 8.0) / upload_time.as_secs_f64();

        Ok(MultipartTestResult {
            test_name: test_name.to_string(),
            server_type,
            file_count,
            total_size_mb,
            upload_time,
            processing_time,
            throughput_mbps,
            success: true,
            error: None,
        })
    }

    // 테스트 결과 출력
    fn print_multipart_test_results(&self) {
        println!("\n📊 멀티파트 테스트 결과 요약");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let successful = self.test_results.iter().filter(|r| r.success).count();
        let total = self.test_results.len();

        println!(
            "전체 테스트: {} / 성공: {} / 실패: {}",
            total,
            successful,
            total - successful
        );

        println!("\n📋 상세 결과:");
        println!(
            "{:<20} {:<10} {:<8} {:<12} {:<12} {:<12}",
            "테스트", "서버", "파일수", "크기(MB)", "업로드(ms)", "처리량(MB/s)"
        );
        println!("{:-<80}", "");

        for result in &self.test_results {
            let status = if result.success { "✅" } else { "❌" };
            if result.success {
                println!(
                    "{} {:<18} {:<10} {:<8} {:<12.2} {:<12.0} {:<12.1}",
                    status,
                    result.test_name,
                    result.server_type,
                    result.file_count,
                    result.total_size_mb,
                    result.upload_time.as_millis(),
                    result.throughput_mbps
                );
            } else {
                println!("{} {:<18} {:<10} - - - -", status, result.test_name, "실패");
                if let Some(error) = &result.error {
                    println!("     오류: {}", error);
                }
            }
        }

        // 성능 요약
        println!("\n🎯 성능 요약:");

        let successful_results: Vec<_> = self.test_results.iter().filter(|r| r.success).collect();
        if !successful_results.is_empty() {
            let avg_throughput: f64 = successful_results
                .iter()
                .map(|r| r.throughput_mbps)
                .sum::<f64>()
                / successful_results.len() as f64;

            let max_throughput = successful_results
                .iter()
                .map(|r| r.throughput_mbps)
                .fold(0.0f64, |a, b| a.max(b));

            println!("평균 처리량: {:.1} MB/s", avg_throughput);
            println!("최고 처리량: {:.1} MB/s", max_throughput);

            if let Some(server_type) = successful_results.first().map(|r| &r.server_type) {
                println!("서버 타입: {}", server_type);

                #[cfg(feature = "arena")]
                if server_type == "arena" {
                    println!("\n💡 Arena 서버의 멀티파트 장점:");
                    println!("  ✅ 제로카피: 파일 데이터를 복사하지 않고 직접 접근");
                    println!("  ✅ 메모리 효율성: Arena 할당으로 메모리 사용량 최적화");
                    println!("  ✅ 빠른 파싱: String 생성 없이 바이트 직접 처리");
                    println!("  ✅ 낮은 GC 압박: 메모리 할당/해제 부담 최소화");
                }
            }
        }

        if successful == total {
            println!("\n🏆 모든 멀티파트 테스트 통과!");
        } else {
            println!(
                "\n⚠️ 일부 멀티파트 테스트 실패 ({}/{})",
                total - successful,
                total
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    println!("🚀 통합 멀티파트 테스트 도구");

    #[cfg(feature = "arena")]
    println!("모드: Arena + Zero-copy 멀티파트");

    #[cfg(not(feature = "arena"))]
    println!("모드: 표준 멀티파트");

    let port = 9090;
    let mut test_manager = IntegratedMultipartTest::new(port);
    test_manager.run_integrated_multipart_test().await?;

    println!("\n✨ 통합 멀티파트 테스트 완료!");
    println!("\n💡 추가 테스트 실행:");
    println!("   Arena: cargo run --example integrated_multipart_test --features arena");
    println!("   표준:  cargo run --example integrated_multipart_test");
    println!("   릴리즈: cargo run --release --example integrated_multipart_test --features arena");

    Ok(())
}
