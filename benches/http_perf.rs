//! 0.11 → 0.12 성능 개선 측정용 마이크로벤치.
//! 공개 API로 측정 가능한 두 경로에 집중:
//!   1. 멀티파트 파싱 (task 1, 11)
//!   2. ZeroCopyCache 히트 (task 4)
//!
//! Run: `cargo bench --bench http_perf --features arena -- --quick`

use atomic_http::{Body, RequestUtils, ZeroCopyCache};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use http::Request;
use std::hint::black_box;

// ────────────────────────────────────────────────────────────────
// 1) Multipart parsing benchmark
// ────────────────────────────────────────────────────────────────

fn build_multipart_body(num_parts: usize, body_size: usize) -> Vec<u8> {
    let boundary = "----WebKitFormBoundaryABCDEFG12345";
    let mut buf = Vec::with_capacity(num_parts * (body_size + 256));
    for i in 0..num_parts {
        buf.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        if i == 0 {
            buf.extend_from_slice(
                b"Content-Disposition: form-data; name=\"upload\"; filename=\"test.bin\"\r\n",
            );
            buf.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
            buf.extend(std::iter::repeat(b'A').take(body_size));
            buf.extend_from_slice(b"\r\n");
        } else {
            buf.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"field{}\"\r\n\r\n", i).as_bytes(),
            );
            buf.extend(std::iter::repeat(b'x').take(body_size));
            buf.extend_from_slice(b"\r\n");
        }
    }
    buf.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    buf
}

fn build_request(body: Vec<u8>) -> Request<Body> {
    let len = body.len();
    Request::builder()
        .header(
            "content-type",
            "multipart/form-data; boundary=----WebKitFormBoundaryABCDEFG12345",
        )
        .body(Body {
            bytes: body,
            body: String::new(),
            len,
            ip: None,
        })
        .unwrap()
}

fn bench_multipart(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("multipart_parse");
    group.sample_size(30);

    for &(parts, size) in &[(2usize, 256usize), (8, 1024), (16, 4096)] {
        let label = format!("{}parts_{}B", parts, size);
        group.bench_with_input(BenchmarkId::from_parameter(&label), &(parts, size), |b, &(p, s)| {
            let body_bytes = build_multipart_body(p, s);
            b.iter(|| {
                let mut req = build_request(body_bytes.clone());
                let form = rt.block_on(req.get_multi_part()).unwrap();
                black_box(form);
            });
        });
    }
    group.finish();
}

// ────────────────────────────────────────────────────────────────
// 2) ZeroCopyCache hit benchmark
// ────────────────────────────────────────────────────────────────

fn prepare_cache_file(size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("atomic_http_bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("cache_{}.bin", size));
    if !path.exists() || std::fs::metadata(&path).unwrap().len() as usize != size {
        std::fs::write(&path, vec![0u8; size]).unwrap();
    }
    path
}

fn bench_cache_hit(c: &mut Criterion) {
    let cache = ZeroCopyCache::global();
    let mut group = c.benchmark_group("cache_hit");
    group.sample_size(50);

    for &size in &[4 * 1024usize, 64 * 1024, 256 * 1024, 1024 * 1024] {
        let path = prepare_cache_file(size);
        // 첫 번째 로드로 캐시 워밍업
        let _ = cache.load_file(&path).unwrap();

        group.bench_with_input(BenchmarkId::from_parameter(size), &path, |b, p| {
            b.iter(|| {
                let result = cache.load_file(p).unwrap();
                black_box(result.as_bytes().len());
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_multipart, bench_cache_hit);
criterion_main!(benches);
