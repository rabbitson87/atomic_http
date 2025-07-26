use atomic_http::TestData;
use clap::{Arg, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

struct LoadTester {
    client: reqwest::Client,
    base_url: String,
    concurrency: usize,
    total_requests: usize,
    payload_size: usize,
}

impl LoadTester {
    fn new(
        base_url: String,
        concurrency: usize,
        total_requests: usize,
        payload_size: usize,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        Self {
            client,
            base_url,
            concurrency,
            total_requests,
            payload_size,
        }
    }

    async fn run_test(&self) -> (usize, usize, Duration, Vec<Duration>) {
        println!("🚀 부하 테스트 시작:");
        println!("   URL: {}", self.base_url);
        println!("   총 요청 수: {}", self.total_requests);
        println!("   동시 연결 수: {}", self.concurrency);
        println!("   페이로드 크기: {} KB", self.payload_size);

        let semaphore = Arc::new(Semaphore::new(self.concurrency));
        let successful_requests = Arc::new(AtomicUsize::new(0));
        let failed_requests = Arc::new(AtomicUsize::new(0));
        let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let start_time = Instant::now();
        let mut handles = vec![];

        for _i in 0..self.total_requests {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = self.client.clone();
            let url = self.base_url.clone();
            let successful = successful_requests.clone();
            let failed = failed_requests.clone();
            let latencies_clone = latencies.clone();

            // TestData::generate를 사용하여 테스트 데이터 생성
            let payload = TestData::generate(self.payload_size);

            let handle = tokio::spawn(async move {
                let _permit = permit;

                let request_start = Instant::now();
                let result = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&payload)
                    .send()
                    .await;

                let latency = request_start.elapsed();

                match result {
                    Ok(response) => {
                        if response.status().is_success() {
                            successful.fetch_add(1, Ordering::Relaxed);
                        } else {
                            println!("HTTP 오류: {}", response.status());
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        println!("요청 실패: {}", e);
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }

                let mut latencies_guard = latencies_clone.lock().await;
                latencies_guard.push(latency);
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let total_duration = start_time.elapsed();
        let successful = successful_requests.load(Ordering::Relaxed);
        let failed = failed_requests.load(Ordering::Relaxed);
        let latencies_vec = latencies.lock().await.clone();

        (successful, failed, total_duration, latencies_vec)
    }
}

#[tokio::main]
async fn main() {
    let matches = Command::new("Load Test Client")
        .version("1.0")
        .about("HTTP 서버 부하 테스트 도구")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("서버 포트")
                .default_value("9080"),
        )
        .arg(
            Arg::new("requests")
                .short('n')
                .long("requests")
                .value_name("NUMBER")
                .help("총 요청 수")
                .default_value("1000"),
        )
        .arg(
            Arg::new("concurrency")
                .short('c')
                .long("concurrency")
                .value_name("NUMBER")
                .help("동시 연결 수")
                .default_value("50"),
        )
        .arg(
            Arg::new("size")
                .short('s')
                .long("size")
                .value_name("KB")
                .help("페이로드 크기 (KB)")
                .default_value("10"),
        )
        .get_matches();

    let port: u16 = matches.get_one::<String>("port").unwrap().parse().unwrap();
    let requests: usize = matches
        .get_one::<String>("requests")
        .unwrap()
        .parse()
        .unwrap();
    let concurrency: usize = matches
        .get_one::<String>("concurrency")
        .unwrap()
        .parse()
        .unwrap();
    let payload_size: usize = matches.get_one::<String>("size").unwrap().parse().unwrap();

    println!("🔧 테스트 설정:");
    println!("   포트: {}", port);
    println!("   요청 수: {}", requests);
    println!("   동시성: {}", concurrency);
    println!("   페이로드 크기: {} KB", payload_size);

    let url = format!("http://127.0.0.1:{}/test", port);
    let tester = LoadTester::new(url, concurrency, requests, payload_size);

    // 서버 연결 테스트
    println!("\n🔍 서버 연결 확인 중...");
    let test_data = TestData::generate(1);
    let client = reqwest::Client::new();
    match client
        .post(&format!("http://127.0.0.1:{}/test", port))
        .header("Content-Type", "application/json")
        .json(&test_data)
        .send()
        .await
    {
        Ok(response) => {
            println!("✅ 서버 연결 성공! 상태: {}", response.status());
        }
        Err(e) => {
            println!("❌ 서버 연결 실패: {}", e);
            println!("서버가 실행 중인지 확인하세요.");
            return;
        }
    }

    let (successful, failed, duration, mut latencies) = tester.run_test().await;

    // 결과 분석
    latencies.sort();
    let avg_latency = if !latencies.is_empty() {
        latencies.iter().sum::<Duration>() / latencies.len() as u32
    } else {
        Duration::from_millis(0)
    };

    let requests_per_second = successful as f64 / duration.as_secs_f64();

    println!("\n📈 테스트 결과:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "성공 요청:     {} ({:.1}%)",
        successful,
        (successful as f64 / requests as f64) * 100.0
    );
    println!(
        "실패 요청:     {} ({:.1}%)",
        failed,
        (failed as f64 / requests as f64) * 100.0
    );
    println!("평균 지연시간: {:.2}ms", avg_latency.as_millis());
    println!("처리량:        {:.1} req/sec", requests_per_second);
    println!("총 소요시간:   {:.2}초", duration.as_secs_f64());

    if !latencies.is_empty() {
        let p50_index = (latencies.len() as f64 * 0.50) as usize;
        let p95_index = (latencies.len() as f64 * 0.95) as usize;
        let p99_index = (latencies.len() as f64 * 0.99) as usize;

        println!(
            "50th percentile: {:.2}ms",
            latencies[p50_index.min(latencies.len() - 1)].as_millis()
        );
        println!(
            "95th percentile: {:.2}ms",
            latencies[p95_index.min(latencies.len() - 1)].as_millis()
        );
        println!(
            "99th percentile: {:.2}ms",
            latencies[p99_index.min(latencies.len() - 1)].as_millis()
        );
        println!("최소 지연시간:  {:.2}ms", latencies[0].as_millis());
        println!(
            "최대 지연시간:  {:.2}ms",
            latencies[latencies.len() - 1].as_millis()
        );
    }

    // 성능 등급 평가
    println!("\n🏆 성능 평가:");
    match requests_per_second {
        rps if rps > 1000.0 => println!("🟢 우수: {} req/sec", rps.round()),
        rps if rps > 500.0 => println!("🟡 양호: {} req/sec", rps.round()),
        rps if rps > 100.0 => println!("🟠 보통: {} req/sec", rps.round()),
        rps => println!("🔴 개선 필요: {} req/sec", rps.round()),
    }

    if successful > 0 {
        println!("✅ 테스트 완료!");
    } else {
        println!("❌ 모든 요청이 실패했습니다. 서버 상태를 확인하세요.");
    }
}
