use std::path::Path;
use std::sync::Arc;
use std::{env::current_dir, io, net::SocketAddr, path::PathBuf};

#[cfg(feature = "env")]
use std::str::FromStr;

#[cfg(feature = "arena")]
use bumpalo::Bump;
use serde::{Deserialize, Serialize};

pub mod helpers;

#[cfg(feature = "connection_pool")]
pub mod connection_pool;

#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(feature = "router")]
pub mod router;

pub use helpers::traits::http_request::RequestUtils;
#[cfg(feature = "arena")]
pub use helpers::traits::http_request::RequestUtilsArena;
pub use helpers::traits::http_response::ResponseUtil;
#[cfg(feature = "arena")]
pub use helpers::traits::http_response::ResponseUtilArena;
pub use helpers::traits::http_stream::StreamHttp;

#[cfg(feature = "arena")]
pub use helpers::traits::http_stream::{StreamHttpArena, StreamHttpArenaWriter};

pub use helpers::traits::zero_copy::{
    parse_json_file, CacheConfig, CacheStats, CachedFileData, FileLoadResult, ZeroCopyCache,
    ZeroCopyFile,
};

#[cfg(feature = "connection_pool")]
pub use connection_pool::{ConnectionPool, ConnectionPoolConfig, ConnectionStats};

#[cfg(feature = "websocket")]
pub use websocket::StreamResult;

#[cfg(all(feature = "websocket", feature = "arena"))]
pub use websocket::StreamResultArena;

#[cfg(feature = "websocket")]
pub use websocket::StreamResultAuto;

pub mod external {
    pub use async_trait;
    #[cfg(feature = "env")]
    pub use dotenvy;
    pub use http;
    #[cfg(feature = "response_file")]
    pub use mime_guess;
    pub use tokio;

    #[cfg(feature = "arena")]
    pub use bumpalo;

    pub use memmap2;

    #[cfg(feature = "websocket")]
    pub use tokio_tungstenite;
}

use http::{Request, Response};

#[macro_export]
macro_rules! dev_print {
    ($($rest:tt)*) => {
        if cfg!(feature = "debug") {
            println!($($rest)*)
        }
    };
}

use tokio::net::TcpListener;

use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

pub use bytes::Bytes;

pub type SendableError = Box<dyn std::error::Error + Send + Sync>;

/// `parse_request_auto()` 기본 cap — 50 MiB.
/// 이 이하 Content-Length는 arena (zero-copy parsing), 초과/미상은 streaming으로 처리.
/// 커스텀 cap이 필요하면 `parse_request_auto_with_cap(cap)`.
pub const DEFAULT_AUTO_ARENA_CAP: usize = 50 * 1024 * 1024;

/// `Accept::parse_request_auto(arena_cap)` 결과. Content-Length가 `arena_cap` 이하이면
/// `Arena` (zero-copy 파싱), 초과/미지정이면 `Streaming` (메모리 절약).
///
/// arena feature 비활성화 시 `Arena` variant는 컴파일되지 않으므로 match 시 cfg 처리 불필요.
pub enum AutoParseResult {
    /// CL ≤ arena_cap — 전체 body를 한 번에 받아 arena에 보관, zero-copy parsing 가능.
    #[cfg(feature = "arena")]
    Arena {
        request: Request<ArenaBody>,
        response: Response<ArenaWriter>,
    },
    /// CL > arena_cap 또는 CL 미상 — body는 socket stream 으로 청크 단위 read.
    Streaming {
        request: Request<Body>,
        response: Response<Writer>,
    },
}

pub struct Server {
    pub listener: TcpListener,
    /// 공유 옵션. 요청마다 `Arc::clone`만 하면 됨 (full clone 없음).
    /// 설정 변경 시 `Arc::make_mut`로 단일 소유일 때만 in-place 수정,
    /// 공유 중이면 자동으로 copy-on-write.
    pub options: Arc<Options>,
    #[cfg(feature = "connection_pool")]
    pub connection_pool: Option<Arc<ConnectionPool>>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub no_delay: bool,
    pub read_timeout_milliseconds: u64,
    pub root_path: PathBuf,
    pub read_buffer_size: usize,
    pub read_max_retry: u8,
    /// 요청 본문 최대 크기 (바이트). `None`이면 무제한 (기존 동작).
    /// 클라이언트가 보낸 `Content-Length`가 이 값을 초과하면 즉시 거부 (DoS 방어).
    pub max_body_size: Option<usize>,
    /// 헤더 전체 수신 데드라인 (밀리초). `None`이면 자동 = read_timeout * (read_max_retry + 1).
    /// Slowloris 방어 — 클라이언트가 헤더를 1바이트씩 천천히 흘려도 이 시간 내 완료되지 않으면 거부.
    pub header_read_deadline_ms: Option<u64>,
    pub zero_copy_threshold: usize,
    pub enable_file_cache: bool,

