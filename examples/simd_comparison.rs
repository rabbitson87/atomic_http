// SIMD vs 스칼라 직접 비교 벤치마크
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
    request.extend_from_slice(b"Authorization: Bearer token123456789\r\n");
    request.extend_from_slice(b"X-Request-ID: req-123456789\r\n");
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

// 스칼라 구현 (SIMD 없이)
fn find_header_end_scalar(data: &[u8]) -> Option<usize> {
    let pattern = b"\r\n\r\n";
    for i in 0..=data.len().saturating_sub(4) {
        if &data[i..i + 4] == pattern {
            return Some(i);
        }
    }
    None
}

fn split_header_body_scalar(data: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = find_header_end_scalar(data) {
        let header = &data[..pos];
        let body = &data[pos + 4..];
        return (header, body);
    }
    (data, &[])
}

fn benchmark_comparison() {
    println!("🚀 SIMD vs 스칼라 HTTP 파싱 성능 직접 비교");
    println!("{}", "=".repeat(70));

    #[cfg(target_arch = "x86_64")]
    {
        println!("🖥️  CPU 기능 지원:");
        println!("   SSE2: {}", is_x86_feature_detected!("sse2"));
        println!("   AVX2: {}", is_x86_feature_detected!("avx2"));
    }

    for &size in TEST_DATA_SIZES {
        println!("\n📊 테스트 데이터 크기: {} bytes", size);

        let test_data = generate_http_request(size);
        println!("   실제 HTTP 요청 크기: {} bytes", test_data.len());

        // SIMD 방식 테스트 (현재 구현)
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let slice: &[u8] = &test_data;
            let _ = slice.split_header_body_arena();
        }
        let simd_duration = start.elapsed();

        // 스칼라 방식 테스트 (순수 루프)
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            let _ = split_header_body_scalar(&test_data);
        }
        let scalar_duration = start.elapsed();

        // 성능 계산
        let total_bytes = test_data.len() * ITERATIONS;
        let simd_throughput = (total_bytes as f64) / simd_duration.as_secs_f64() / 1024.0 / 1024.0;
        let scalar_throughput = (total_bytes as f64) / scalar_duration.as_secs_f64() / 1024.0 / 1024.0;

        let speedup = scalar_duration.as_nanos() as f64 / simd_duration.as_nanos() as f64;

        println!("   📈 SIMD 방식:");
        println!("     시간: {:?}", simd_duration);
        println!("     처리량: {:.2} MB/s", simd_throughput);
        println!("     평균 처리시간: {:.2} ns/req", simd_duration.as_nanos() as f64 / ITERATIONS as f64);

        println!("   📊 스칼라 방식:");
        println!("     시간: {:?}", scalar_duration);
        println!("     처리량: {:.2} MB/s", scalar_throughput);
        println!("     평균 처리시간: {:.2} ns/req", scalar_duration.as_nanos() as f64 / ITERATIONS as f64);

        println!("   🏆 성능 개선:");
        if speedup > 1.0 {
            println!("     SIMD가 {:.2}x 더 빠름 ({:.1}% 개선)", speedup, (speedup - 1.0) * 100.0);
        } else {
            println!("     스칼라가 {:.2}x 더 빠름", 1.0 / speedup);
        }
        println!("     처리량 개선: {:.1}%", ((simd_throughput / scalar_throughput) - 1.0) * 100.0);
    }

    println!("\n✅ 비교 벤치마크 완료!");
}

fn verify_correctness() {
    println!("\n🔍 SIMD vs 스칼라 결과 일치성 검증");

    let test_cases = vec![
        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\nBody".to_vec(),
        b"POST /api HTTP/1.1\r\nContent-Length: 10\r\n\r\n1234567890".to_vec(),
        generate_http_request(1000),
        generate_http_request(5000),
        generate_http_request(10000),
    ];

    for (i, test_data) in test_cases.iter().enumerate() {
        let slice: &[u8] = test_data;

        // SIMD 방식
        let (simd_header, simd_body) = slice.split_header_body_arena();

        // 스칼라 방식
        let (scalar_header, scalar_body) = split_header_body_scalar(test_data);

        print!("테스트 케이스 {}: ", i + 1);

        if simd_header == scalar_header && simd_body == scalar_body {
            println!("✅ 일치 (헤더: {}bytes, 바디: {}bytes)", simd_header.len(), simd_body.len());
        } else {
            println!("❌ 불일치!");
            println!("   SIMD - 헤더: {}bytes, 바디: {}bytes", simd_header.len(), simd_body.len());
            println!("   스칼라 - 헤더: {}bytes, 바디: {}bytes", scalar_header.len(), scalar_body.len());
        }
    }
}

fn benchmark_patterns() {
    println!("\n🔍 다양한 패턴에서의 성능 비교");

    let test_patterns = vec![
        ("짧은 헤더", b"GET / HTTP/1.1\r\n\r\nshort".to_vec()),
        ("긴 헤더", {
            let mut data = Vec::new();
            data.extend_from_slice(b"POST /api HTTP/1.1\r\n");
            for i in 0..20 {
                data.extend_from_slice(format!("X-Custom-Header-{}: value{}\r\n", i, i).as_bytes());
            }
            data.extend_from_slice(b"\r\nbody");
            data
        }),
        ("패턴이 끝에 있는 경우", {
            let mut data = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n".to_vec();
            data.extend(vec![b'x'; 2000]);
            data.extend_from_slice(b"\r\n\r\nbody");
            data
        }),
    ];

    for (name, test_data) in test_patterns {
        println!("\n📋 {}: {} bytes", name, test_data.len());

        const PATTERN_ITERATIONS: usize = 100000;

        // SIMD
        let start = Instant::now();
        for _ in 0..PATTERN_ITERATIONS {
            let slice: &[u8] = &test_data;
            let _ = slice.split_header_body_arena();
        }
        let simd_time = start.elapsed();

        // 스칼라
        let start = Instant::now();
        for _ in 0..PATTERN_ITERATIONS {
            let _ = split_header_body_scalar(&test_data);
        }
        let scalar_time = start.elapsed();

        let speedup = scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64;

        println!("   SIMD: {:?} | 스칼라: {:?} | 개선: {:.2}x",
                 simd_time, scalar_time, speedup);
    }
}

fn main() {
    #[cfg(feature = "simd")]
    println!("🔥 SIMD 기능이 활성화됨");

    #[cfg(not(feature = "simd"))]
    println!("⚠️  SIMD 기능이 비활성화됨");

    verify_correctness();
    benchmark_comparison();
    benchmark_patterns();
}