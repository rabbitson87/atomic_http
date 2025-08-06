use atomic_http::*;
use clap::{Arg, Command};
use http::StatusCode;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// 테스트용 JSON 파일들 생성
async fn create_test_json_files() -> Result<(), SendableError> {
    let test_dir = Path::new("test_json_files");
    if !test_dir.exists() {
        std::fs::create_dir(test_dir)?;
    }

    // 다양한 크기의 JSON 파일 생성
    let test_files = vec![
        ("small_data.json", TestData::generate(1)),     // 1KB
        ("medium_data.json", TestData::generate(100)),  // 100KB
        ("large_data.json", TestData::generate(1000)),  // 1MB
        ("xlarge_data.json", TestData::generate(5000)), // 5MB
    ];

    for (filename, data) in test_files {
        let filepath = test_dir.join(filename);
        if !filepath.exists() {
            data.save_to_file(&filepath)?;
            println!("✅ 테스트 JSON 파일 생성: {}", filename);
        }
    }

    // 설정 파일도 생성
    let config = json!({
        "server": {
            "port": 8080,
            "zero_copy_enabled": true,
            "cache_size": 100,
            "performance_mode": "optimized"
        },
        "features": {
            "arena": cfg!(feature = "arena"),
            "response_file": cfg!(feature = "response_file")
        }
    });

    let config_path = test_dir.join("server_config.json");
    std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;

    Ok(())
}