    // Connection pooling configuration
    #[cfg(feature = "connection_pool")]
    pub connection_option: ConnectionPoolConfig,
}

impl Options {
    pub fn new() -> Options {
        let mut _options = Options {
            no_delay: true,
            read_timeout_milliseconds: 3000,
            // current_dir() 실패 시 "."로 폴백 (panic 회피)
            root_path: current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            read_buffer_size: 4096,
            read_max_retry: 3,
            max_body_size: None,              // 기본 무제한 (기존 동작 보존)
            header_read_deadline_ms: None,    // 기본: read_timeout * (max_retry+1)
            zero_copy_threshold: 1024 * 1024, // 1MB 이상 파일에 제로카피 적용
            enable_file_cache: true,

            // Connection pooling enabled by default with nginx-like settings
            #[cfg(feature = "connection_pool")]
            connection_option: ConnectionPoolConfig::new(),
        };

        #[cfg(feature = "env")]
        {
            use std::env;
            if let Ok(data) = env::var("NO_DELAY") {
                // true, false
                if let Ok(data) = data.parse::<bool>() {
                    _options.no_delay = data;
                }
            }

            if let Ok(data) = env::var("READ_TIMEOUT_MILLISECONDS") {
                if let Ok(data) = data.parse::<u64>() {
                    _options.read_timeout_milliseconds = data;
                }
            }

            if let Ok(data) = env::var("ROOT_PATH") {
                _options.root_path = PathBuf::from_str(&data).unwrap();
            }

            if let Ok(data) = env::var("READ_BUFFER_SIZE") {
                if let Ok(data) = data.parse::<usize>() {
                    _options.read_buffer_size = data;
                }
            }

            if let Ok(data) = env::var("READ_MAX_RETRY") {
                if let Ok(data) = data.parse::<u8>() {
                    _options.read_max_retry = data;
                }
            }

            if let Ok(data) = env::var("MAX_BODY_SIZE") {
                if let Ok(data) = data.parse::<usize>() {
                    _options.max_body_size = Some(data);
                }
            }

            if let Ok(data) = env::var("HEADER_READ_DEADLINE_MS") {
                if let Ok(data) = data.parse::<u64>() {
                    _options.header_read_deadline_ms = Some(data);
                }
            }

            if let Ok(data) = env::var("ZERO_COPY_THRESHOLD") {
                if let Ok(data) = data.parse::<usize>() {
                    _options.zero_copy_threshold = data;
                }
            }

            if let Ok(data) = env::var("ENABLE_FILE_CACHE") {
                if let Ok(data) = data.parse::<bool>() {
                    _options.enable_file_cache = data;
                }
            }

            // Connection pooling environment variables
            #[cfg(feature = "connection_pool")]
            {
                let mut needs_config = false;

                let enable_keep_alive = if let Ok(data) = env::var("ENABLE_KEEP_ALIVE") {
                    needs_config = true;
                    data.parse::<bool>().unwrap_or(true)
                } else {
                    true
                };

                let keep_alive_timeout = if let Ok(data) = env::var("KEEP_ALIVE_TIMEOUT") {
                    needs_config = true;
                    data.parse::<u64>().unwrap_or(75)
                } else {
                    75
                };

                let max_connections = if let Ok(data) = env::var("MAX_IDLE_CONNECTIONS_PER_HOST") {
                    needs_config = true;
                    data.parse::<usize>().unwrap_or(32)
                } else {
                    32
                };

                if needs_config {
                    _options.connection_option = ConnectionPoolConfig::new()
                        .keep_alive(enable_keep_alive)
                        .idle_timeout(keep_alive_timeout)
                        .max_connections_per_host(max_connections);
                }
            }
        }

        _options
    }

    pub fn set_zero_copy_threshold(&mut self, threshold: usize) {
        self.zero_copy_threshold = threshold;
    }

    pub fn enable_zero_copy_cache(&mut self, enable: bool) {
        self.enable_file_cache = enable;
    }

    // Connection pooling configuration methods
    #[cfg(feature = "connection_pool")]
    pub fn set_connection_option(&mut self, config: ConnectionPoolConfig) {
        self.connection_option = config;
    }

    #[cfg(feature = "connection_pool")]
    pub fn get_connection_option(&self) -> &ConnectionPoolConfig {
        &self.connection_option
    }

    #[cfg(feature = "connection_pool")]
    pub fn get_connection_option_mut(&mut self) -> &mut ConnectionPoolConfig {
        &mut self.connection_option
    }

    #[cfg(feature = "connection_pool")]
    pub fn is_connection_pool_enabled(&self) -> bool {
        self.connection_option.enable_keep_alive
    }

