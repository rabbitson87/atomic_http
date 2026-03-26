# Atomic HTTP

고성능 HTTP 서버 라이브러리 - Arena 메모리 관리와 Zero-copy 기술을 활용한 최적화된 HTTP 서버

## 🚀 주요 기능

- **Arena 메모리 관리**: 효율적인 메모리 할당과 관리
- **Zero-copy 기술**: 메모리 복사 없는 파일 서빙
- **멀티파트 지원**: 고성능 파일 업로드 처리
- **비동기 처리**: Tokio 기반 비동기 I/O
- **타입 안전성**: Rust의 타입 시스템 활용

## 📦 설치

```toml
[dependencies]
atomic_http = { version = "0.6.0", features = ["arena"] }
```

## 🛠️ 기능별 Features

- `arena`: Arena 메모리 관리 (기본 활성화)
- `router`: Radix trie 기반 라우터 ([마이그레이션 가이드](docs/router-migration.md))
- `websocket`: WebSocket 업그레이드 지원
- `connection_pool`: Keep-alive 연결 풀링
- `response_file`: 파일 응답 지원
- `env`: 환경변수 설정 지원
- `debug`: 디버그 출력 활성화

## 🧪 테스트 실행 가이드

### 📋 새로운 통합 테스트 (추천)

#### 1. 완전 통합 테스트
서버와 클라이언트를 자동으로 시작하고 종합적인 테스트 실행
```bash
# Arena + Zero-copy 모드
cargo run --example integrated_test --features arena

# 표준 모드
cargo run --example integrated_test
```

#### 2. 성능 비교 벤치마크
Arena 서버와 표준 서버의 성능을 직접 비교
```bash
# Arena vs 표준 서버 성능 비교
cargo run --example comparative_benchmark --features arena

# 릴리즈 모드로 정확한 성능 측정
cargo run --release --example comparative_benchmark --features arena
```

#### 3. 멀티파트 테스트
파일 업로드와 멀티파트 폼 데이터 처리 테스트
```bash
# Arena 멀티파트 테스트
cargo run --example integrated_multipart_test --features arena

# 표준 멀티파트 테스트  
cargo run --example integrated_multipart_test
```

### 🔧 간단한 테스트

#### 0. 문제 해결용 디버그 테스트
```bash
# Arena 서버 디버그
cargo run --example debug_test --features arena -- 9999

# 표준 서버 디버그
cargo run --example debug_test -- 9999
```

#### 1. 간단한 벤치마크 테스트 (추천)
```bash
# Arena 간단 벤치마크
cargo run --example simple_benchmark_test --features arena -- 9998

# 표준 간단 벤치마크  
cargo run --example simple_benchmark_test -- 9998
```

#### 2. 기본 서버 테스트
```bash
# Arena 서버
cargo run --example simple_server_test --features arena -- 9080

# 표준 서버
cargo run --example simple_server_test -- 9081
```

#### 2. 간단한 성능 테스트
```bash
# Arena 성능 테스트
cargo run --example simple_performance_test --features arena

# 표준 성능 테스트
cargo run --example simple_performance_test
```

### 📊 벤치마크

Criterion을 사용한 마이크로 벤치마크:
```bash
cargo bench
```

### 🔍 기존 테스트들 (레거시)

필요시 개별 기능 테스트:
```bash
# 기존 서버 테스트
cargo run --example server --features arena

# 부하 테스트 클라이언트 (별도 실행)
cargo run --example load_test_client -- -n 1000 -c 50

# 파일 서빙 테스트
cargo run --example file_serving_test --features arena,response_file

# 제로카피 테스트
cargo run --example zero_copy_test --features arena
```

## 📈 성능 특징

### Arena 서버의 장점
- ✅ **제로카피**: 메모리 복사 없음
- ✅ **낮은 메모리 사용량**: 원본 데이터만 유지  
- ✅ **빠른 파싱**: String 생성 없이 바이트 직접 접근
- ✅ **예측 가능한 성능**: GC 압박 없음
- ✅ **대용량 파일 최적화**: 메모리 효율적

### 표준 서버 특징
- 📝 **안정적이고 검증된 방식**
- 📝 **메모리 복사와 String 할당 포함**
- 📝 **상대적으로 높은 메모리 사용량**

