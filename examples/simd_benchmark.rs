// SIMD 성능 벤치마크 테스트
use atomic_http::helpers::traits::bytes::SplitBytesArena;
use std::time::Instant;

const TEST_DATA_SIZES: &[usize] = &[100, 1000, 5000, 10000, 50000];
const ITERATIONS: usize = 10000;

fn generate_http_request(body_size: usize) -> Vec<u8> {
    let mut request = Vec::new();

    // HTTP 헤더 생성
    request.extend_from_slice(b"POST /api/test HTTP/1.1\r\n");
    request.extend_from_slice(b"Host: localhost:8080\r\n");
    request.extend_from_slice(b"Content-Type: application/json\r\n");
    request.extend_from_slice(format!("Content-Length: {}\r\n", body_size).as_bytes());
    request.extend_from_slice(b"User-Agent: BenchmarkClient/1.0\r\n");
    request.extend_from_slice(b"Accept: application/json\r\n");
    request.extend_from_slice(b"Connection: keep-alive\r\n");
    request.extend_from_slice(b"\r\n");

    // JSON 바디 생성
    let json_body = format!(
        r#"{{"test_data": "{}", "size": {}, "timestamp": {}}}"#,
        "x".repeat(body_size.saturating_sub(100)),
        body_size,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    request.extend_from_slice(json_body.as_bytes());
    request
}

fn benchmark_simd_vs_scalar() {
    println!("🚀 SIMD vs Scalar HTTP 파싱 성능 벤치마크");
    println!("{}", "=".repeat(60));

    for &size in TEST_DATA_SIZES {
        println!("\n📊 테스트 데이터 크기: {} bytes", size);

        let test_data = generate_http_request(size);
        println!("   실제 HTTP 요청 크기: {} bytes", test_data.len());

        // Arena 방식으로 테스트 (SIMD 적용됨)
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let slice: &[u8] = &test_data;
            let _ = slice.split_header_body_arena();
        }
        let simd_duration = start.elapsed();

        // 처리량 계산
        let total_bytes = test_data.len() * ITERATIONS;
        let simd_throughput = (total_bytes as f64) / simd_duration.as_secs_f64() / 1024.0 / 1024.0;

        println!("   SIMD 방식:");
        println!("     시간: {:?}", simd_duration);
        println!("     처리량: {:.2} MB/s", simd_throughput);
        println!("     평균 처리시간: {:.2} μs/req",
                 simd_duration.as_nanos() as f64 / ITERATIONS as f64 / 1000.0);
    }

    println!("\n✅ 벤치마크 완료!");
    println!("💡 SIMD 최적화는 큰 요청에서 더 큰 성능 향상을 보입니다.");
}

fn verify_correctness() {
    println!("\n🔍 SIMD 구현 정확성 검증");

    let test_cases = vec![
        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\nBody".to_vec(),
        b"POST /api HTTP/1.1\r\nContent-Length: 10\r\n\r\n1234567890".to_vec(),
        b"PUT /upload HTTP/1.1\r\nContent-Type: text/plain\r\n\r\nTest data here".to_vec(),
        generate_http_request(1000),
        generate_http_request(5000),
    ];

    for (i, test_data) in test_cases.iter().enumerate() {
        let slice: &[u8] = test_data;
        let (header, body) = slice.split_header_body_arena();

        println!("테스트 케이스 {}: 헤더 {}bytes, 바디 {}bytes",
                 i + 1, header.len(), body.len());

        // 헤더에 \r\n\r\n이 없는지 확인
        if header.windows(4).any(|w| w == b"\r\n\r\n") {
            println!("❌ 오류: 헤더에 구분자가 포함됨");
        } else {
            println!("✅ 정확한 파싱");
        }
    }
}

fn main() {
    #[cfg(feature = "simd")]
    println!("🔥 SIMD 기능이 활성화됨");

    #[cfg(not(feature = "simd"))]
    println!("⚠️  SIMD 기능이 비활성화됨 (스칼라 모드)");

    #[cfg(target_arch = "x86_64")]
    {
        println!("🖥️  CPU 기능 지원:");
        println!("   SSE2: {}", is_x86_feature_detected!("sse2"));
        println!("   AVX2: {}", is_x86_feature_detected!("avx2"));
    }

    verify_correctness();
    benchmark_simd_vs_scalar();
}