    #[cfg(feature = "connection_pool")]
    pub fn enable_connection_pool(&mut self) {
        self.connection_option.enable_keep_alive = true;
    }

    #[cfg(feature = "connection_pool")]
    pub fn disable_connection_pool(&mut self) {
        self.connection_option.enable_keep_alive = false;
    }
}

impl Server {
    pub async fn new(address: &str) -> Result<Server, SendableError> {
        dev_print!("✅ Server initialized with Arena support");

        let mut server = Server {
            listener: TcpListener::bind(address).await?,
            options: Arc::new(Options::new()),
            #[cfg(feature = "connection_pool")]
            connection_pool: None,
        };

        // Auto-enable connection pool if enabled in default options
        #[cfg(feature = "connection_pool")]
        if server.options.is_connection_pool_enabled() {
            if let Err(e) = server.enable_connection_pool() {
                dev_print!("⚠️  Failed to enable connection pool: {}", e);
            }
        }

        Ok(server)
    }

    /// Create server with custom options (including connection pool configuration)
    pub async fn with_options(address: &str, options: Options) -> Result<Server, SendableError> {
        dev_print!("✅ Server initialized with custom options");

        let mut server = Server {
            listener: TcpListener::bind(address).await?,
            options: Arc::new(options),
            #[cfg(feature = "connection_pool")]
            connection_pool: None,
        };

        // Auto-enable connection pool if enabled in options
        #[cfg(feature = "connection_pool")]
        if server.options.is_connection_pool_enabled() {
            if let Err(e) = server.enable_connection_pool() {
                dev_print!("⚠️  Failed to enable connection pool: {}", e);
            }
        }

        Ok(server)
    }

    /// 단일 소유 상태일 때만 in-place 수정. 공유 중이면 copy-on-write.
    /// 서버 설정은 일반적으로 요청 처리 시작 전에 끝나므로 대부분 in-place.
    pub fn options_mut(&mut self) -> &mut Options {
        Arc::make_mut(&mut self.options)
    }

    /// Create server with connection pool configuration (legacy method)
    #[cfg(feature = "connection_pool")]
    pub async fn with_connection_pool(
        address: &str,
        pool_config: Option<ConnectionPoolConfig>,
    ) -> Result<Server, SendableError> {
        let mut options = Options::new();
        if let Some(config) = pool_config {
            options.set_connection_option(config);
        } else {
            options.disable_connection_pool();
        }
        Self::with_options(address, options).await
    }

    /// Enable connection pooling with configuration from options
    #[cfg(feature = "connection_pool")]
    pub fn enable_connection_pool(&mut self) -> Result<(), SendableError> {
        if self.options.is_connection_pool_enabled() {
            let pool = ConnectionPool::new(self.options.connection_option.clone());
            pool.start_cleanup_task();
            self.connection_pool = Some(Arc::new(pool));
            dev_print!("✅ Connection pool enabled for server");
            Ok(())
        } else {
            dev_print!("❌ Connection pool is disabled in options");
            Err("Connection pool is disabled".into())
        }
    }

    /// Disable connection pooling
    #[cfg(feature = "connection_pool")]
    pub async fn disable_connection_pool(&mut self) {
        if let Some(pool) = self.connection_pool.take() {
            pool.shutdown().await;
        }
        dev_print!("🔴 Connection pool disabled for server");
    }

    /// Get connection pool statistics
    #[cfg(feature = "connection_pool")]
    pub async fn get_connection_pool_stats(&self) -> Option<ConnectionStats> {
        self.connection_pool.as_ref().map(|pool| pool.stats())
    }

    /// Print connection pool statistics
    #[cfg(feature = "connection_pool")]
    pub async fn print_connection_pool_stats(&self) {
        if let Some(stats) = self.get_connection_pool_stats().await {
            println!("🌐 {}", stats);
        } else {
            println!("🔴 Connection pool is disabled");
        }
    }

    /// Clear connection pool
    #[cfg(feature = "connection_pool")]
    pub async fn clear_connection_pool(&self) {
        if let Some(pool) = &self.connection_pool {
            pool.clear().await;
        }
    }

