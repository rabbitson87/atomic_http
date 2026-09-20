// SSE 스트리밍 예제
//   cargo run --example sse_test            # 기본 포트 8080
//   curl -N http://127.0.0.1:8080/events
use atomic_http::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let port = std::env::args().nth(1).unwrap_or_else(|| "8080".into());
    let mut server = Server::new(&format!("127.0.0.1:{port}")).await?;
    println!("SSE 서버: http://127.0.0.1:{port}/events");

    loop {
        let accept = server.accept().await?;
        tokio::spawn(async move {
            let Ok((_request, response)) = accept.parse_request_arena_writer().await else {
                return;
            };
            if let Err(e) = stream_events(response).await {
                // 클라이언트가 먼저 끊은 경우는 정상 종료로 본다.
                if !is_disconnect(&e) {
                    eprintln!("stream error: {e}");
                }
            }
        });
    }
}

async fn stream_events(response: http::Response<ArenaWriter>) -> Result<(), SendableError> {
    let mut sse = response.into_sse().await?;
    sse.send(&SseEvent::retry(2000)).await?;

    for n in 1..=5u32 {
        // 느린 작업 동안 15초마다 주석 프레임을 보내 연결을 유지한다.
        let square = sse
            .keepalive_while(Duration::from_secs(15), async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                n * n
            })
            .await?;
        sse.send_event("tick", &format!("{{\"n\":{n},\"square\":{square}}}"))
            .await?;
    }
    sse.send_event("done", "{}").await?;
    sse.finish().await
}
