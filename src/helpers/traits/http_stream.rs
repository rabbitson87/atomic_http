use crate::dev_print;
use async_trait::async_trait;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{self, AsyncReadExt};
use tokio::net::TcpStream;
#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;

#[cfg(feature = "arena")]
use crate::{ArenaBody, ArenaWriter};
use crate::{Body, Options, SendableError, Writer};

pub struct Form {
    /// 모든 텍스트 필드(`(name, value)`). 이전 0.13.x의 `text: (String, String)`은
    /// 단일 필드만 보관해서 다중 텍스트 필드가 있는 폼에선 마지막 것만 남는 버그가 있었음.
    pub text_fields: Vec<(String, String)>,
    pub parts: Vec<Part>,
}

impl Form {
    pub fn new() -> Self {
        Self {
            text_fields: Vec::new(),
            parts: Vec::new(),
        }
    }

    pub fn add_text_field(&mut self, name: String, value: String) {
        self.text_fields.push((name, value));
    }

    pub fn add_part(&mut self, part: Part) {
        self.parts.push(part);
    }

    /// 이름으로 텍스트 필드 값 찾기 (첫 매치).
    pub fn find_text_field(&self, name: &str) -> Option<&str> {
        self.text_fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct Part {
    pub name: String,
    pub file_name: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Part {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            file_name: String::new(),
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    pub fn set_name(&mut self, name: &mut &str) {
        self.name = std::mem::take(name).into();
    }

    pub fn set_file_name(&mut self, file_name: &mut &str) {
        self.file_name = std::mem::take(file_name).into();
    }
}
#[cfg(feature = "arena")]
pub struct ArenaForm {
    // Arena 메모리 참조를 보관
    _arena_data_ptr: *const u8,
    _arena_data_len: usize,
    pub text_fields: Vec<ArenaTextFieldRef>,
    pub parts: Vec<ArenaPartRef>,
}

#[cfg(feature = "arena")]
pub struct ArenaTextFieldRef {
    name_ptr: *const u8,
    name_len: usize,
    value_ptr: *const u8,
    value_len: usize,
}

#[cfg(feature = "arena")]
pub struct ArenaPartRef {
    name_ptr: *const u8,
    name_len: usize,
    file_name_ptr: *const u8,
    file_name_len: usize,
    content_type_ptr: *const u8,
    content_type_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    pub headers: Vec<ArenaHeaderRef>,
}

#[cfg(feature = "arena")]
pub struct ArenaHeaderRef {
    key_ptr: *const u8,
    key_len: usize,
    value_ptr: *const u8,
    value_len: usize,
}

#[cfg(feature = "arena")]
impl ArenaForm {
    pub fn new(arena_data: &[u8]) -> Self {
        Self {
            _arena_data_ptr: arena_data.as_ptr(),
            _arena_data_len: arena_data.len(),
            text_fields: Vec::new(),
            parts: Vec::new(),
        }
    }

    // 안전한 문자열 접근 메서드들
    pub fn get_text_field_name(&self, index: usize) -> Option<&str> {
        self.text_fields
            .get(index)
            .and_then(|field| field.get_name())
    }

    pub fn get_text_field_value(&self, index: usize) -> Option<&str> {
        self.text_fields
            .get(index)
            .and_then(|field| field.get_value())
    }

    pub fn find_text_field(&self, name: &str) -> Option<&str> {
        self.text_fields
            .iter()
            .find(|field| field.get_name().map_or(false, |n| n == name))
            .and_then(|field| field.get_value())
    }

    pub fn get_part(&self, index: usize) -> Option<&ArenaPartRef> {
        self.parts.get(index)
    }

    pub fn find_file_part(&self, name: &str) -> Option<&ArenaPartRef> {
        self.parts
            .iter()
            .find(|part| part.get_name().map_or(false, |n| n == name))
    }
}

#[cfg(feature = "arena")]
impl ArenaTextFieldRef {
    pub fn new(name_slice: &[u8], value_slice: &[u8]) -> Self {
        Self {
            name_ptr: name_slice.as_ptr(),
            name_len: name_slice.len(),
            value_ptr: value_slice.as_ptr(),
            value_len: value_slice.len(),
        }
    }

    pub fn get_name(&self) -> Option<&str> {
        if self.name_ptr.is_null() || self.name_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.name_ptr, self.name_len);
            std::str::from_utf8(slice).ok()
        }
    }

    pub fn get_value(&self) -> Option<&str> {
        if self.value_ptr.is_null() || self.value_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.value_ptr, self.value_len);
            std::str::from_utf8(slice).ok()
        }
    }
}

#[cfg(feature = "arena")]
impl ArenaPartRef {
    pub fn new(body_slice: &[u8]) -> Self {
        Self {
            name_ptr: std::ptr::null(),
            name_len: 0,
            file_name_ptr: std::ptr::null(),
            file_name_len: 0,
            content_type_ptr: std::ptr::null(),
            content_type_len: 0,
            body_ptr: body_slice.as_ptr(),
            body_len: body_slice.len(),
            headers: Vec::new(),
        }
    }

    pub fn set_name(&mut self, name_slice: &[u8]) {
        self.name_ptr = name_slice.as_ptr();
        self.name_len = name_slice.len();
    }

    pub fn set_file_name(&mut self, filename_slice: &[u8]) {
        self.file_name_ptr = filename_slice.as_ptr();
        self.file_name_len = filename_slice.len();
    }

    pub fn set_content_type(&mut self, content_type_slice: &[u8]) {
        self.content_type_ptr = content_type_slice.as_ptr();
        self.content_type_len = content_type_slice.len();
    }

    pub fn get_name(&self) -> Option<&str> {
        if self.name_ptr.is_null() || self.name_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.name_ptr, self.name_len);
            std::str::from_utf8(slice).ok()
        }
    }

    pub fn get_file_name(&self) -> Option<&str> {
        if self.file_name_ptr.is_null() || self.file_name_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.file_name_ptr, self.file_name_len);
            std::str::from_utf8(slice).ok()
        }
    }

    pub fn get_content_type(&self) -> Option<&str> {
        if self.content_type_ptr.is_null() || self.content_type_len == 0 {
            return Some("application/octet-stream"); // 기본값
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.content_type_ptr, self.content_type_len);
            std::str::from_utf8(slice).ok()
        }
    }

    pub fn get_body(&self) -> &[u8] {
        if self.body_ptr.is_null() || self.body_len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.body_ptr, self.body_len) }
    }

    pub fn find_header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.get_key().map_or(false, |k| k.eq_ignore_ascii_case(name)))
            .and_then(|h| h.get_value())
    }
}