    pub async fn accept(&mut self) -> Result<Accept, SendableError> {
        use std::time::Duration;

        let (stream, addr) = match self.listener.accept().await {
            Ok(data) => data,
            Err(e) => {
                if is_connection_error(&e) {
                    return Err(e.into());
                }
                dev_print!("Accept Error: {:?}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
                return Err(e.into());
            }
        };
        // Options는 이미 Arc — 요청마다 atomic increment 1회. peer는 Accept에 별도 저장.
        Ok(Accept::new(stream, Arc::clone(&self.options), addr))
    }

    pub fn set_no_delay(&mut self, no_delay: bool) {
        self.options_mut().no_delay = no_delay;
    }

    /// 캐시 통계 출력
    pub fn print_cache_stats(&self) {
        let stats = ZeroCopyCache::global().stats();
        println!("📊 {}", stats);
    }
}

pub struct Accept {
    pub tcp_stream: TcpStream,
    pub option: Arc<Options>,
    /// 요청 송신자 주소. 요청별 값이므로 Options와 분리하여 여기에 보관.
    pub peer: SocketAddr,
}

impl Accept {
    pub fn new(tcp_stream: TcpStream, option: Arc<Options>, peer: SocketAddr) -> Self {
        Self {
            tcp_stream,
            option,
            peer,
        }
    }

    /// 요청 클라이언트의 IP 주소를 문자열로 반환 (`Options::get_request_ip` 대체).
    pub fn get_request_ip(&self) -> String {
        self.peer.ip().to_string()
    }

    /// 요청 클라이언트의 SocketAddr 반환.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    pub async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        Ok(self
            .tcp_stream
            .parse_request(self.option, self.peer)
            .await?)
    }

    /// 0.14.0 신규 — body를 streaming 모드로 받기. 헤더만 먼저 읽고
    /// Body가 socket의 read half를 보유. 핸들러는 `body.read_chunk()` /
    /// `body.into_multipart(...)` / `body.into_stream()` 으로 청크 단위 처리 가능.
    pub async fn parse_request_streaming(
        self,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        use crate::helpers::traits::http_stream::StreamHttp;
        self.tcp_stream
            .parse_request_streaming(self.option, self.peer)
            .await
    }

    /// 0.14.0 신규 — 헤더만 먼저 읽고 `Content-Length` 보고 자동 분기.
    /// `DEFAULT_AUTO_ARENA_CAP` (50 MiB) 이하면 arena 경로, 초과/미상이면 streaming.
    /// 커스텀 cap이 필요하면 `parse_request_auto_with_cap(cap)` 사용.
    pub async fn parse_request_auto(self) -> Result<AutoParseResult, SendableError> {
        self.parse_request_auto_with_cap(DEFAULT_AUTO_ARENA_CAP)
            .await
    }

    /// `parse_request_auto` 의 명시적 cap 버전.
    /// CL ≤ `arena_cap` 이면 arena (zero-copy parsing), 초과/미상이면 streaming (메모리 절약).
    /// arena feature 비활성화 시 항상 `AutoParseResult::Streaming` 반환.
    pub async fn parse_request_auto_with_cap(
        self,
        arena_cap: usize,
    ) -> Result<AutoParseResult, SendableError> {
        use crate::helpers::traits::http_stream::{read_headers_only, HeaderReadResult};

        self.tcp_stream.set_nodelay(self.option.no_delay)?;
        let HeaderReadResult {
            header_bytes,
            leftover,
            content_length,
            stream,
        } = read_headers_only(self.tcp_stream, &self.option).await?;

        // arena 경로 분기: feature 있고, CL 있고, cap 이하
        #[cfg(feature = "arena")]
        if let Some(cl) = content_length {
            if cl <= arena_cap {
                use crate::helpers::traits::http_stream::{
                    get_parse_result_arena_writer, parse_http_request_arena, read_remaining_body,
                };

                // 남은 body 마저 읽어 단일 Vec로 모으기
                let (full_body, stream2) =
                    read_remaining_body(leftover, stream, content_length, &self.option).await?;

                // 헤더 + body 합쳐 ArenaBody 생성 (bump 안으로 복사)
                let header_end = header_bytes.len();
                let mut full = Vec::with_capacity(header_end + full_body.len());
                full.extend_from_slice(&header_bytes);
                full.extend_from_slice(&full_body);
                let arena_body = ArenaBody::new(&full, header_end, header_end);

                let request = parse_http_request_arena(arena_body)?;
                let (request, response) =
                    get_parse_result_arena_writer(request, stream2, self.option, self.peer)?;
                return Ok(AutoParseResult::Arena { request, response });
            }
        }
        // arena_cap 인자가 streaming 경로에선 안 쓰이지만 BREAKING 인자 없애지 않으려고 명시 변수화.
        let _ = arena_cap;

        // streaming 경로
        use crate::helpers::traits::http_stream::get_request;
        let request_buffered = get_request(header_bytes).await?;
        let (parts, _empty_body) = request_buffered.into_parts();

        let (read_half, write_half) = stream.into_split();
        let body = Body::new_streaming(
            leftover,
            read_half,
            content_length,
            Some(self.peer),
            self.option.max_body_size,
        );
        let request = Request::from_parts(parts, body);
        let version = request.version();

        let response = Response::builder()
            .version(version)
            .header(http::header::CONTENT_TYPE, "application/json")
            .status(400)
            .body(Writer {
                stream: write_half,
                body: String::new(),
                bytes: vec![],
                use_file: false,
                options: self.option,
            })?;
        Ok(AutoParseResult::Streaming { request, response })
    }

    #[cfg(feature = "websocket")]
    pub async fn stream_parse(self) -> Result<StreamResult, SendableError> {
        self.tcp_stream.set_nodelay(self.option.no_delay)?;
        websocket::try_upgrade(self.tcp_stream, self.option, self.peer).await
    }

    /// 0.14.0 신규 — WebSocket 분기 + HTTP auto (arena/streaming) 분기를 한 번에.
    /// `DEFAULT_AUTO_ARENA_CAP` (50 MiB) 사용. 커스텀 cap은 `stream_parse_auto_with_cap`.
    #[cfg(feature = "websocket")]
    pub async fn stream_parse_auto(self) -> Result<StreamResultAuto, SendableError> {
        self.stream_parse_auto_with_cap(DEFAULT_AUTO_ARENA_CAP).await
    }

    /// `stream_parse_auto` 의 명시적 cap 버전.
    #[cfg(feature = "websocket")]
    pub async fn stream_parse_auto_with_cap(
        self,
        arena_cap: usize,
    ) -> Result<StreamResultAuto, SendableError> {
        self.tcp_stream.set_nodelay(self.option.no_delay)?;
        websocket::try_upgrade_auto(self.tcp_stream, self.option, self.peer, arena_cap).await
    }

    #[cfg(all(feature = "websocket", feature = "arena"))]
    pub async fn stream_parse_arena(self) -> Result<StreamResultArena, SendableError> {
        self.tcp_stream.set_nodelay(self.option.no_delay)?;
        websocket::try_upgrade_arena(self.tcp_stream, self.option, self.peer).await
    }

    #[cfg(feature = "arena")]
    pub async fn parse_request_arena_writer(
        self,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        use crate::helpers::traits::http_stream::StreamHttpArenaWriter;

        Ok(self
            .tcp_stream
            .parse_request_arena_writer(self.option, self.peer)
            .await?)
    }
}

