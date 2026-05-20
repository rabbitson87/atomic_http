use crate::dev_print;
use async_trait::async_trait;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt};
use tokio::net::TcpStream;
#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;

#[cfg(feature = "arena")]
use crate::{ArenaBody, ArenaWriter};
use crate::{Body, Options, SendableError, Writer};

pub struct Form {
    pub text: (String, String),
    pub parts: Vec<Part>,
}

impl Form {
    pub fn new() -> Self {
        Self {
            text: (String::new(), String::new()),
            parts: Vec::new(),
        }
    }

    pub fn add_text_field(&mut self, name: &mut String, value: &mut String) {
        self.text = (std::mem::take(name), std::mem::take(value));
    }

    pub fn add_part(&mut self, part: Part) {
        self.parts.push(part);
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

// Send + Sync 구현 (Arena 메모리는 thread-safe)
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
    async fn parse_request(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError>;
}

#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(self, &options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(request, stream, options)?)
    }
}

#[cfg(feature = "tokio_rustls")]
#[async_trait]
impl StreamHttp for TlsStream<TcpStream> {
    async fn parse_request(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        let stream = self.into_inner().0;
        stream.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(stream, &options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(request, stream, options)?)
    }
}

pub(crate) fn get_parse_result_from_request(
    mut request: Request<Body>,
    stream: TcpStream,
    options: Arc<Options>,
) -> Result<(Request<Body>, Response<Writer>), SendableError> {
    let version = request.version();
    request.body_mut().ip = options.current_client_addr;

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(Writer {
                stream,
                body: String::new(),
                bytes: vec![],
                use_file: false,
                options,
            })?,
    ))
}
pub(crate) async fn get_bytes_from_reader(
    mut stream: TcpStream,
    options: &Options,
) -> Result<(Vec<u8>, TcpStream), SendableError> {
    const MAX_HEADER_SIZE: usize = 64 * 1024; // 64KB 헤더 제한
    const INITIAL_READ_SIZE: usize = 4096; // 첫 읽기 크기
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    let buffer_size = match options.read_buffer_size {
        0 => INITIAL_READ_SIZE,
        _ => options.read_buffer_size,
    }
    .max(MAX_HEADER_SIZE);

    // 1단계: 작은 버퍼로 헤더 먼저 읽기
    let mut header_buffer = Vec::with_capacity(buffer_size);
    let mut content_length = None;
    let mut header_end_pos = None;
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;
    let read_timeout = Duration::from_millis(options.read_timeout_miliseconds);

    while header_end_pos.is_none() && header_buffer.len() < buffer_size && retry_count < max_retry {
        let current_len = header_buffer.len();
        header_buffer.resize(current_len + buffer_size, 0);

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
                        header_end_pos = Some(search_start + relative_pos + 4); // +4 for "\r\n\r\n"
                        content_length = extract_content_length_simple(
                            &header_buffer[..header_end_pos.unwrap()],
                        );
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

    // 2단계: 전체 크기 계산 및 최종 버퍼 할당
    let total_expected_size = header_end + content_length.unwrap_or(0);

    // 이미 읽은 데이터보다 크면 더 큰 버퍼 필요
    let mut final_buffer = if total_expected_size > header_buffer.len() {
        let mut buf = Vec::with_capacity(total_expected_size);
        buf.extend_from_slice(&header_buffer);
        buf
    } else {
        header_buffer
    };

    // 3단계: 남은 바디 데이터 읽기
    let mut total_read = final_buffer.len();
    while total_read < total_expected_size && retry_count < options.read_max_retry {
        let remaining = total_expected_size - total_read;
        let chunk_size = remaining.min(buffer_size);

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

#[cfg(feature = "arena")]
fn determine_next_read_size(current_size: usize) -> usize {
    match current_size {
        0..=512 => 1024,      // 작은 헤더: 1KB 단위
        513..=4096 => 2048,   // 보통 헤더: 2KB 단위
        4097..=16384 => 4096, // 큰 헤더: 4KB 단위
        _ => 8192,            // 매우 큰 헤더: 8KB 단위
    }
}

// ✅ 간단하고 빠른 헤더 끝 찾기 (SIMD 없이도 충분히 빠름)
pub(crate) fn find_header_end_optimized(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }

    // 최적화된 단순 검색 - 대부분의 경우 충분히 빠릅니다
    let mut i = 0;
    while i <= data.len() - 4 {
        // 4바이트를 한 번에 비교 (컴파일러가 최적화)
        if data[i] == b'\r' && data[i + 1] == b'\n' && data[i + 2] == b'\r' && data[i + 3] == b'\n'
        {
            return Some(i);
        }

        // 첫 번째 바이트가 \r이 아니면 빠르게 스킵
        if data[i] != b'\r' {
            // \r을 찾을 때까지 빠르게 이동
            while i < data.len() && data[i] != b'\r' {
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    None
}

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
    let len = body_bytes.len();

    let request = builder.body(Body {
        body: String::new(),
        bytes: body_bytes,
        len,
        ip: None,
    })?;

    Ok(request)
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait StreamHttpArena {
    async fn parse_request_arena(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArena for TcpStream {
    async fn parse_request_arena(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, &options).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena(request, stream, options)?)
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
    const INITIAL_ARENA_SIZE: usize = 8192;
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    let buffer_size = match options.read_buffer_size {
        0 => INITIAL_ARENA_SIZE,
        _ => options.read_buffer_size,
    }
    .max(MAX_HEADER_SIZE);

    // 1단계: 헤더만 먼저 읽어서 Content-Length 파악
    let mut temp_header_buf = Vec::with_capacity(buffer_size);
    let mut header_end_pos = None;
    let mut content_length = None;
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;
    let read_timeout = Duration::from_millis(options.read_timeout_miliseconds);

    // 헤더 읽기 및 파싱
    while header_end_pos.is_none() && temp_header_buf.len() < buffer_size && retry_count < max_retry
    {
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
                        header_end_pos = Some(search_start + pos + 4);
                        content_length = extract_content_length_simple(
                            &temp_header_buf[..header_end_pos.unwrap()],
                        );
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
    let total_size = header_end + content_length.unwrap_or(0);

    // 2단계: 남은 바디 데이터 읽기 (Vec으로)
    let mut final_buffer = if total_size > temp_header_buf.len() {
        let mut buf = Vec::with_capacity(total_size);
        buf.extend_from_slice(&temp_header_buf);
        buf
    } else {
        temp_header_buf
    };

    // 3단계: 남은 바디 데이터 읽기
    let mut total_read = final_buffer.len();
    retry_count = 0;

    while total_read < total_size && retry_count < options.read_max_retry {
        let remaining = total_size - total_read;
        let chunk_size = remaining.min(options.read_buffer_size);

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
) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
    let version = request.version();
    request.body_mut().ip = options.current_client_addr;

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(Writer {
                stream,
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
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArenaWriter for TcpStream {
    async fn parse_request_arena_writer(
        self,
        options: Arc<Options>,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, &options).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena_writer(request, stream, options)?)
    }
}

#[cfg(feature = "arena")]
pub(crate) fn get_parse_result_arena_writer(
    mut request: Request<ArenaBody>,
    stream: TcpStream,
    options: Arc<Options>,
) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
    let version = request.version();
    request.body_mut().ip = options.current_client_addr;

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(ArenaWriter::new(stream, options))?,
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
