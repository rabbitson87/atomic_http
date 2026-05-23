# Security & Hardening Guide

`atomic_http` 는 낮은 레벨의 HTTP 서버 라이브러리이며, 프로덕션 환경에서는
다음 권장사항을 적용해 운영하는 것을 권장합니다.

## 요청 한도 (DoS 방어)

```rust
use atomic_http::{Options, Server};

let mut opts = Options::new();
opts.max_body_size = Some(10 * 1024 * 1024);   // 10 MB 본문 cap
opts.header_read_deadline_ms = Some(5_000);    // 5초 내 헤더 완전 수신
opts.read_timeout_milliseconds = 3_000;          // 단일 read 타임아웃

let server = Server::with_options("0.0.0.0:8080", opts).await?;
```

- `max_body_size`: 클라이언트가 광고한 `Content-Length` 가 이 값을 넘으면
  바이트를 읽기 전에 거부 — 대용량 alloc 회피.
- `header_read_deadline_ms`: 헤더 수신 전체에 절대 deadline.
  Slowloris (헤더를 1바이트씩 흘리는 공격) 차단.
- `read_timeout_milliseconds`: 단일 read 호출 타임아웃. 위 deadline의 분할 단위.

## `response_file` 사용 시

- `Options::root_path` 가 모든 정적 파일의 root 역할을 합니다.
- 사용자 입력으로 받은 경로는 `..`, 절대 경로, Windows 드라이브 prefix가
  포함되면 자동으로 거부됩니다 (0.14.0+).
- 추가 방어로 root 자체를 가능한 작게 잡고 (예: `./public/`),
  심볼릭 링크가 root 밖을 가리키지 않도록 관리하세요.

## TLS (`tokio_rustls` 피쳐)

이 크레이트는 TLS 종단 자체를 강제하지 않고, `tokio_rustls::TlsStream<TcpStream>` 에
`StreamHttp` 트레잇을 구현해 두었습니다. `ServerConfig` 는 사용자가 직접 구성합니다.
권장 baseline:

```rust
use std::sync::Arc;
use tokio_rustls::rustls::{ServerConfig, version};

let config = ServerConfig::builder_with_protocol_versions(&[
        &version::TLS13,
        &version::TLS12, // TLS 1.0/1.1 비활성
    ])
    .with_no_client_auth()
    .with_single_cert(cert_chain, private_key)?;
let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
```

- TLS 1.0 / 1.1 비활성, TLS 1.3 우선.
- 인증서는 ECDSA P-256 또는 RSA 2048+ 권장.
- HTTP/3 / QUIC 가 필요한 경우 별도 스택을 검토하세요 (본 크레이트는 HTTP/1.x).

## 응답 헤더

다음 헤더를 라우터/핸들러에서 일관되게 설정하는 것을 권장합니다.

- `Strict-Transport-Security: max-age=63072000; includeSubDomains` (TLS 종단인 경우)
- `X-Content-Type-Options: nosniff`
- `Content-Security-Policy: ...` (서비스 정책에 맞게)
- `Referrer-Policy: strict-origin-when-cross-origin`

## 취약점 신고

보안 이슈는 공개 issue 대신 메인테이너에게 직접 메일 부탁드립니다:
**hsng95@gmail.com**