/// HTTP 요청 본문. 헤더만 먼저 파싱한 뒤 stream을 그대로 보유하여
/// 핸들러가 청크 단위로 읽거나 (`read_chunk` / `into_multipart`),
/// 전체를 한 번에 버퍼링 (`bytes(cap)`) 할 수 있다.
///
/// **마이그레이션**: 이전의 `pub bytes: Vec<u8>` 필드는 제거됨.
/// 기존 코드의 `req.body().bytes.as_slice()` 같은 패턴은
/// `req.body_mut().bytes(Some(N)).await?` 로 교체.
pub struct Body {
    /// 헤더 read 시 미리 들어와 있던 body 앞부분 바이트.
    /// 헤더 파싱이 끝난 뒤 \r\n\r\n 이후 남은 바이트가 여기 들어감.
    leftover: Vec<u8>,
    /// 남은 body를 스트리밍할 socket의 read half. None이면 EOF 또는 이미 소비됨.
    stream: Option<OwnedReadHalf>,
    /// Content-Length 헤더 값. `None`이면 헤더가 없거나 chunked.
    /// (0.14.0 현재 chunked transfer encoding은 미지원 — 정확한 길이를 모르면 connection close까지 읽음.)
    content_length: Option<usize>,
    /// 전체에서 이미 핸들러에 반환한 누적 바이트 수.
    consumed: usize,
    /// 광고/실제 body 크기 cap. `bytes(None)` 호출 시 `bytes(max_body_size)` 와 동일.
    max_body_size: Option<usize>,
    /// 요청 클라이언트 IP.
    pub ip: Option<SocketAddr>,
}

impl Body {
    /// 헤더 파싱 직후 스트리밍 가능한 Body를 만든다 (라이브러리 내부용).
    pub(crate) fn new_streaming(
        leftover: Vec<u8>,
        stream: OwnedReadHalf,
        content_length: Option<usize>,
        ip: Option<SocketAddr>,
        max_body_size: Option<usize>,
    ) -> Self {
        Self {
            leftover,
            stream: Some(stream),
            content_length,
            consumed: 0,
            max_body_size,
            ip,
        }
    }

    /// 테스트/internal용: 이미 모든 바이트가 메모리에 있는 Body 생성.
    /// stream은 None이라 read_chunk는 즉시 EOF 반환.
    pub fn from_bytes(bytes: Vec<u8>, ip: Option<SocketAddr>) -> Self {
        let len = bytes.len();
        Self {
            leftover: bytes,
            stream: None,
            content_length: Some(len),
            consumed: 0,
            max_body_size: None,
            ip,
        }
    }

