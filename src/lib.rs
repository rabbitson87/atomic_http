use std::path::Path;
use std::{env::current_dir, io, net::SocketAddr, path::PathBuf};

#[cfg(feature = "env")]
use std::str::FromStr;

#[cfg(feature = "arena")]
use bumpalo_herd::{Herd, Member};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod helpers;

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
    parse_json_file, CacheStats, CachedFileData, FileLoadResult, ZeroCopyCache, ZeroCopyFile,
};

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
    #[cfg(feature = "arena")]
    pub use bumpalo_herd;

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

#[cfg(not(feature = "tokio_rustls"))]
use tokio::net::TcpListener;

use tokio::net::TcpStream;
#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;

pub type SendableError = Box<dyn std::error::Error + Send + Sync>;

pub struct Server {
    #[cfg(not(feature = "tokio_rustls"))]
    pub listener: TcpListener,
    pub options: Options,
    #[cfg(feature = "arena")]
    pub herd: Arc<Herd>,
    pub zero_copy_cache: Option<Arc<ZeroCopyCache>>,
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
}

impl Server {
    #[cfg(all(feature = "arena", not(feature = "tokio_rustls")))]
    pub async fn new(address: &str) -> Result<Server, SendableError> {
        let listener = TcpListener::bind(address).await?;
        let zero_copy_cache = Some(Arc::new(ZeroCopyCache::default()));

        dev_print!("✅ Server initialized with Arena support");

        Ok(Server {
            listener,
            options: Options::new(),
            herd: Arc::new(Herd::new()),
            zero_copy_cache,
        })
    }

    #[cfg(all(not(feature = "arena"), not(feature = "tokio_rustls")))]
    pub async fn new(address: &str) -> Result<Server, SendableError> {
        let listener = TcpListener::bind(address).await?;
        let zero_copy_cache = Some(Arc::new(ZeroCopyCache::default()));

        dev_print!("✅ Server initialized with Zero-copy support");

        Ok(Server {
            listener,
            options: Options::new(),
            zero_copy_cache,
        })
    }

    #[cfg(feature = "tokio_rustls")]
    pub async fn new() -> Result<Server, SendableError> {
        let zero_copy_cache = Some(Arc::new(ZeroCopyCache::default()));
        Ok(Server {
            options: Options::new(),
            zero_copy_cache,
        })
    }

    #[cfg(all(feature = "arena", not(feature = "tokio_rustls")))]
    pub async fn accept(&mut self) -> Result<(TcpStream, Options, Arc<Herd>), SendableError> {
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
        Ok((stream, self.options.clone(), self.herd.clone()))
    }

    #[cfg(all(not(feature = "arena"), not(feature = "tokio_rustls")))]
    pub async fn accept(&mut self) -> Result<(TcpStream, Options), SendableError> {
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
        Ok((stream, self.options.clone()))
    }

    #[cfg(not(feature = "tokio_rustls"))]
    pub async fn parse_request(
        stream: TcpStream,
        options: Options,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        Ok(stream.parse_request(&options).await?)
    }

    #[cfg(feature = "tokio_rustls")]
    pub async fn parse_request(
        stream: TlsStream<TcpStream>,
        options: Options,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        let (stream, _connect) = stream.into_inner();
        Ok(stream.parse_request(&options).await?)
    }

    #[cfg(feature = "arena")]
    pub async fn parse_request_arena(
        stream: TcpStream,
        options: Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
        use crate::helpers::traits::http_stream::StreamHttpArena;

        Ok(stream.parse_request_arena(&options, herd).await?)
    }

    #[cfg(feature = "arena")]
    pub async fn parse_request_arena_writer(
        stream: TcpStream,
        options: Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        use crate::helpers::traits::http_stream::StreamHttpArenaWriter;

        Ok(stream.parse_request_arena_writer(&options, herd).await?)
    }

    pub fn set_no_delay(&mut self, no_delay: bool) {
        self.options.no_delay = no_delay;
    }

    #[cfg(feature = "arena")]
    pub fn get_herd(&self) -> &Arc<Herd> {
        &self.herd
    }

    pub fn get_zero_copy_cache(&self) -> Option<&Arc<ZeroCopyCache>> {
        self.zero_copy_cache.as_ref()
    }

    pub fn set_zero_copy_cache(&mut self, cache: ZeroCopyCache) {
        self.zero_copy_cache = Some(Arc::new(cache));
    }

    /// 캐시 통계 출력
    pub fn print_cache_stats(&self) {
        if let Some(cache) = &self.zero_copy_cache {
            let stats = cache.stats();
            println!("📊 {}", stats);
        }
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
    _member: Box<Member<'static>>,
    data_ptr: *const u8,
    total_len: usize,
    header_end: usize,
    body_start: usize,
    pub ip: Option<SocketAddr>,
}

#[cfg(feature = "arena")]
impl ArenaBody {
    // 안전한 생성자
    pub fn new(
        member: Member<'_>,
        allocated_data: &[u8],
        header_end: usize,
        body_start: usize,
    ) -> Self {
        let member_box = unsafe {
            std::mem::transmute::<Box<Member<'_>>, Box<Member<'static>>>(Box::new(member))
        };

        Self {
            _member: member_box,
            data_ptr: allocated_data.as_ptr(),
            total_len: allocated_data.len(),
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
    pub herd: Arc<Herd>,
    _member: Option<Box<Member<'static>>>,
    response_data_ptr: *const u8,
    response_data_len: usize,
    pub use_file: bool,
    pub options: Options,
}

#[cfg(feature = "arena")]
impl ArenaWriter {
    pub fn new(stream: TcpStream, herd: Arc<Herd>, options: Options) -> Self {
        Self {
            stream,
            herd,
            _member: None,
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
        let member = self.herd.get();
        let allocated_data = member.alloc_str(data);

        // SAFETY: Member의 수명을 'static으로 변환하고 포인터로 저장
        let member_box = unsafe {
            std::mem::transmute::<Box<Member<'_>>, Box<Member<'static>>>(Box::new(member))
        };

        self.response_data_ptr = allocated_data.as_ptr();
        self.response_data_len = allocated_data.len();
        self._member = Some(member_box);
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

        let member = self.herd.get();

        if let Ok(metadata) = std::fs::metadata(&file_path) {
            let file_size = metadata.len() as usize;
            if file_size <= self.options.zero_copy_threshold {
                let path_with_marker =
                    format!("__ZERO_COPY_FILE__:{}", file_path.to_str().unwrap());
                let allocated_path = member.alloc_str(&path_with_marker);

                let member_box = unsafe {
                    std::mem::transmute::<Box<Member<'_>>, Box<Member<'static>>>(Box::new(member))
                };

                self.response_data_ptr = allocated_path.as_ptr();
                self.response_data_len = allocated_path.len();
                self._member = Some(member_box);
                self.use_file = true;
                return Ok(());
            }
        }

        // 기존 방식
        let path_str = file_path.to_str().unwrap();
        let allocated_path = member.alloc_str(path_str);

        let member_box = unsafe {
            std::mem::transmute::<Box<Member<'_>>, Box<Member<'static>>>(Box::new(member))
        };

        self.response_data_ptr = allocated_path.as_ptr();
        self.response_data_len = allocated_path.len();
        self._member = Some(member_box);
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