// Arena + 제로카피 서버
#[cfg(all(feature = "arena", feature = "response_file"))]
async fn run_zero_copy_server(port: u16) -> Result<(), SendableError> {
    println!("🚀 Arena + Zero-copy 서버 시작 (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
    println!("✅ 하이브리드 서버 실행 중! Arena + memmap2 제로카피");

    let request_count = Arc::new(AtomicUsize::new(0));

    loop {
        let (stream, options, herd) = server.accept().await?;
        let req_count = request_count.clone();

        tokio::spawn(async move {
            let req_num = req_count.fetch_add(1, Ordering::Relaxed) + 1;

            match Server::parse_request_arena_writer(stream, options, herd).await {
                Ok((request, mut response)) => {
                    let start_time = Instant::now();
                    let path = request.uri().path();

                    match path {
                        "/" => {
                            let welcome_data = json!({
                                "message": "🚀 Arena + Hybrid Zero-copy HTTP Server",
                                "features": {
                                    "arena_memory": true,
                                    "hybrid_file_cache": true,
                                    "memory_cache_for_small_files": true,
                                    "mmap_for_large_files": true,
                                    "json_parsing": true,
                                    "no_file_handle_leaks": true
                                },
                                "cache_strategy": {
                                    "small_files": "메모리 캐시 (≤1MB)",
                                    "large_files": "직접 memmap2 (>1MB)",
                                    "advantages": "파일 핸들 누수 없음, 예측 가능한 메모리 사용"
                                },
                                "endpoints": {
                                    "/": "서버 정보",
                                    "/json/<filename>": "하이브리드 제로카피 JSON 파일 서빙",
                                    "/test/json": "JSON 파싱 테스트",
                                    "/test/performance": "성능 비교 테스트",
                                    "/stats": "캐시 통계"
                                },
                                "request_number": req_num,
                                "processing_time_ms": start_time.elapsed().as_millis()
                            });

                            response.body_mut().set_arena_json(&welcome_data).unwrap();
                            *response.status_mut() = StatusCode::OK;
                        }

                        path if path.starts_with("/json/") => {
                            let filename = &path[6..]; // "/json/" 제거
                            let file_path = format!("test_json_files/{}", filename);

                            if Path::new(&file_path).exists() {
                                // 제로카피 파일 서빙 사용
                                if let Err(e) = response.body_mut().response_file(&file_path) {
                                    eprintln!("파일 응답 설정 실패: {}", e);
                                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                                } else {
                                    *response.status_mut() = StatusCode::OK;
                                    println!(
                                        "📁 Zero-copy 파일 서빙: {} (요청 #{})",
                                        filename, req_num
                                    );
                                }
                            } else {
                                *response.status_mut() = StatusCode::NOT_FOUND;
                                let _ = response
                                    .body_mut()
                                    .set_arena_response("JSON file not found");
                            }
                        }

                        "/test/json" => {
                            // JSON 파싱 성능 테스트
                            match request.get_json_arena::<TestData>() {
                                Ok(data) => {
                                    let process_time = start_time.elapsed();
                                    let response_data = json!({
                                        "status": "success",
                                        "server_type": "arena_zero_copy",
                                        "received_id": data.id,
                                        "data_size_bytes": data.description.len() + data.payload.len(),
                                        "processing_time_ms": process_time.as_millis(),
                                        "memory_info": "arena_allocated_zero_copy_parsing",
                                        "request_number": req_num,
                                        "performance": {
                                            "memory_copies": 0,
                                            "string_allocations": "minimal",
                                            "direct_byte_access": true
                                        }
                                    });
                                    response.body_mut().set_arena_json(&response_data).unwrap();
                                    *response.status_mut() = StatusCode::OK;

                                    println!(
                                        "🧪 JSON 파싱 완료: {}KB, {:.2}ms (요청 #{})",
                                        (data.description.len() + data.payload.len()) / 1024,
                                        process_time.as_millis(),
                                        req_num
                                    );
                                }
                                Err(e) => {
                                    eprintln!("JSON 파싱 실패: {}", e);
                                    *response.status_mut() = StatusCode::BAD_REQUEST;
                                    response
                                        .body_mut()
                                        .set_arena_response("Invalid JSON data")
                                        .unwrap();
                                }
                            }
                        }

                        "/test/performance" => {
                            // 성능 테스트 결과 제공
                            let file_tests = vec![
                                ("small_data.json", "1KB"),
                                ("medium_data.json", "100KB"),
                                ("large_data.json", "1MB"),
                                ("xlarge_data.json", "5MB"),
                            ];

                            let mut results = Vec::new();
                            for (filename, size) in file_tests {
                                let file_path = format!("test_json_files/{}", filename);
                                if Path::new(&file_path).exists() {
                                    let file_start = Instant::now();

                                    match parse_json_file::<TestData, _>(&file_path) {
                                        Ok(data) => {
                                            let parse_time = file_start.elapsed();
                                            results.push(json!({
                                                "file": filename,
                                                "size": size,
                                                "parse_time_ms": parse_time.as_millis(),
                                                "method": "hybrid_cache",
                                                "success": true,
                                                "data_id": data.id
                                            }));
                                        }
                                        Err(e) => {
                                            results.push(json!({
                                                "file": filename,
                                                "size": size,
                                                "error": e.to_string(),
                                                "method": "hybrid_cache",
                                                "success": false
                                            }));
                                        }
                                    }
                                }
                            }

                            let response_data = json!({
                                "status": "performance_test_complete",
                                "server_type": "arena_zero_copy_hybrid",
                                "test_results": results,
                                "total_time_ms": start_time.elapsed().as_millis(),
                                "request_number": req_num,
                                "features_active": {
                                    "arena": true,
                                    "memory_cache": true,
                                    "mmap_for_large_files": true
                                }
                            });

                            response.body_mut().set_arena_json(&response_data).unwrap();
                            *response.status_mut() = StatusCode::OK;
                        }

                        "/stats" => {
                            // 캐시 통계 제공
                            let response_data = json!({
                                "status": "cache_stats",
                                "server_type": "arena_zero_copy_hybrid",
                                "total_requests": req_num,
                                "uptime_info": "서버 실행 중",
                                "memory_info": {
                                    "arena_allocations": "efficient",
                                    "memory_cache": "active_for_small_files",
                                    "mmap_for_large_files": "active",
                                    "file_cache_strategy": "hybrid"
                                },
                                "cache_explanation": {
                                    "small_files": "메모리에 완전히 로드하여 캐시 (1MB 이하)",
                                    "large_files": "필요시 memmap2로 직접 접근 (1MB 초과)",
                                    "advantages": [
                                        "파일 핸들을 계속 열어두지 않음",
                                        "작은 파일은 빠른 메모리 액세스",
                                        "큰 파일은 제로카피 메모리 매핑",
                                        "메모리 사용량 예측 가능"
                                    ]
                                },
                                "request_number": req_num,
                                "processing_time_ms": start_time.elapsed().as_millis()
                            });

                            response.body_mut().set_arena_json(&response_data).unwrap();
                            *response.status_mut() = StatusCode::OK;
                        }

                        _ => {
                            *response.status_mut() = StatusCode::NOT_FOUND;
                            let _ = response
                                .body_mut()
                                .set_arena_response("페이지를 찾을 수 없습니다");
                        }
                    }

                    if let Err(e) = response.responser_arena().await {
                        eprintln!("응답 전송 실패: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("요청 파싱 실패: {}", e);
                }
            }
        });
    }
}

// 표준 서버 (비교용)
#[cfg(not(all(feature = "arena")))]
async fn run_standard_server(port: u16) -> Result<(), SendableError> {
    println!("🚀 표준 HTTP 서버 시작 (포트: {})", port);
    let mut server = Server::new(&format!("127.0.0.1:{}", port)).await?;
    println!("✅ 표준 서버 실행 중");

    let request_count = Arc::new(AtomicUsize::new(0));

    loop {
        let (stream, options) = server.accept().await?;
        let req_count = request_count.clone();

        tokio::spawn(async move {
            let req_num = req_count.fetch_add(1, Ordering::Relaxed) + 1;

            match Server::parse_request(stream, options).await {
                Ok((mut request, mut response)) => {
                    let start_time = Instant::now();
                    let path = request.uri().path();

                    match path {
                        "/" => {
                            let welcome_data = json!({
                                "message": "📝 Standard HTTP Server",
                                "features": {
                                    "arena_memory": false,
                                    "zero_copy_files": false,
                                    "standard_parsing": true
                                },
                                "request_number": req_num,
                                "processing_time_ms": start_time.elapsed().as_millis()
                            });

                            response.body_mut().body = welcome_data.to_string();
                            *response.status_mut() = StatusCode::OK;
                        }

                        "/test/json" => match request.get_json::<TestData>() {
                            Ok(data) => {
                                let process_time = start_time.elapsed();
                                let response_data = json!({
                                    "status": "success",
                                    "server_type": "standard",
                                    "received_id": data.id,
                                    "data_size_bytes": data.description.len() + data.payload.len(),
                                    "processing_time_ms": process_time.as_millis(),
                                    "memory_info": "heap_allocated_with_copies",
                                    "request_number": req_num
                                });
                                response.body_mut().body = response_data.to_string();
                                *response.status_mut() = StatusCode::OK;
                            }
                            Err(e) => {
                                eprintln!("JSON 파싱 실패: {}", e);
                                *response.status_mut() = StatusCode::BAD_REQUEST;
                            }
                        },

                        _ => {
                            *response.status_mut() = StatusCode::NOT_FOUND;
                            response.body_mut().body = "페이지를 찾을 수 없습니다".to_string();
                        }
                    }

                    if let Err(e) = response.responser().await {
                        eprintln!("응답 전송 실패: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("요청 파싱 실패: {}", e);
                }
            }
        });
    }
}

// 클라이언트 테스트
async fn run_client_tests(port: u16) -> Result<(), SendableError> {
    println!("🧪 제로카피 기능 테스트 시작");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let base_url = format!("http://127.0.0.1:{}", port);

    // 서버 연결 확인
    println!("\n🔍 서버 연결 확인...");
    match client.get(&base_url).send().await {
        Ok(response) => {
            let body: serde_json::Value = response.json().await?;
            println!("✅ 서버 연결 성공!");
            println!("📊 서버 정보: {}", serde_json::to_string_pretty(&body)?);
        }
        Err(e) => {
            println!("❌ 서버 연결 실패: {}", e);
            return Ok(());
        }
    }

    // JSON 파일 다운로드 테스트
    println!("\n📁 제로카피 파일 다운로드 테스트");
    let test_files = vec!["small_data.json", "medium_data.json", "large_data.json"];

    for filename in test_files {
        let start = Instant::now();
        let url = format!("{}/json/{}", base_url, filename);

        match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let size = response.content_length().unwrap_or(0);
                    let _content = response.text().await?;
                    let duration = start.elapsed();

                    println!(
                        "  ✅ {}: {}KB, {:.2}ms",
                        filename,
                        size / 1024,
                        duration.as_millis()
                    );
                } else {
                    println!("  ❌ {}: HTTP {}", filename, response.status());
                }
            }
            Err(e) => {
                println!("  ❌ {}: {}", filename, e);
            }
        }
    }

    // JSON 파싱 성능 테스트
    println!("\n🧪 JSON 파싱 성능 테스트");
    let test_sizes = vec![10, 100, 1000]; // KB

    for size_kb in test_sizes {
        let test_data = TestData::generate(size_kb);
        let start = Instant::now();

        match client
            .post(&format!("{}/test/json", base_url))
            .header("Content-Type", "application/json")
            .json(&test_data)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let result: serde_json::Value = response.json().await?;
                    let server_time = result["processing_time_ms"].as_u64().unwrap_or(0);
                    let total_time = start.elapsed();

                    println!(
                        "  ✅ {}KB JSON: 서버 {}ms, 총 {}ms",
                        size_kb,
                        server_time,
                        total_time.as_millis()
                    );
                } else {
                    println!("  ❌ {}KB JSON: HTTP {}", size_kb, response.status());
                }
            }
            Err(e) => {
                println!("  ❌ {}KB JSON: {}", size_kb, e);
            }
        }
    }

    // 성능 비교 테스트
    println!("\n⚡ 성능 비교 테스트");
    match client
        .get(&format!("{}/test/performance", base_url))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                let result: serde_json::Value = response.json().await?;
                println!("📊 성능 테스트 결과:");
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        Err(e) => {
            println!("❌ 성능 테스트 실패: {}", e);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let matches = Command::new("Zero-Copy HTTP Test")
        .version("1.0")
        .about("제로카피 HTTP 서버 기능 테스트")
        .arg(
            Arg::new("mode")
                .short('m')
                .long("mode")
                .value_name("MODE")
                .help("실행 모드: server 또는 client")
                .default_value("server"),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("서버 포트")
                .default_value("8080"),
        )
        .get_matches();

    let mode = matches.get_one::<String>("mode").unwrap();
    let port: u16 = matches.get_one::<String>("port").unwrap().parse()?;

    match mode.as_str() {
        "server" => {
            // 테스트 파일 생성
            create_test_json_files().await?;

            #[cfg(all(feature = "arena", feature = "response_file"))]
            {
                println!("🏗️  Arena + Zero-copy 모드로 서버 시작");
                run_zero_copy_server(port).await?;
            }

            #[cfg(not(all(feature = "arena")))]
            {
                println!("📝 표준 HTTP 모드로 서버 시작");
                run_standard_server(port).await?;
            }
        }
        "client" => {
            run_client_tests(port).await?;
        }
        _ => {
            println!("❌ 올바르지 않은 모드입니다. 'server' 또는 'client'를 사용하세요.");
        }
    }

    Ok(())
}
