use atomic_http::TestData;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

// 표준 HTTP 파싱 벤치마크 (arena 피쳐 없이)
fn bench_standard_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("standard_parsing");

    for size in [1, 10, 100].iter() {
        let data = TestData::generate(*size);
        let json_bytes = serde_json::to_vec(&data).unwrap();

        group.bench_with_input(BenchmarkId::new("json_parsing", size), size, |b, _| {
            b.iter(|| {
                // 표준 Vec<u8> → String → JSON 파싱
                let json_string = String::from_utf8(json_bytes.clone()).unwrap();
                let _parsed: TestData = serde_json::from_str(&json_string).unwrap();
            })
        });

        group.bench_with_input(BenchmarkId::new("string_conversion", size), size, |b, _| {
            b.iter(|| black_box(String::from_utf8(json_bytes.clone()).unwrap()))
        });
    }

    group.finish();
}

// 아레나 파싱 벤치마크 (arena 피쳐 포함)
#[cfg(feature = "arena")]
fn bench_arena_parsing(c: &mut Criterion) {
    use bumpalo_herd::Herd;
    use std::sync::Arc;

    let mut group = c.benchmark_group("arena_parsing");

    for size in [1, 10, 100].iter() {
        let data = TestData::generate(*size);
        let json_bytes = serde_json::to_vec(&data).unwrap();

        group.bench_with_input(
            BenchmarkId::new("arena_json_parsing", size),
            size,
            |b, _| {
                b.iter(|| {
                    let herd = Arc::new(Herd::new());
                    let member = herd.get();

                    // 아레나에 할당
                    let allocated_data = member.alloc_slice_copy(&json_bytes);

                    // 직접 바이트에서 파싱 (제로카피)
                    let _parsed: TestData = serde_json::from_slice(allocated_data).unwrap();
                })
            },
        );

        group.bench_with_input(BenchmarkId::new("arena_allocation", size), size, |b, _| {
            b.iter(|| {
                let herd = Arc::new(Herd::new());
                let member = herd.get();
                black_box(member.alloc_slice_copy(&json_bytes));
            })
        });
    }

    group.finish();
}

// 메모리 할당 벤치마크
fn bench_memory_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_allocation");

    for size in [1, 10, 100].iter() {
        let size_bytes = size * 1024;
        let test_data = vec![0u8; size_bytes];

        group.bench_with_input(BenchmarkId::new("vec_clone", size), size, |b, _| {
            b.iter(|| black_box(test_data.clone()))
        });

        group.bench_with_input(BenchmarkId::new("string_conversion", size), size, |b, _| {
            b.iter(|| {
                let cloned = test_data.clone();
                black_box(String::from_utf8(cloned).unwrap_or_default())
            })
        });

        #[cfg(feature = "arena")]
        group.bench_with_input(BenchmarkId::new("arena_allocation", size), size, |b, _| {
            b.iter(|| {
                use bumpalo_herd::Herd;
                use std::sync::Arc;

                let herd = Arc::new(Herd::new());
                let member = herd.get();
                black_box(member.alloc_slice_copy(&test_data));
            })
        });
    }

    group.finish();
}

// 벤치마크 그룹 정의
#[cfg(feature = "arena")]
criterion_group!(
    benches,
    bench_standard_parsing,
    bench_arena_parsing,
    bench_memory_allocation
);

#[cfg(not(feature = "arena"))]
criterion_group!(benches, bench_standard_parsing, bench_memory_allocation);

criterion_main!(benches);