    /// 옵션에서 가져온 max_body_size 설정 (`parse_request` 가 호출).
    pub fn set_max_body_size(&mut self, max: Option<usize>) {
        self.max_body_size = max;
    }

    /// 현재 설정된 max_body_size 캡.
    pub fn max_body_size(&self) -> Option<usize> {
        self.max_body_size
    }

    /// Content-Length 헤더 값 (있을 때).
    pub fn content_length(&self) -> Option<usize> {
        self.content_length
    }

    /// **동기** body 바이트 접근. body가 streaming 모드인 경우, 이미 leftover에
    /// 들어와 있는 바이트만 반환 (실제 전체 body는 `bytes(cap).await?` 호출 필요).
    /// `from_bytes`로 만든 (테스트/internal) Body에서는 전체 바이트가 leftover에 있음.
    ///
    /// 마이그레이션 가이드 (BREAKING): 이전 `req.body().bytes.as_slice()` 코드는
    /// `req.body().buffered_bytes()` 로 임시 대체 가능. 단 streaming 모드에서는
    /// 이 메서드만으로는 데이터를 못 받으니 `req.body_mut().bytes(Some(N)).await?` 사용 권장.
    pub fn buffered_bytes(&self) -> &[u8] {
        &self.leftover
    }

    /// 클라이언트 IP.
    pub fn ip(&self) -> Option<SocketAddr> {
        self.ip
    }

    /// 다음 청크. EOF (Content-Length 도달 or 연결 종료) 시 `None`.
    pub async fn read_chunk(&mut self) -> Result<Option<Bytes>, SendableError> {
        // 1) leftover에 있는 바이트 먼저 소비.
        if !self.leftover.is_empty() {
            let chunk = std::mem::take(&mut self.leftover);
            self.consumed += chunk.len();
            return Ok(Some(Bytes::from(chunk)));
        }

        // 2) Content-Length 도달했으면 종료.
        if let Some(cl) = self.content_length {
            if self.consumed >= cl {
                return Ok(None);
            }
        }

        // 3) socket에서 더 읽기.
        let stream = match self.stream.as_mut() {
            Some(s) => s,
            None => return Ok(None),
        };

        // 남은 만큼만 읽도록 cap. 모르면 64KB.
        let remaining = match self.content_length {
            Some(cl) => cl.saturating_sub(self.consumed).min(64 * 1024),
            None => 64 * 1024,
        };
        if remaining == 0 {
            return Ok(None);
        }

        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; remaining];
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            // 연결 종료
            self.stream = None;
            return Ok(None);
        }
        buf.truncate(n);
        self.consumed += n;
        Ok(Some(Bytes::from(buf)))
    }

    /// 전체 body를 메모리에 버퍼링해 반환. `max` 초과 시 즉시 에러.
    /// `None`이면 무제한 (위험 — 신뢰된 환경에서만).
    pub async fn bytes(&mut self, max: Option<usize>) -> Result<Vec<u8>, SendableError> {
        // Content-Length 광고만으로 cap 초과 즉시 거부 (alloc 회피).
        if let (Some(cl), Some(cap)) = (self.content_length, max) {
            if cl > cap {
                return Err(
                    format!("body too large: content-length={} exceeds cap={}", cl, cap).into(),
                );
            }
        }

        let mut buf = match self.content_length {
            // CL 있고 cap 안이면 정확히 예약 (한 번 alloc).
            Some(cl) if max.map_or(true, |m| cl <= m) => Vec::with_capacity(cl),
            _ => Vec::new(),
        };
        while let Some(chunk) = self.read_chunk().await? {
            if let Some(cap) = max {
                if buf.len() + chunk.len() > cap {
                    return Err(format!(
                        "body too large: exceeded cap={} during streaming read",
                        cap
                    )
                    .into());
                }
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }

    /// body를 `futures::Stream<Item = Result<Bytes, _>>` 로 변환.
    /// multer 등 streaming 파서에 직접 넘길 때 사용.
    pub fn into_stream(
        self,
    ) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
        futures_util::stream::unfold(self, |mut body| async move {
            match body.read_chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), body)),
                Ok(None) => None,
                Err(e) => Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )),
                    body,
                )),
            }
        })
    }

    /// `into_stream()`을 multer Multipart로 감싸서 반환.
    /// 핸들러는 `.next_field()` 로 part 하나씩 청크 단위 처리 가능.
    pub fn into_multipart(self, boundary: String) -> multer::Multipart<'static> {
        multer::Multipart::new(self.into_stream(), boundary)
    }
}