#[cfg(feature = "arena")]
impl ArenaHeaderRef {
    pub fn new(key_slice: &[u8], value_slice: &[u8]) -> Self {
        Self {
            key_ptr: key_slice.as_ptr(),
            key_len: key_slice.len(),
            value_ptr: value_slice.as_ptr(),
            value_len: value_slice.len(),
        }
    }

    pub fn get_key(&self) -> Option<&str> {
        if self.key_ptr.is_null() || self.key_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.key_ptr, self.key_len);
            std::str::from_utf8(slice).ok()
        }
    }

    pub fn get_value(&self) -> Option<&str> {
        if self.value_ptr.is_null() || self.value_len == 0 {
            return None;
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.value_ptr, self.value_len);
            std::str::from_utf8(slice).ok()
        }
    }
}

// SAFETY: 이 타입들의 raw `*const u8` 필드는 모두
// `ArenaBody._bump` (Box<Bump>) 내부 할당을 가리킴.
// 사용 규약: 동일 요청 처리 중 ArenaBody가 살아있는 한 ArenaForm/PartRef/TextFieldRef/HeaderRef
// 가 가리키는 메모리는 유효. ArenaBody와 같은 lifetime/스레드로 함께 이동되므로
// dangling 위험 없음. raw pointer 자체는 !Send/!Sync 이지만 위 규약을 사용자가 지키면 안전.
#[cfg(feature = "arena")]
unsafe impl Send for ArenaForm {}
#[cfg(feature = "arena")]
unsafe impl Sync for ArenaForm {}
#[cfg(feature = "arena")]
unsafe impl Send for ArenaPartRef {}
#[cfg(feature = "arena")]
unsafe impl Sync for ArenaPartRef {}
#[cfg(feature = "arena")]
unsafe impl Send for ArenaTextFieldRef {}
#[cfg(feature = "arena")]
unsafe impl Sync for ArenaTextFieldRef {}
#[cfg(feature = "arena")]
unsafe impl Send for ArenaHeaderRef {}
#[cfg(feature = "arena")]
unsafe impl Sync for ArenaHeaderRef {}

#[async_trait]
pub trait StreamHttp {
    /// **buffered** body 파싱: 헤더+body 전체를 메모리에 읽은 뒤 Request 반환.
    /// 작은 JSON/form 요청에 적합. `max_body_size` 가 OOM 방어를 강제.
    async fn parse_request(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError>;

    /// **streaming** body 파싱 (0.14.0 신규): 헤더만 읽고 socket의 read half를
    /// `Body` 에 부착하여 반환. 핸들러는 `body.read_chunk()` / `body.into_multipart()` /
    /// `body.into_stream()` 으로 청크 단위 처리 가능. 대용량 업로드(파일/multipart)에 적합.
    ///
    /// `body.bytes(cap)` 으로 전체 버퍼링도 가능하지만, 그 경우엔 그냥 `parse_request` 가 더 단순.
    async fn parse_request_streaming(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError>;
}

#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(self, &options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(
            request, stream, options, peer,
        )?)
    }

    async fn parse_request_streaming(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let HeaderReadResult {
            header_bytes,
            leftover,
            content_length,
            stream,
        } = read_headers_only(self, &options).await?;

        // 1) 헤더만으로 Request<Body> 빌드 (parser는 body 부분 비어있어도 OK)
        let request_buffered = get_request(header_bytes).await?;
        let (parts, _empty_body) = request_buffered.into_parts();

        // 2) stream을 split — read half는 Body로, write half는 Writer로
        let (read_half, write_half) = stream.into_split();

        // 3) Body를 streaming 모드로 재구성
        let streaming_body = Body::new_streaming(
            leftover,
            read_half,
            content_length,
            Some(peer),
            options.max_body_size,
        );
        let request = Request::from_parts(parts, streaming_body);
        let version = request.version();

        // 4) Writer는 write half + 빈 응답
        Ok((
            request,
            Response::builder()
                .version(version)
                .header(CONTENT_TYPE, "application/json")
                .status(400)
                .body(Writer {
                    stream: write_half,
                    body: String::new(),
                    bytes: vec![],
                    use_file: false,
                    options,
                })?,
        ))
    }
}

#[cfg(feature = "tokio_rustls")]
#[async_trait]
impl StreamHttp for TlsStream<TcpStream> {
    async fn parse_request(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        let stream = self.into_inner().0;
        stream.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(stream, &options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(
            request, stream, options, peer,
        )?)
    }

    async fn parse_request_streaming(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        // TLS 변형은 TcpStream::into_split이 불가하므로 (TlsStream이 감싸고 있어)
        // 평문 TcpStream 추출 후 동일 처리. 다만 TLS 위에서 body streaming 의 보안 의미는
        // 별도 검토 필요 (헤더는 TLS로 보호되지만 body 청크 read도 TLS 통과해야 함).
        // 현재 구현은 TLS 종료 후 평문 streaming → TLS 보호 외 영역으로 데이터 흐름.
        // TLS streaming이 필요하면 추후 별도 API 추가.
        let stream = self.into_inner().0;
        StreamHttp::parse_request_streaming(stream, options, peer).await
    }
}

pub(crate) fn get_parse_result_from_request(
    mut request: Request<Body>,
    stream: TcpStream,
    options: Arc<Options>,
    peer: SocketAddr,
) -> Result<(Request<Body>, Response<Writer>), SendableError> {
    let version = request.version();
    request.body_mut().ip = Some(peer);

    // 0.14.0: body는 이미 buffered 모드로 다 읽혔으므로 read half는 버리고
    // write half 만 Writer 로. (streaming 경로는 parse_request_streaming 별도 함수.)
    let (_read_half, write_half) = stream.into_split();

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(Writer {
                stream: write_half,
                body: String::new(),
                bytes: vec![],
                use_file: false,
                options,
            })?,
    ))
}
/// `read_headers_only` 결과 — 헤더 부분 + body 앞부분으로 미리 들어온 leftover +
/// Content-Length(있을 때) + 후속 stream.
pub(crate) struct HeaderReadResult {
    pub header_bytes: Vec<u8>,
    pub leftover: Vec<u8>,
    pub content_length: Option<usize>,
    pub stream: TcpStream,
}

