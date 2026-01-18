use std::path::Path;
use std::{env::current_dir, io, net::SocketAddr, path::PathBuf};

#[cfg(feature = "env")]
use std::str::FromStr;

#[cfg(feature = "arena")]
use bumpalo::Bump;
use serde::{Deserialize, Serialize};
#[cfg(feature = "connection_pool")]
use std::sync::Arc;

pub mod helpers;

#[cfg(feature = "connection_pool")]
pub mod connection_pool;

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

pub mod external {
    pub use async_trait;
    #[cfg(feature = "env")]
    pub use dotenv;
    pub use http;
    #[cfg(feature = "response_file")]
    pub use mime_guess;
    pub use tokio;

    #[cfg(feature = "arena")]
    pub use bumpalo;

    pub use memmap2;
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

use tokio::net::TcpStream;

pub type SendableError = Box<dyn std::error::Error + Send + Sync>;

pub struct Server {
    pub listener: TcpListener,
    pub options: Options,
    #[cfg(feature = "connection_pool")]
    pub connection_pool: Option<Arc<tokio::sync::Mutex<ConnectionPool>>>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub no_delay: bool,
    pub read_timeout_miliseconds: u64,
    pub root_path: PathBuf,
    pub read_buffer_size: usize,
    pub read_max_retry: u8,
    pub read_imcomplete_size: usize,
    pub current_client_addr: Option<SocketAddr>,
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
            read_timeout_miliseconds: 3000,
            root_path: current_dir().unwrap(),
            read_buffer_size: 4096,
            read_max_retry: 3,
            read_imcomplete_size: 0,
            current_client_addr: None,
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

            if let Ok(data) = env::var("READ_TIMEOUT_MILISECONDS") {
                if let Ok(data) = data.parse::<u64>() {
                    _options.read_timeout_miliseconds = data;
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

            if let Ok(data) = env::var("READ_IMCOMPLETE_SIZE") {
                if let Ok(data) = data.parse::<usize>() {
                    _options.read_imcomplete_size = data;
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

    pub fn get_request_ip(&self) -> String {
        match &self.current_client_addr {
            Some(addr) => addr.ip().to_string(),
            None => "".into(),
        }
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
            options: Options::new(),
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
            options,
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
            let mut pool = ConnectionPool::new(self.options.connection_option.clone());
            pool.start_cleanup_task();
            self.connection_pool = Some(Arc::new(tokio::sync::Mutex::new(pool)));
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
        if let Some(pool_arc) = self.connection_pool.take() {
            let mut pool = pool_arc.lock().await;
            pool.shutdown().await;
        }
        dev_print!("🔴 Connection pool disabled for server");
    }

    /// Get connection pool statistics
    #[cfg(feature = "connection_pool")]
    pub async fn get_connection_pool_stats(&self) -> Option<ConnectionStats> {
        if let Some(pool_arc) = &self.connection_pool {
            let pool = pool_arc.lock().await;
            Some(pool.stats())
        } else {
            None
        }
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
        if let Some(pool_arc) = &self.connection_pool {
            let pool = pool_arc.lock().await;
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
        self.options.current_client_addr = Some(addr);
        Ok(Accept::new(stream, self.options.clone()))
    }

    pub fn set_no_delay(&mut self, no_delay: bool) {
        self.options.no_delay = no_delay;
    }

    /// 캐시 통계 출력
    pub fn print_cache_stats(&self) {
        let stats = ZeroCopyCache::global().stats();
        println!("📊 {}", stats);
    }
}

pub struct Accept {
    pub tcp_stream: TcpStream,
    pub option: Options,
}

impl Accept {
    pub fn new(tcp_stream: TcpStream, option: Options) -> Self {
        Self { tcp_stream, option }
    }

    pub async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        Ok(self.tcp_stream.parse_request(&self.option).await?)
    }

    #[cfg(feature = "arena")]
    pub async fn parse_request_arena_writer(
        self,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        use crate::helpers::traits::http_stream::StreamHttpArenaWriter;

        Ok(self
            .tcp_stream
            .parse_request_arena_writer(&self.option)
            .await?)
    }
}

pub struct Body {
    pub bytes: Vec<u8>,
    pub body: String,
    pub len: usize,
    pub ip: Option<SocketAddr>,
}

#[cfg(feature = "arena")]
#[repr(align(64))]
pub struct ArenaBody {
    _bump: Box<Bump>,
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
        // SAFETY: The bump allocator is owned by this struct and will not be dropped
        // until the struct is dropped. The pointer remains valid for the lifetime of the struct.
        let allocated_data = bump.alloc_slice_copy(data);
        let data_ptr = allocated_data.as_ptr();
        let total_len = allocated_data.len();

        Self {
            _bump: bump,
            data_ptr,
            total_len,
            header_end,
            body_start,
            ip: None,
        }
    }

    pub fn get_headers(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data_ptr, self.header_end) }
    }

    pub fn get_body_bytes(&self) -> &[u8] {
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

#[cfg(feature = "arena")]
unsafe impl Send for ArenaBody {}

#[cfg(feature = "arena")]
unsafe impl Sync for ArenaBody {}

pub struct Writer {
    pub stream: TcpStream,
    pub body: String,
    pub bytes: Vec<u8>,
    pub use_file: bool,
    pub options: Options,
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
    pub stream: TcpStream,
    _bump: Option<Box<Bump>>,
    response_data_ptr: *const u8,
    response_data_len: usize,
    pub use_file: bool,
    pub options: Options,
}

#[cfg(feature = "arena")]
impl ArenaWriter {
    pub fn new(stream: TcpStream, options: Options) -> Self {
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
        let root_path = &self.options.root_path;
        let file_path = root_path.join(path);

        let bump = Box::new(Bump::new());

        if let Ok(metadata) = std::fs::metadata(&file_path) {
            let file_size = metadata.len() as usize;
            if file_size <= self.options.zero_copy_threshold {
                let path_with_marker =
                    format!("__ZERO_COPY_FILE__:{}", file_path.to_str().unwrap());
                let allocated_path = bump.alloc_str(&path_with_marker);

                self.response_data_ptr = allocated_path.as_ptr();
                self.response_data_len = allocated_path.len();
                self._bump = Some(bump);
                self.use_file = true;
                return Ok(());
            }
        }

        // 기존 방식
        let path_str = file_path.to_str().unwrap();
        let allocated_path = bump.alloc_str(path_str);

        self.response_data_ptr = allocated_path.as_ptr();
        self.response_data_len = allocated_path.len();
        self._bump = Some(bump);
        self.use_file = true;
        Ok(())
    }
}

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