## 💡 사용 예제

### 기본 서버 설정

```rust
use atomic_http::*;
use http::StatusCode;

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let mut server = Server::new("127.0.0.1:8080").await?;
    
    loop {
        let accept = server.accept().await?;
        
        tokio::spawn(async move {
            let (request, mut response) = accept.parse_request_arena_writer().await?;
            
            // JSON 파싱 (Zero-copy)
            let data: MyData = request.get_json_arena()?;
            
            // 응답 생성 (Arena 할당)
            let result = serde_json::json!({
                "status": "success",
                "data": data
            });
            response.body_mut().set_arena_json(&result)?;
            *response.status_mut() = StatusCode::OK;
            
            response.responser_arena().await?;
            
            Ok::<(), SendableError>(())
        });
    }
}
```

### 멀티파트 파일 업로드

```rust
// Arena 멀티파트 처리
match request.get_multi_part_arena()? {
    Some(form) => {
        // 텍스트 필드 접근
        for i in 0..form.text_fields.len() {
            let name = form.get_text_field_name(i);
            let value = form.get_text_field_value(i);
        }
        
        // 파일 처리
        for part in &form.parts {
            let filename = part.get_file_name();
            let file_data = part.get_body(); // Zero-copy 접근
            
            // 파일 저장
            tokio::fs::write(filename?, file_data).await?;
        }
    }
    None => {
        // JSON 또는 다른 형식 처리
    }
}
```

## 🔧 환경 설정

환경변수를 통한 서버 설정 (env 피쳐 활성화 시):

```bash
export NO_DELAY=true
export READ_TIMEOUT_MILISECONDS=5000
export READ_BUFFER_SIZE=8192
export ROOT_PATH="/var/www"
export ZERO_COPY_THRESHOLD=1048576
export ENABLE_FILE_CACHE=true
```

## 🏗️ 개발 및 기여

### 요구사항
- Rust 1.77 이상
- Tokio 런타임

### 컴파일
```bash
# 기본 빌드
cargo build

# Arena + Zero-copy 모든 기능
cargo build --features arena,response_file,env

# 릴리즈 빌드  
cargo build --release --features arena
```

### 테스트 
```bash
# 단위 테스트
cargo test

# 통합 테스트
cargo run --example integrated_test --features arena

# 성능 벤치마크
cargo bench
```

## 📄 라이선스

Apache License 2.0

## 🤝 기여

이슈와 풀 리퀘스트를 환영합니다. 주요 변경사항은 먼저 이슈를 열어 논의해 주세요.

## 📞 지원

- GitHub Issues: [프로젝트 이슈](https://github.com/rabbitson87/atomic_http/issues)
- 문서: [docs.rs](https://docs.rs/atomic_http)

---

**추천 테스트 순서:**

1. `cargo run --example debug_test --features arena` - 기본 동작 확인
2. `cargo run --example simple_benchmark_test --features arena` - 간단한 벤치마크
3. `cargo run --example integrated_test --features arena` - 전체 기능 확인
4. `cargo run --example comparative_benchmark --features arena` - 성능 비교
5. `cargo bench` - 마이크로 벤치마크

각 테스트는 서버와 클라이언트를 자동으로 관리하므로 별도 설정이 필요하지 않습니다.

## 🚨 문제 해결

### 벤치마크 실패 문제
만약 `comparative_benchmark`에서 요청이 실패한다면:

1. **기본 테스트부터 실행**:
   ```bash
   cargo run --example debug_test --features arena
   ```

2. **간단한 벤치마크 실행**:
   ```bash
   cargo run --example simple_benchmark_test --features arena
   ```

3. **포트 충돌 확인**:
   ```bash
   # 다른 포트 사용
   cargo run --example simple_benchmark_test --features arena -- 8888
   ```

### 일반적인 문제들
- **컴파일 오류**: `cargo clean && cargo build --features arena`
- **연결 실패**: 방화벽이나 포트 사용 중인지 확인
- **성능 측정**: `--release` 모드로 실행

### 로그 확인
각 테스트는 상세한 로그를 출력하므로 실패 지점을 쉽게 파악할 수 있습니다.