/// `read_headers_only` 이후 남은 body를 마저 읽어 단일 Vec로 반환.
/// `parse_request_auto` 의 arena 경로에서 사용 (작은 요청은 통째 buffered 가 필요).
pub(crate) async fn read_remaining_body(
    initial_leftover: Vec<u8>,
    mut stream: TcpStream,
    content_length: Option<usize>,
    options: &Options,
) -> Result<(Vec<u8>, TcpStream), SendableError> {
    const BODY_READ_CHUNK: usize = 64 * 1024;
    let total_expected = content_length.unwrap_or(0);
    let mut final_buffer = initial_leftover;
    let mut total_read = final_buffer.len();
    let read_timeout = Duration::from_millis(options.read_timeout_milliseconds);
    let mut retry_count: u8 = 0;

    while total_read < total_expected && retry_count < options.read_max_retry {
        let remaining = total_expected - total_read;
        let chunk_size = remaining.min(BODY_READ_CHUNK);
        if final_buffer.len() < total_read + chunk_size {
            final_buffer.resize(total_read + chunk_size, 0);
        }
        match tokio::time::timeout(
            read_timeout,
            stream.read(&mut final_buffer[total_read..total_read + chunk_size]),
        )
        .await
        {
            Ok(Ok(n)) => {
                if n == 0 {
                    break;
                }
                total_read += n;
                retry_count = 0;
            }
            _ => {
                retry_count += 1;
            }
        }
    }
    final_buffer.truncate(total_read);
    Ok((final_buffer, stream))
}

