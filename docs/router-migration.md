# Router Migration Guide

`atomic_http` v0.10.0부터 `router` 피처를 통해 radix trie 기반 라우터를 제공합니다. 기존의 수동 `match` 패턴을 대체하여 경로 파라미터 추출, HTTP 메서드 디스패치, 스코프 기반 그룹핑을 지원합니다.

## 설치

```toml
[dependencies]
atomic_http = { version = "0.10", features = ["router"] }
```

> `router` 피처는 [`matchit`](https://crates.io/crates/matchit) 크레이트(~500 LOC, 의존성 없음)를 추가합니다.

---

## Before / After

### Before: 수동 match

```rust
use atomic_http::*;
use http::StatusCode;

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    let mut server = Server::new("127.0.0.1:8080").await?;

    loop {
        let accept = server.accept().await?;
        tokio::spawn(async move {
            let (request, mut response) = accept.parse_request().await?;
            let path = request.uri().path();

            match path {
                "/" => {
                    response.body_mut().body = "Hello".into();
                    *response.status_mut() = StatusCode::OK;
                }
                "/users" => match request.method() {
                    &http::Method::GET => { /* list users */ }
                    &http::Method::POST => { /* create user */ }
                    _ => { *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED; }
                },
                path if path.starts_with("/files/") => {
                    let filename = &path[7..]; // 수동 파싱
                    // serve file...
                }
                _ => {
                    *response.status_mut() = StatusCode::NOT_FOUND;
                }
            }

            response.responser().await?;
            Ok::<(), SendableError>(())
        });
    }
}
```

### After: Router

```rust
use atomic_http::router::Router;
use atomic_http::*;
use http::StatusCode;

// 1. 라우트 enum 정의
enum Route { Home, ListUsers, CreateUser, ServeFile }

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    // 2. 라우터 구성 (서버 시작 전, 한 번만)
    let router: &'static Router<Route> = Box::leak(Box::new(
        Router::new()
            .get("/", Route::Home)
            .get("/users", Route::ListUsers)
            .post("/users", Route::CreateUser)
            .get("/files/{*path}", Route::ServeFile)
    ));

    let mut server = Server::new("127.0.0.1:8080").await?;

    loop {
        let accept = server.accept().await?;
        tokio::spawn(async move {
            let (request, mut response) = accept.parse_request().await?;

            // 3. match 블록을 router.find()로 교체
            match router.find(request.method(), request.uri().path()) {
                Some(m) => match m.value {
                    Route::Home => {
                        response.body_mut().body = "Hello".into();
                        *response.status_mut() = StatusCode::OK;
                    }
                    Route::ListUsers => { /* list users */ }
                    Route::CreateUser => { /* create user */ }
                    Route::ServeFile => {
                        let filename = m.params.get("path").unwrap_or("");
                        // serve file — 수동 &path[7..] 불필요
                    }
                },
                None => {
                    *response.status_mut() = StatusCode::NOT_FOUND;
                }
            }

            response.responser().await?;
            Ok::<(), SendableError>(())
        });
    }
}
```

---

## 변경 요약

| Before | After |
|---|---|
| `match path { "/foo" => ... }` | `router.find(method, path)` → `match m.value` |
| `path.starts_with("/files/")` + `&path[7..]` | `.get("/files/{*path}", ...)` + `m.params.get("path")` |
| 경로 안에서 `match request.method()` | `.get()`, `.post()` 등으로 메서드별 등록 |
| 404: `_ =>` 매치 암 | 404: `None =>` 매치 암 |
| 같은 경로 다른 메서드: 수동 분기 | `.get("/users", ...) .post("/users", ...)` 자동 디스패치 |
| prefix 그룹핑 불가 | `.scope("/api/v1", \|s\| s.get(...))` |

---

## 경로 패턴

```rust
// Named parameter — 다음 세그먼트까지 매칭
.get("/users/{id}", Route::GetUser)
// m.params.get("id") => Some("42")

// 여러 파라미터
.get("/orgs/{org}/repos/{repo}", Route::GetRepo)
// m.params.get("org"), m.params.get("repo")

// Catch-all — 경로 끝까지 전부 매칭
.get("/files/{*path}", Route::ServeFile)
// m.params.get("path") => Some("images/logo.png")
```

---

## 스코프 (그룹핑)

반복되는 prefix를 `scope()`로 묶을 수 있습니다:

```rust
let router = Router::new()
    .get("/", Route::Home)
    .scope("/api/v1", |s| s
        .scope("/users", |s| s
            .get("/", Route::ListUsers)       // → /api/v1/users/
            .get("/{id}", Route::GetUser)     // → /api/v1/users/{id}
            .post("/", Route::CreateUser)     // → /api/v1/users/
            .delete("/{id}", Route::DeleteUser) // → /api/v1/users/{id}
        )
        .scope("/files", |s| s
            .get("/{*path}", Route::ServeFile) // → /api/v1/files/{*path}
        )
    );
```

스코프는 순수 등록 시점의 편의 기능입니다. 내부적으로 모든 라우트는 flat한 radix trie에 저장되므로 런타임 오버헤드는 없습니다.

---

## Arena 모드

Router는 body 타입에 의존하지 않으므로 Arena 모드에서도 동일하게 사용됩니다:

```rust
// Standard
let (request, mut response) = accept.parse_request().await?;
match router.find(request.method(), request.uri().path()) { ... }
response.responser().await?;

// Arena — 라우터 코드는 동일
let (request, mut response) = accept.parse_request_arena_writer().await?;
match router.find(request.method(), request.uri().path()) { ... }
response.responser_arena().await?;
```

---

## WebSocket + Router

`websocket` 피처와 함께 사용할 때:

```rust
match accept.stream_parse().await? {
    StreamResult::WebSocket(ws_stream, request, peer) => {
        // WebSocket 라우팅 — request.uri().path()로 분기
        match router.find(&http::Method::GET, request.uri().path()) {
            Some(m) => match m.value {
                Route::WsEcho => { /* echo handler */ }
                Route::WsChat => { /* chat handler */ }
                _ => {}
            },
            None => { /* unknown ws path */ }
        }
    }
    StreamResult::Http(request, mut response) => {
        // HTTP 라우팅 — 동일
        match router.find(request.method(), request.uri().path()) { ... }
        response.responser().await?;
    }
}
```

---

## 변경되지 않는 것

- `Server::new()`, `server.accept()` — 동일
- `accept.parse_request()` / `parse_request_arena_writer()` — 동일
- `response.responser()` / `responser_arena()` — 동일
- `response.body_mut()`, `response.status_mut()` — 동일
- `RequestUtils` (`get_json`, `get_text`, `get_multi_part`) — 동일
- `ResponseUtil` (`set_arena_json`, `response_file`) — 동일
- Connection pool, zero-copy cache — 동일

Router는 `match` 블록만 대체합니다. 나머지 API는 모두 그대로입니다.

---

## `insert()` (동적 등록)

빌더 패턴 대신 런타임에 동적으로 라우트를 추가할 수 있습니다:

```rust
let mut router = Router::new();
router.insert(Method::GET, "/", Route::Home)?;
router.insert(Method::GET, "/users/{id}", Route::GetUser)?;
// InsertError 반환 — 충돌하는 패턴일 경우
```

---

## API Reference

| Method | Description |
|---|---|
| `Router::new()` | 빈 라우터 생성 |
| `.get(path, value)` | GET 라우트 등록 |
| `.post(path, value)` | POST 라우트 등록 |
| `.put(path, value)` | PUT 라우트 등록 |
| `.delete(path, value)` | DELETE 라우트 등록 |
| `.patch(path, value)` | PATCH 라우트 등록 |
| `.head(path, value)` | HEAD 라우트 등록 |
| `.options(path, value)` | OPTIONS 라우트 등록 |
| `.route(method, path, value)` | 임의 메서드 라우트 등록 |
| `.scope(prefix, closure)` | 공통 prefix 그룹핑 |
| `.insert(method, path, value)` | 동적 등록 (Result 반환) |
| `.find(method, path)` | 라우트 매칭 (Option 반환) |
| `Match.value` | 매칭된 값 참조 |
| `Match.params.get(key)` | 경로 파라미터 조회 |
| `Match.params.iter()` | 모든 파라미터 순회 |
| `Match.params.len()` | 파라미터 개수 |