#[cfg(feature = "arena")]
#[repr(align(64))]
pub struct ArenaBody {
    /// Bump allocator를 소유 — drop 시 모든 할당 해제. data_ptr가 가리키는
    /// 메모리의 lifetime은 이 필드의 lifetime과 정확히 일치.
    /// 필드 이동(move) 시에도 Box 내부 주소는 안정적이므로 data_ptr 유효.
    _bump: Box<Bump>,
    /// _bump 내부 할당의 시작 주소. _bump가 살아있는 동안만 dereference 가능.
    data_ptr: *const u8,
    total_len: usize,
    header_end: usize,
    body_start: usize,
    pub ip: Option<SocketAddr>,
}

#[cfg(feature = "arena")]
impl ArenaBody {
    /// Create ArenaBody with per-request Bump allocator
    /// Memory is automatically freed when ArenaBody is dropped
    pub fn new(data: &[u8], header_end: usize, body_start: usize) -> Self {
        let bump = Box::new(Bump::new());
        let allocated_data = bump.alloc_slice_copy(data);
        let data_ptr = allocated_data.as_ptr();
        let total_len = allocated_data.len();

        Self {
            // SAFETY invariant: data_ptr는 `bump` 내부 할당을 가리키며
            // bump를 Box<Bump>로 소유하여 ArenaBody drop 전까지 해제되지 않음.
            // Box를 move 해도 힙 주소 불변이므로 데이터 포인터 안전.
            _bump: bump,
            data_ptr,
            total_len,
            header_end,
            body_start,
            ip: None,
        }
    }

    pub fn get_headers(&self) -> &[u8] {
        // SAFETY: data_ptr는 _bump가 소유하는 할당의 시작이고 total_len 바이트만큼
        // 유효함이 new()에서 보장됨. header_end <= total_len (생성자 호출 측에서 보장).
        // 반환 슬라이스의 lifetime은 &self에 묶여 _bump보다 짧음.
        unsafe { std::slice::from_raw_parts(self.data_ptr, self.header_end) }
    }

    pub fn get_body_bytes(&self) -> &[u8] {
        // SAFETY: data_ptr는 total_len 바이트 유효 영역의 시작.
        // body_start <= total_len 이 생성자 호출 측에서 보장되므로
        // `data_ptr.add(body_start)` 부터 `total_len - body_start` 바이트는 동일 할당 내부.
        unsafe {
            std::slice::from_raw_parts(
                self.data_ptr.add(self.body_start),
                self.total_len - self.body_start,
            )
        }
    }

    pub fn get_body_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.get_body_bytes())
    }

    pub fn len(&self) -> usize {
        self.total_len - self.body_start
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_raw_data(&self) -> &[u8] {
        // SAFETY: get_headers/get_body_bytes와 동일 invariant.
        unsafe { std::slice::from_raw_parts(self.data_ptr, self.total_len) }
    }

    // 제로카피 JSON 파싱 추가
    pub fn parse_json_zero_copy<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let body_str = self.get_body_str()?;
        dev_print!("Arena zero-copy JSON parsing: {} bytes", body_str.len());
        Ok(serde_json::from_str(body_str)?)
    }
}

// SAFETY: ArenaBody는 raw `*const u8`를 보유하지만 그 메모리는 Box<Bump>로
// 소유되어 ArenaBody와 함께 이동/해제됨. 외부에서 동시 접근하는 별칭이 존재하지 않으므로
// 다른 스레드로 이동(Send) 및 공유 참조 (Sync) 모두 안전.
// (Bump 자체는 !Sync지만 ArenaBody 외부에 노출되지 않음.)
#[cfg(feature = "arena")]
unsafe impl Send for ArenaBody {}

#[cfg(feature = "arena")]
unsafe impl Sync for ArenaBody {}

/// 응답 작성기. 0.14.0부터 `stream`은 socket의 write half(`OwnedWriteHalf`)이다.
/// 같은 TCP 연결의 read half는 `Body` 가 보유하며, 응답을 보내기 전에
/// body 스트리밍이 끝나는 것이 일반적인 흐름.
pub struct Writer {
    pub stream: OwnedWriteHalf,
    pub body: String,
    pub bytes: Vec<u8>,
    pub use_file: bool,
    pub options: Arc<Options>,
}

fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

#[cfg(feature = "arena")]
pub struct ArenaWriter {
    pub stream: OwnedWriteHalf,
    _bump: Option<Box<Bump>>,
    response_data_ptr: *const u8,
    response_data_len: usize,
    pub use_file: bool,
    pub options: Arc<Options>,
}

#[cfg(feature = "arena")]
impl ArenaWriter {
    pub fn new(stream: OwnedWriteHalf, options: Arc<Options>) -> Self {
        Self {
            stream,
            _bump: None,
            response_data_ptr: std::ptr::null(),
            response_data_len: 0,
            use_file: false,
            options,
        }
    }