/// **헤더만 읽고** body는 socket에 그대로 남겨둔 채 반환한다.
/// `parse_request_streaming` 전용. body 부분이 미리 들어와 있으면 `leftover` 로 분리.
pub(crate) async fn read_headers_only(
    mut stream: TcpStream,
    options: &Options,
) -> Result<HeaderReadResult, SendableError> {
    const MAX_HEADER_SIZE: usize = 64 * 1024;
    const INITIAL_READ_SIZE: usize = 4096;
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    let initial_read = match options.read_buffer_size {
        0 => INITIAL_READ_SIZE,
        n => n,
    };

    let mut buffer: Vec<u8> = Vec::with_capacity(initial_read);
    let mut content_length = None;
    let mut header_end_pos: Option<usize> = None;
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;
    let read_timeout = Duration::from_millis(options.read_timeout_milliseconds);
    let header_deadline_ms = options
        .header_read_deadline_ms
        .unwrap_or_else(|| options.read_timeout_milliseconds * (max_retry as u64 + 1));
    let header_start = Instant::now();

    while header_end_pos.is_none() && buffer.len() < MAX_HEADER_SIZE && retry_count < max_retry {
        if header_start.elapsed().as_millis() as u64 > header_deadline_ms {
            return Err(format!(
                "Header read deadline exceeded ({} ms) — possible slowloris",
                header_deadline_ms
            )
            .into());
        }
        let current_len = buffer.len();
        let chunk = if current_len == 0 {
            initial_read
        } else {
            determine_next_read_size(current_len)
        };
        let new_size = (current_len + chunk).min(MAX_HEADER_SIZE);
        buffer.resize(new_size, 0);

        match tokio::time::timeout(read_timeout, stream.read(&mut buffer[current_len..])).await {
            Ok(Ok(n)) => {
                if n == 0 {
                    buffer.truncate(current_len);
                    break;
                }
                let new_len = current_len + n;
                buffer.truncate(new_len);

                let search_start = current_len.saturating_sub(HEADER_END_MARKER.len() - 1);
                if let Some(rel) = find_header_end_optimized(&buffer[search_start..]) {
                    let end = search_start + rel + 4;
                    header_end_pos = Some(end);
                    content_length = extract_content_length_simple(&buffer[..end]);
                    break;
                }
                retry_count = 0;
            }
            Ok(Err(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                buffer.truncate(current_len);
                continue;
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                buffer.truncate(current_len);
                retry_count += 1;
                continue;
            }
        }
    }

    if buffer.is_empty() {
        return Err("No data received".into());
    }

    let header_end = match header_end_pos {
        Some(p) => p,
        None => return Err("Headers incomplete: \\r\\n\\r\\n not found".into()),
    };

    // 광고된 Content-Length가 cap을 넘으면 streaming 시작 전에 거부.
    if let (Some(cl), Some(cap)) = (content_length, options.max_body_size) {
        if cl > cap {
            return Err(format!(
                "Request body too large: content-length={} exceeds max_body_size={}",
                cl, cap
            )
            .into());
        }
    }

    // 헤더 read 시 body 앞부분이 같이 들어왔을 수 있음 → leftover로 분리
    let leftover = buffer.split_off(header_end);
    let header_bytes = buffer;

    Ok(HeaderReadResult {
        header_bytes,
        leftover,
        content_length,
        stream,
    })
}

pub(crate) async fn get_bytes_from_reader(
    mut stream: TcpStream,
    options: &Options,
) -> Result<(Vec<u8>, TcpStream), SendableError> {
    const MAX_HEADER_SIZE: usize = 64 * 1024; // 64KB 헤더 cap (RFC 표준 헤더 한도)
    const INITIAL_READ_SIZE: usize = 4096; // 첫 읽기 4KB
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    // 사용자 설정값이 있으면 그것을 첫 읽기로, 아니면 4KB.
    // MAX_HEADER_SIZE는 cap에만 사용 (이전: min으로도 작동해 매 요청 64KB zero-init).
    let initial_read = match options.read_buffer_size {
        0 => INITIAL_READ_SIZE,
        n => n,
    };

    // 1단계: 점진적 grow로 헤더 읽기 (작은 요청은 4KB 1회로 끝)
    let mut header_buffer: Vec<u8> = Vec::with_capacity(initial_read);
    let mut content_length = None;
    let mut header_end_pos = None;
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;
    let read_timeout = Duration::from_millis(options.read_timeout_milliseconds);

    // Slowloris 방어: 헤더 수신 전체에 절대 deadline.
    // 옵션 미설정 시 read_timeout * (max_retry + 1)로 자동 산출.
    let header_deadline_ms = options
        .header_read_deadline_ms
        .unwrap_or_else(|| options.read_timeout_milliseconds * (max_retry as u64 + 1));
    let header_start = Instant::now();

    while header_end_pos.is_none()
        && header_buffer.len() < MAX_HEADER_SIZE
        && retry_count < max_retry
    {
        if header_start.elapsed().as_millis() as u64 > header_deadline_ms {
            return Err(format!(
                "Header read deadline exceeded ({} ms) — possible slowloris",
                header_deadline_ms
            )
            .into());
        }
        let current_len = header_buffer.len();
        let chunk = if current_len == 0 {
            initial_read
        } else {
            determine_next_read_size(current_len)
        };
        let new_size = (current_len + chunk).min(MAX_HEADER_SIZE);
        header_buffer.resize(new_size, 0);

        match tokio::time::timeout(read_timeout, stream.read(&mut header_buffer[current_len..]))
            .await
        {
            Ok(read_result) => match read_result {
                Ok(n) => {
                    if n == 0 {
                        header_buffer.truncate(current_len);
                        break;
                    }

                    let new_len = current_len + n;
                    header_buffer.truncate(new_len);

                    // 새로 추가된 부분부터 헤더 끝 찾기 (중복 스캔 방지)
                    let search_start = current_len.saturating_sub(HEADER_END_MARKER.len() - 1);
                    if let Some(relative_pos) =
                        find_header_end_optimized(&header_buffer[search_start..])
                    {
                        let end = search_start + relative_pos + 4; // +4 for "\r\n\r\n"
                        header_end_pos = Some(end);
                        content_length = extract_content_length_simple(&header_buffer[..end]);
                        break;
                    }
                    retry_count = 0;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    header_buffer.truncate(current_len);
                    continue;
                }
                Err(e) => return Err(e.into()),
            },
            Err(_) => {
                header_buffer.truncate(current_len);
                retry_count += 1;
                continue;
            }
        }
    }

    if header_buffer.is_empty() {
        return Err("No data received".into());
    }

    let header_end = header_end_pos.unwrap_or(header_buffer.len());

    // DoS 방어: 광고된 Content-Length가 cap을 넘으면 즉시 거부 (대용량 alloc 방지)
    if let (Some(cl), Some(cap)) = (content_length, options.max_body_size) {
        if cl > cap {
            return Err(format!(
                "Request body too large: content-length={} exceeds max_body_size={}",
                cl, cap
            )
            .into());
        }
    }

    // 2단계: 전체 크기 계산. 사전 alloc 하지 않고 들어오는 만큼만 점진 grow.
    // (Content-Length 만큼 미리 alloc 하면 100GB CL 헤더만으로 OOM 가능.)
    let total_expected_size = header_end + content_length.unwrap_or(0);
    let mut final_buffer = header_buffer;

    // 3단계: 남은 바디 데이터 읽기 (64KB 청크 단위 점진 grow)
    const BODY_READ_CHUNK: usize = 64 * 1024;
    let mut total_read = final_buffer.len();
    while total_read < total_expected_size && retry_count < options.read_max_retry {
        let remaining = total_expected_size - total_read;
        let chunk_size = remaining.min(BODY_READ_CHUNK);

        // 필요하면 버퍼 확장
        if final_buffer.len() < total_read + chunk_size {
            final_buffer.resize(total_read + chunk_size, 0);
        }

        match tokio::time::timeout(
            read_timeout,
            stream.read(&mut final_buffer[total_read..total_read + chunk_size]),
        )
        .await
        {
            Ok(Ok(n)) => {
                if n == 0 {
                    break; // 연결 종료
                }
                total_read += n;
                retry_count = 0;
            }
            _ => {
                retry_count += 1;
            }
        }
    }

    // 실제 읽은 크기로 조정
    final_buffer.truncate(total_read);

    dev_print!(
        "HTTP 파싱 완료: 헤더={}B, 바디={}B, 총={}B",
        header_end,
        total_read.saturating_sub(header_end),
        total_read
    );

    Ok((final_buffer, stream))
}

/// 헤더 읽기 시 다음 read 청크 크기. 점진 grow로 작은 요청은 4KB 1회로 끝.
fn determine_next_read_size(current_size: usize) -> usize {
    match current_size {
        0..=512 => 1024,      // 작은 헤더: 1KB 단위
        513..=4096 => 2048,   // 보통 헤더: 2KB 단위
        4097..=16384 => 4096, // 큰 헤더: 4KB 단위
        _ => 8192,            // 매우 큰 헤더: 8KB 단위
    }
}

// 헤더 끝 찾기는 `bytes::find_header_end`로 통합됨.
// 기존 외부 호출자(websocket.rs)는 이 모듈에서 임포트 가능하도록 re-export 유지.
pub(crate) use crate::helpers::traits::bytes::find_header_end as find_header_end_optimized;

// ✅ Content-Length 추출 — 라인 단위 스캔 (O(n))
// HTTP 헤더는 \r\n 구분이므로 각 라인의 시작 위치에서만 prefix 비교.
// 이전의 모든 오프셋에서 풀 needle을 비교하던 O(n·m) 패턴을 제거.
fn extract_content_length_simple(headers: &[u8]) -> Option<usize> {
    const PATTERN: &[u8] = b"content-length:";

    let mut line_start = 0;
    while line_start + PATTERN.len() <= headers.len() {
        // 라인 끝 찾기 (\n까지)
        let mut line_end = line_start;
        while line_end < headers.len() && headers[line_end] != b'\n' {
            line_end += 1;
        }

        // prefix가 PATTERN과 일치하는지 무할당 case-insensitive 비교
        let line_prefix = &headers[line_start..line_start + PATTERN.len()];
        let mut matched = true;
        for j in 0..PATTERN.len() {
            // PATTERN은 이미 소문자, 입력만 to_ascii_lowercase
            if line_prefix[j].to_ascii_lowercase() != PATTERN[j] {
                matched = false;
                break;
            }
        }

        if matched {
            // 값 추출: PATTERN 이후 공백/탭 스킵 → 숫자 슬라이스
            let mut pos = line_start + PATTERN.len();
            while pos < line_end && (headers[pos] == b' ' || headers[pos] == b'\t') {
                pos += 1;
            }
            let number_start = pos;
            while pos < line_end && headers[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos > number_start {
                // 숫자만 있으므로 from_utf8 안전, 직접 파싱
                let mut value: usize = 0;
                for &b in &headers[number_start..pos] {
                    value = value.checked_mul(10)?.checked_add((b - b'0') as usize)?;
                }
                return Some(value);
            }
            return None;
        }

        // 다음 라인으로 (line_end가 끝을 가리키면 종료)
        if line_end >= headers.len() {
            break;
        }
        line_start = line_end + 1;
    }

    None
}

/// httparse 기반 헤더 슬롯 개수.
/// 일반적인 브라우저 요청은 20개 내외, 여유 있게 64개로 설정.
const MAX_PARSED_HEADERS: usize = 64;

pub(crate) async fn get_request(bytes: Vec<u8>) -> Result<Request<Body>, SendableError> {
    dev_print!("bytes len: {:?}", &bytes.len());

    // httparse로 헤더 파싱 (zero-copy 슬라이스 반환)
    let mut builder = Request::builder();
    let body_start: usize;
    {
        let mut headers_buf = [httparse::EMPTY_HEADER; MAX_PARSED_HEADERS];
        let mut req = httparse::Request::new(&mut headers_buf);
        let status = req.parse(&bytes)?;

        body_start = match status {
            httparse::Status::Complete(n) => n,
            // 헤더가 불완전한 경우에도 best-effort로 처리 (기존 동작 유지)
            httparse::Status::Partial => bytes.len(),
        };

        if let Some(m) = req.method {
            if let Ok(method) = m.parse::<http::Method>() {
                builder = builder.method(method);
            }
        }
        if let Some(p) = req.path {
            if let Ok(uri) = p.parse::<http::Uri>() {
                builder = builder.uri(uri);
            } else {
                builder = builder.uri("/");
            }
        } else {
            builder = builder.uri("/");
        }
        builder = builder.version(match req.version {
            Some(0) => http::Version::HTTP_10,
            Some(1) => http::Version::HTTP_11,
            _ => http::Version::HTTP_11,
        });

        // httparse는 빈 슬롯을 끝에 둠 (h.name.is_empty()로 판정)
        for h in req.headers.iter() {
            if h.name.is_empty() {
                break;
            }
            // header() 내부에서 HeaderName/HeaderValue로 복사됨 (req drop 안전)
            builder = builder.header(h.name, h.value);
        }
    } // req drop → bytes 차용 해제

    // 바디는 split_off로 복사 없이 분리
    let mut owned = bytes;
    let body_bytes = if body_start < owned.len() {
        owned.split_off(body_start)
    } else {
        Vec::new()
    };

    // parse_http_request 호출 시점에는 still buffered (전체 body가 메모리에 있음).
    // parse_request_streaming 경로에서는 별도 함수가 body 를 streaming Body 로 생성.
    let request = builder.body(Body::from_bytes(body_bytes, None))?;

    Ok(request)
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait StreamHttpArena {
    async fn parse_request_arena(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArena for TcpStream {
    async fn parse_request_arena(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, &options).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena(request, stream, options, peer)?)
    }
}

#[cfg(feature = "arena")]
pub(crate) async fn get_bytes_arena_direct(
    mut stream: TcpStream,
    options: &Options,
) -> Result<(ArenaBody, TcpStream), SendableError> {
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    const MAX_HEADER_SIZE: usize = 64 * 1024;
    const INITIAL_ARENA_SIZE: usize = 4096;
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    // 첫 읽기 크기만 사용자 설정 반영, 64KB 강제 cap 제거 (사전 malloc 회피).
    let initial_read = match options.read_buffer_size {
        0 => INITIAL_ARENA_SIZE,
        n => n,
    };

    // 1단계: 헤더만 먼저 읽어서 Content-Length 파악
    let mut temp_header_buf: Vec<u8> = Vec::with_capacity(initial_read);
    let mut header_end_pos = None;
    let mut content_length = None;
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;
    let read_timeout = Duration::from_millis(options.read_timeout_milliseconds);

    // Slowloris 방어: 헤더 수신 전체 deadline
    let header_deadline_ms = options
        .header_read_deadline_ms
        .unwrap_or_else(|| options.read_timeout_milliseconds * (max_retry as u64 + 1));
    let header_start = Instant::now();

    // 헤더 읽기 및 파싱
    while header_end_pos.is_none()
        && temp_header_buf.len() < MAX_HEADER_SIZE
        && retry_count < max_retry
    {
        if header_start.elapsed().as_millis() as u64 > header_deadline_ms {
            return Err(format!(
                "Header read deadline exceeded ({} ms) — possible slowloris",
                header_deadline_ms
            )
            .into());
        }
        let next_chunk_size = determine_next_read_size(temp_header_buf.len());
        let current_len = temp_header_buf.len();
        temp_header_buf.resize(current_len + next_chunk_size, 0);

        match tokio::time::timeout(
            read_timeout,
            stream.read(&mut temp_header_buf[current_len..]),
        )
        .await
        {
            Ok(read_result) => match read_result {
                Ok(n) => {
                    if n == 0 {
                        temp_header_buf.truncate(current_len);
                        break;
                    }

                    let new_len = current_len + n;
                    temp_header_buf.truncate(new_len);

                    // 헤더 끝 찾기
                    let search_start = current_len.saturating_sub(HEADER_END_MARKER.len() - 1);
                    if let Some(pos) = find_header_end_optimized(&temp_header_buf[search_start..]) {
                        let end = search_start + pos + 4;
                        header_end_pos = Some(end);
                        content_length = extract_content_length_simple(&temp_header_buf[..end]);
                        break;
                    }
                    retry_count = 0;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    temp_header_buf.truncate(current_len);
                    continue;
                }
                Err(e) => return Err(e.into()),
            },
            Err(_) => {
                temp_header_buf.truncate(current_len);
                retry_count += 1;
                continue;
            }
        }
    }

    let header_end = header_end_pos.unwrap_or(temp_header_buf.len());

    // DoS 방어: 광고된 Content-Length가 cap 초과 시 즉시 거부
    if let (Some(cl), Some(cap)) = (content_length, options.max_body_size) {
        if cl > cap {
            return Err(format!(
                "Request body too large: content-length={} exceeds max_body_size={}",
                cl, cap
            )
            .into());
        }
    }

    let total_size = header_end + content_length.unwrap_or(0);

    // 2단계: 사전 alloc 하지 않고 들어오는 만큼만 점진 grow.
    // (Content-Length 만큼 미리 alloc 하면 100GB CL 헤더만으로 OOM 가능.)
    let mut final_buffer = temp_header_buf;

    // 3단계: 남은 바디 데이터 읽기
    let mut total_read = final_buffer.len();
    retry_count = 0;

    // 바디 읽기 청크 (read_buffer_size=0인 경우 0-byte read로 무한 루프 방지)
    const BODY_READ_CHUNK: usize = 64 * 1024;
    let body_chunk = if options.read_buffer_size == 0 {
        BODY_READ_CHUNK
    } else {
        options.read_buffer_size
    };
    while total_read < total_size && retry_count < options.read_max_retry {
        let remaining = total_size - total_read;
        let chunk_size = remaining.min(body_chunk);

        if final_buffer.len() < total_read + chunk_size {
            final_buffer.resize(total_read + chunk_size, 0);
        }

        match tokio::time::timeout(
            read_timeout,
            stream.read(&mut final_buffer[total_read..total_read + chunk_size]),
        )
        .await
        {
            Ok(Ok(n)) => {
                if n == 0 {
                    break;
                }
                total_read += n;
                retry_count = 0;
            }
            _ => {
                retry_count += 1;
            }
        }
    }

    final_buffer.truncate(total_read);

    // 4단계: ArenaBody 생성 (per-request Bump 사용)
    let body_start = header_end;
    let arena_body = ArenaBody::new(&final_buffer, header_end, body_start);

    dev_print!(
        "Arena HTTP 파싱 완료: 헤더={}B, 바디={}B, 총={}B (Per-request Arena)",
        header_end,
        total_read.saturating_sub(header_end),
        total_read
    );

    Ok((arena_body, stream))
}

#[cfg(feature = "arena")]
pub(crate) fn parse_http_request_arena(
    mut body: ArenaBody,
) -> Result<Request<ArenaBody>, SendableError> {
    let mut builder = Request::builder();
    {
        // get_headers()는 \r\n\r\n 마커를 포함한 헤더 슬라이스를 반환하므로
        // httparse가 Status::Complete를 반환할 수 있음.
        let headers_bytes = body.get_headers();
        let mut headers_buf = [httparse::EMPTY_HEADER; MAX_PARSED_HEADERS];
        let mut req = httparse::Request::new(&mut headers_buf);
        let _ = req.parse(headers_bytes)?;

        if let Some(m) = req.method {
            if let Ok(method) = m.parse::<http::Method>() {
                builder = builder.method(method);
            }
        }
        if let Some(p) = req.path {
            if let Ok(uri) = p.parse::<http::Uri>() {
                builder = builder.uri(uri);
            } else {
                builder = builder.uri("/");
            }
        } else {
            builder = builder.uri("/");
        }
        builder = builder.version(match req.version {
            Some(0) => http::Version::HTTP_10,
            Some(1) => http::Version::HTTP_11,
            _ => http::Version::HTTP_11,
        });

        for h in req.headers.iter() {
            if h.name.is_empty() {
                break;
            }
            builder = builder.header(h.name, h.value);
        }
    } // req drop → body 차용 해제

    body.ip = None; // 이후에 설정됨
    let request = builder.body(body)?;
    Ok(request)
}

#[cfg(feature = "arena")]
fn get_parse_result_arena(
    mut request: Request<ArenaBody>,
    stream: TcpStream,
    options: Arc<Options>,
    peer: SocketAddr,
) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
    let version = request.version();
    request.body_mut().ip = Some(peer);

    let (_read_half, write_half) = stream.into_split();

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(Writer {
                stream: write_half,
                body: String::new(),
                bytes: vec![],
                use_file: false,
                options,
            })?,
    ))
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait StreamHttpArenaWriter {
    async fn parse_request_arena_writer(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArenaWriter for TcpStream {
    async fn parse_request_arena_writer(
        self,
        options: Arc<Options>,
        peer: SocketAddr,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, &options).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena_writer(
            request, stream, options, peer,
        )?)
    }
}

#[cfg(feature = "arena")]
pub(crate) fn get_parse_result_arena_writer(
    mut request: Request<ArenaBody>,
    stream: TcpStream,
    options: Arc<Options>,
    peer: SocketAddr,
) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
    let version = request.version();
    request.body_mut().ip = Some(peer);

    let (_read_half, write_half) = stream.into_split();

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(ArenaWriter::new(write_half, options))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_end_detection() {
        let test_cases = vec![
            // 작은 헤더: "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n"
            // \r\n\r\n 시작 위치 = 33 (15 + 2 + 17 - 1 = index 33)
            (b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n".as_slice(), Some(33)),

            // 일반적인 헤더
            (b"POST /api HTTP/1.1\r\nHost: example.com\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n".as_slice(), Some(90)),

            // 헤더 끝이 없는 경우
            (b"GET / HTTP/1.1\r\nHost: example.com\r\n".as_slice(), None),

            // 빈 데이터
            (b"".as_slice(), None),
        ];

        for (data, expected) in test_cases {
            let result = find_header_end_optimized(data);
            assert_eq!(
                result,
                expected,
                "Failed for data: {:?}",
                std::str::from_utf8(data)
            );
        }
    }

    #[test]
    fn test_content_length_extraction() {
        let headers = b"GET / HTTP/1.1\r\nHost: example.com\r\nContent-Length: 1234\r\nUser-Agent: test\r\n\r\n";
        let result = extract_content_length_simple(headers);
        assert_eq!(result, Some(1234));

        // 대소문자 혼합
        let headers2 = b"GET / HTTP/1.1\r\nHost: example.com\r\nContent-LENGTH: 5678\r\n\r\n";
        let result2 = extract_content_length_simple(headers2);
        assert_eq!(result2, Some(5678));

        // 없는 경우
        let headers3 = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let result3 = extract_content_length_simple(headers3);
        assert_eq!(result3, None);
    }

    // 실제 TCP 소켓 페어 — get_bytes_from_reader는 진짜 TcpStream 만 받기 때문에
    // mock 대신 임시 포트 listener + 즉시 connect로 한 쌍을 만든다.
    async fn socket_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (client_res, server_res) =
            tokio::join!(tokio::net::TcpStream::connect(addr), listener.accept(),);
        (client_res.unwrap(), server_res.unwrap().0)
    }

    #[tokio::test]
    async fn body_exceeds_max_body_size_is_rejected_by_content_length() {
        // 핵심 회귀 방지: 광고된 Content-Length가 max_body_size를 초과하면
        // 단 한 바이트도 alloc/read 하지 않고 즉시 거부되어야 한다.
        let (mut client, server) = socket_pair().await;

        let req = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1073741824\r\n\r\n";
        use tokio::io::AsyncWriteExt;
        client.write_all(req).await.unwrap();

        let mut options = Options::new();
        options.max_body_size = Some(1024 * 1024); // 1 MiB cap
        options.read_timeout_milliseconds = 200;
        options.read_max_retry = 1;

        let result = get_bytes_from_reader(server, &options).await;
        assert!(result.is_err(), "expected rejection of CL > cap");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("max_body_size"),
            "error msg should mention max_body_size cap, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn body_within_max_body_size_passes_through() {
        // 음성 케이스: 광고된 Content-Length가 cap 이하이면 정상 처리되어야 한다.
        let (mut client, server) = socket_pair().await;

        let body = b"hello";
        let req = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        use tokio::io::AsyncWriteExt;
        client.write_all(req.as_bytes()).await.unwrap();

        let mut options = Options::new();
        options.max_body_size = Some(1024); // 1 KiB cap, body는 5B
        options.read_timeout_milliseconds = 200;
        options.read_max_retry = 1;

        let (buf, _stream) = get_bytes_from_reader(server, &options)
            .await
            .expect("should accept body within cap");
        // 헤더 + body 가 모두 들어왔는지 확인 (body=hello 가 buf 끝부분에 있어야 함)
        assert!(
            buf.ends_with(b"hello"),
            "expected body 'hello' at end of buffer, got len={}",
            buf.len()
        );
    }

    // ===== 0.14.0 신규 streaming API 검증 =====

    #[tokio::test]
    async fn streaming_read_chunk_returns_exact_bytes() {
        // 본격: read_headers_only → split → Body::new_streaming → read_chunk 반복
        let (mut client, server) = socket_pair().await;

        let body = vec![b'X'; 50_000];
        let req_head = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );

        use tokio::io::AsyncWriteExt;
        let body_clone = body.clone();
        tokio::spawn(async move {
            client.write_all(req_head.as_bytes()).await.unwrap();
            for chunk in body_clone.chunks(8_000) {
                client.write_all(chunk).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });

        let mut options = Options::new();
        options.read_timeout_milliseconds = 1000;
        options.read_max_retry = 10;

        let hr = read_headers_only(server, &options).await.unwrap();
        let (read_half, _write_half) = hr.stream.into_split();
        let mut body_streaming =
            crate::Body::new_streaming(hr.leftover, read_half, hr.content_length, None, None);

        let mut collected = Vec::new();
        while let Some(chunk) = body_streaming.read_chunk().await.unwrap() {
            collected.extend_from_slice(&chunk);
        }

        assert_eq!(collected.len(), body.len(), "received length mismatch");
        assert_eq!(collected, body, "received bytes don't match sent body");
    }

    #[tokio::test]
    async fn auto_branches_to_arena_when_content_length_within_cap() {
        // 작은 요청: CL=20, cap=1MB → AutoParseResult::Arena
        let (mut client, server) = socket_pair().await;

        let body = b"hello-world-payload!";
        let req = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        use tokio::io::AsyncWriteExt;
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
        });

        let options = std::sync::Arc::new(crate::Options::new());
        let peer = "127.0.0.1:1234".parse().unwrap();
        let accept = crate::Accept::new(server, options, peer);

        let result = accept
            .parse_request_auto_with_cap(1 * 1024 * 1024) // 1 MiB cap
            .await
            .unwrap();

        match result {
            crate::AutoParseResult::Arena {
                request,
                response: _,
            } => {
                // arena body 가 정확한 body 를 가지고 있는지
                let body_bytes = request.body().get_body_bytes();
                assert_eq!(body_bytes, body);
            }
            crate::AutoParseResult::Streaming { .. } => {
                panic!("expected Arena variant for CL within cap");
            }
        }
    }

    #[tokio::test]
    async fn auto_default_cap_50mb_routes_small_request_to_arena() {
        // parse_request_auto() — 인자 없는 default (50 MiB cap). 작은 요청은 Arena로.
        let (mut client, server) = socket_pair().await;

        let body = b"small-payload";
        let req = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        use tokio::io::AsyncWriteExt;
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
        });

        let options = std::sync::Arc::new(crate::Options::new());
        let peer = "127.0.0.1:1234".parse().unwrap();
        let accept = crate::Accept::new(server, options, peer);

        let result = accept.parse_request_auto().await.unwrap();
        match result {
            crate::AutoParseResult::Arena {
                request,
                response: _,
            } => {
                assert_eq!(request.body().get_body_bytes(), body);
            }
            crate::AutoParseResult::Streaming { .. } => {
                panic!("expected Arena for 13B body under 50MiB default cap");
            }
        }
    }

    #[tokio::test]
    async fn auto_branches_to_streaming_when_content_length_exceeds_cap() {
        // 큰 요청: CL=200_000, cap=1KB → AutoParseResult::Streaming
        let (mut client, server) = socket_pair().await;

        let body = vec![b'Z'; 200_000];
        let req_head = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        use tokio::io::AsyncWriteExt;
        let body_clone = body.clone();
        tokio::spawn(async move {
            client.write_all(req_head.as_bytes()).await.unwrap();
            for chunk in body_clone.chunks(8_000) {
                client.write_all(chunk).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });

        let mut opt = crate::Options::new();
        opt.read_timeout_milliseconds = 1000;
        opt.read_max_retry = 10;
        let options = std::sync::Arc::new(opt);
        let peer = "127.0.0.1:1234".parse().unwrap();
        let accept = crate::Accept::new(server, options, peer);

        let result = accept
            .parse_request_auto_with_cap(1024) // 1 KiB cap → 200KB body 는 streaming 으로
            .await
            .unwrap();

        match result {
            crate::AutoParseResult::Streaming {
                mut request,
                response: _,
            } => {
                // streaming body 청크 단위로 읽어 검증
                let collected = request.body_mut().bytes(None).await.unwrap();
                assert_eq!(collected.len(), body.len());
                assert_eq!(collected, body);
            }
            crate::AutoParseResult::Arena { .. } => {
                panic!("expected Streaming variant for CL exceeding cap");
            }
        }
    }

    #[tokio::test]
    async fn streaming_into_multipart_parses_chunked_boundary() {
        // multer 가 청크 경계에 걸친 boundary 도 올바르게 파싱하는지 확인.
        // 클라이언트가 1-바이트 단위로 흘려보내는 극단 시나리오로 boundary
        // 인식이 read 청크 경계에 의존하지 않음을 증명.
        let (mut client, server) = socket_pair().await;

        let boundary = "BNDRY";
        let mut body_raw = Vec::new();
        body_raw.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body_raw
            .extend_from_slice(b"Content-Disposition: form-data; name=\"field1\"\r\n\r\nhello\r\n");
        body_raw.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body_raw.extend_from_slice(
            b"Content-Disposition: form-data; name=\"field2\"; filename=\"f.bin\"\r\n\r\n",
        );
        body_raw.extend_from_slice(&[1u8, 2, 3, 4, 5]);
        body_raw.extend_from_slice(b"\r\n");
        body_raw.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        let req_head = format!(
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Type: multipart/form-data; boundary={}\r\nContent-Length: {}\r\n\r\n",
            boundary,
            body_raw.len()
        );

        use tokio::io::AsyncWriteExt;
        let body_clone = body_raw.clone();
        tokio::spawn(async move {
            client.write_all(req_head.as_bytes()).await.unwrap();
            // 1바이트씩 흘려 보냄 — boundary 가 청크 경계에 무조건 걸치게.
            for b in body_clone.iter() {
                client.write_all(&[*b]).await.unwrap();
            }
        });

        let mut options = Options::new();
        options.read_timeout_milliseconds = 2000;
        options.read_max_retry = 20;

        let hr = read_headers_only(server, &options).await.unwrap();
        let (read_half, _write_half) = hr.stream.into_split();
        let body_streaming =
            crate::Body::new_streaming(hr.leftover, read_half, hr.content_length, None, None);

        let mut mp = body_streaming.into_multipart(boundary.to_string());
        let mut got_field1 = None;
        let mut got_field2_bytes = None;

        while let Some(mut field) = mp.next_field().await.unwrap() {
            let name = field.name().map(|s| s.to_string());
            let mut bytes = Vec::new();
            while let Some(chunk) = field.chunk().await.unwrap() {
                bytes.extend_from_slice(&chunk);
            }
            match name.as_deref() {
                Some("field1") => got_field1 = Some(String::from_utf8(bytes).unwrap()),
                Some("field2") => got_field2_bytes = Some(bytes),
                _ => {}
            }
        }

        assert_eq!(got_field1.as_deref(), Some("hello"));
        assert_eq!(got_field2_bytes.as_deref(), Some(&[1u8, 2, 3, 4, 5][..]));
    }

    #[tokio::test]
    async fn header_read_deadline_triggers_when_client_stalls() {
        // 핵심 회귀 방지: 클라이언트가 헤더를 부분만 보내고 멈추면
        // header_read_deadline_ms 내에 거부되어야 한다 (slowloris 방어).
        let (mut client, server) = socket_pair().await;

        // 일부러 \r\n\r\n 를 보내지 않음 — 서버는 영원히 헤더 끝을 못 찾는다.
        use tokio::io::AsyncWriteExt;
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: stalled")
            .await
            .unwrap();
        // client는 drop하지 않고 유지 (drop하면 EOF로 종료되어 deadline 이전에 break됨)

        let mut options = Options::new();
        options.header_read_deadline_ms = Some(150); // 150ms 안에 deadline
        options.read_timeout_milliseconds = 50;
        options.read_max_retry = 10; // retry는 deadline보다 오래 가도록 충분히 크게

        let start = std::time::Instant::now();
        let result = get_bytes_from_reader(server, &options).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "expected slowloris rejection");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Header read deadline") || err.contains("slowloris"),
            "error msg should mention deadline/slowloris, got: {}",
            err
        );
        // deadline 150ms 인데 elapsed 가 1초 이상이면 실패 (deadline이 사실상 안 먹은 것)
        assert!(
            elapsed.as_millis() < 1000,
            "deadline should fire fast (<1s), took {:?}",
            elapsed
        );

        // client는 여기서 drop — 명시적으로 살아있어야 했으므로 유지 흔적 남김
        drop(client);
    }

    #[tokio::test]
    async fn test_large_header_handling() {
        // 큰 헤더 생성 (10KB Cookie)
        let large_cookie = "x".repeat(10240);
        let large_header = format!(
            "GET / HTTP/1.1\r\nHost: example.com\r\nCookie: {}\r\nContent-Length: 100\r\n\r\n{}",
            large_cookie,
            "x".repeat(100)
        );

        // 실제 스트림을 시뮬레이션하기 위해 Cursor 사용
        use std::io::Cursor;
        Cursor::new(large_header.as_bytes());

        // 헤더 끝 위치 확인
        let header_end = find_header_end_optimized(large_header.as_bytes());
        assert!(header_end.is_some());

        let content_length = extract_content_length_simple(large_header.as_bytes());
        assert_eq!(content_length, Some(100));

        println!(
            "✅ 큰 헤더 테스트 통과: 헤더={}B, 바디=100B",
            header_end.unwrap() - 4
        );
    }
}