    pub async fn write_arena_bytes(&mut self) -> Result<(), SendableError> {
        if !self.response_data_ptr.is_null() && self.response_data_len > 0 {
            use crate::helpers::traits::http_response::SendBytes;
            let data_ptr = self.response_data_ptr;
            let data_len = self.response_data_len;

            // SAFETY: response_data_ptr/len 은 set_arena_response 에서 _bump 내부 할당으로
            // 채워지며 _bump가 살아있는 동안 유효. &mut self 호출이라 동시 변경 불가.
            let data = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };

            self.stream.send_bytes(data).await?;
        }
        Ok(())
    }

    pub fn set_arena_response(&mut self, data: &str) -> Result<bool, SendableError> {
        let bump = Box::new(Bump::new());
        let allocated_data = bump.alloc_str(data);

        self.response_data_ptr = allocated_data.as_ptr();
        self.response_data_len = allocated_data.len();
        self._bump = Some(bump);
        Ok(true)
    }

    pub fn set_arena_json<T>(&mut self, data: &T) -> Result<bool, SendableError>
    where
        T: serde::Serialize,
    {
        let json_string = serde_json::to_string(data)?;
        self.set_arena_response(&json_string)
    }

    pub fn get_response_data(&self) -> &[u8] {
        if self.response_data_ptr.is_null() || self.response_data_len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.response_data_ptr, self.response_data_len) }
        }
    }

    pub fn get_response_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.get_response_data())
    }

    #[cfg(feature = "response_file")]
    pub fn response_file<P>(&mut self, path: P) -> Result<(), SendableError>
    where
        P: AsRef<Path>,
    {
        use crate::helpers::traits::safe_path_join;
        use percent_encoding::percent_decode_str;

        let root_path = &self.options.root_path;
        let path_ref = path.as_ref().to_str().unwrap_or_default();
        let decoded_path = percent_decode_str(path_ref).decode_utf8_lossy();
        // path traversal 방어 — `..` / 절대 경로는 거부
        let file_path = safe_path_join(root_path, decoded_path.as_ref())
            .ok_or("invalid path: traversal segments are not allowed")?;

        let bump = Box::new(Bump::new());

        // non-UTF-8 경로(Windows 등)에서 panic 회피 — lossy 변환 후 처리
        let path_str_cow = file_path.to_string_lossy();

        if let Ok(metadata) = std::fs::metadata(&file_path) {
            let file_size = metadata.len() as usize;
            if file_size <= self.options.zero_copy_threshold {
                let path_with_marker = format!("__ZERO_COPY_FILE__:{}", path_str_cow);
                let allocated_path = bump.alloc_str(&path_with_marker);

                self.response_data_ptr = allocated_path.as_ptr();
                self.response_data_len = allocated_path.len();
                self._bump = Some(bump);
                self.use_file = true;
                return Ok(());
            }
        }

        // 기존 방식
        let allocated_path = bump.alloc_str(&path_str_cow);

        self.response_data_ptr = allocated_path.as_ptr();
        self.response_data_len = allocated_path.len();
        self._bump = Some(bump);
        self.use_file = true;
        Ok(())
    }
}

// SAFETY: ArenaWriter의 raw pointer는 자체 _bump가 소유한 메모리를 가리키며
// ArenaWriter와 함께 이동/해제됨. 외부 별칭 없음. ArenaBody와 동일 invariant.
#[cfg(feature = "arena")]
unsafe impl Send for ArenaWriter {}

#[cfg(feature = "arena")]
unsafe impl Sync for ArenaWriter {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestData {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub description: String,
    pub tags: Vec<String>,
    pub metadata: Vec<String>,
    pub payload: Vec<u8>,
}

impl TestData {
    pub fn generate(size_kb: usize) -> Self {
        let target_size = size_kb * 1024;

        // 기본 구조체 크기를 고려해서 payload 크기 결정
        let base_size = 200; // 대략적인 기본 크기
        let payload_size = if target_size > base_size {
            target_size - base_size
        } else {
            target_size
        };

        Self {
            id: 12345,
            name: "test_user_with_longer_name_for_realistic_data".to_string(),
            email: "test.user.with.longer.email@example.com".to_string(),
            description: "x".repeat(payload_size / 4), // 일부는 description에
            tags: vec![
                "tag1".to_string(),
                "tag2".to_string(),
                "performance".to_string(),
                "test".to_string(),
                "benchmark".to_string(),
            ],
            metadata: vec![
                "metadata1".to_string(),
                "metadata2".to_string(),
                "some_additional_info".to_string(),
            ],
            payload: vec![0u8; payload_size * 3 / 4], // 대부분은 바이너리 데이터
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), SendableError> {
        let json_str = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json_str)?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, SendableError> {
        parse_json_file(path)
    }
}
