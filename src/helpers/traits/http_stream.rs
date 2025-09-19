use crate::dev_print;
use async_trait::async_trait;
#[cfg(feature = "arena")]
use bumpalo_herd::Herd;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;
#[cfg(feature = "arena")]
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt};
use tokio::net::TcpStream;

use crate::helpers::traits::bytes::SplitBytes;
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
        options: &Options,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError>;
}

#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(
        self,
        options: &Options,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(self, options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(request, stream, options)?)
    }
}

#[cfg(feature = "tokio_rustls")]
#[async_trait]
impl StreamHttp for TlsStream<TcpStream> {
    async fn parse_request(
        self,
        options: &Options,
    ) -> Result<(Request<Body>, Response<Writer>), SendableError> {
        let stream = self.into_inner().0;
        stream.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(stream, options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(request, stream, options)?)
    }
}

fn get_parse_result_from_request(
    mut request: Request<Body>,
    stream: TcpStream,
    options: &Options,
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
                options: options.clone(),
            })?,
    ))
}
async fn get_bytes_from_reader(
    mut stream: TcpStream,
    options: &Options,
) -> Result<(Vec<u8>, TcpStream), SendableError> {
    const MAX_HEADER_SIZE: usize = 64 * 1024; // 64KB 헤더 제한
    const INITIAL_READ_SIZE: usize = 4096; // 첫 읽기 크기
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    let buffer_size = match options.read_buffer_size {
        0 => INITIAL_READ_SIZE,
        _ => options.read_buffer_size,
    }.max(MAX_HEADER_SIZE);

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
                if let Some(relative_pos) = find_header_end_optimized(&header_buffer[search_start..]) {
                    header_end_pos = Some(search_start + relative_pos + 4); // +4 for "\r\n\r\n"
                    content_length = extract_content_length_simple(&header_buffer[..header_end_pos.unwrap()]);
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
fn find_header_end_optimized(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    
    // 최적화된 단순 검색 - 대부분의 경우 충분히 빠릅니다
    let mut i = 0;
    while i <= data.len() - 4 {
        // 4바이트를 한 번에 비교 (컴파일러가 최적화)
        if data[i] == b'\r' && 
           data[i + 1] == b'\n' && 
           data[i + 2] == b'\r' && 
           data[i + 3] == b'\n' {
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

// ✅ 간단한 Content-Length 추출
fn extract_content_length_simple(headers: &[u8]) -> Option<usize> {
    // 바이트 단위로 직접 검색 (String 변환 없음)
    let pattern = b"content-length:";
    
    // 대소문자 구분 없이 검색
    if let Some(start_pos) = find_pattern_case_insensitive(headers, pattern) {
        let value_start = start_pos + pattern.len();
        
        // 숫자 부분만 추출
        let mut _value_end = value_start;
        let mut number_start = value_start;
        
        // 공백 스킵
        while number_start < headers.len() && (headers[number_start] == b' ' || headers[number_start] == b'\t') {
            number_start += 1;
        }
        
        // 숫자 찾기
        _value_end = number_start;
        while _value_end < headers.len() {
            match headers[_value_end] {
                b'0'..=b'9' => _value_end += 1,
                b'\r' | b'\n' | b' ' | b'\t' => break,
                _ => break,
            }
        }
        
        if _value_end > number_start {
            if let Ok(value_str) = std::str::from_utf8(&headers[number_start.._value_end]) {
                return value_str.parse().ok();
            }
        }
    }
    
    None
}

// 대소문자 구분 없는 패턴 검색
fn find_pattern_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    
    for i in 0..=haystack.len() - needle.len() {
        let mut matches = true;
        for j in 0..needle.len() {
            let h = haystack[i + j].to_ascii_lowercase();
            let n = needle[j].to_ascii_lowercase();
            if h != n {
                matches = false;
                break;
            }
        }
        if matches {
            return Some(i);
        }
    }
    
    None
}

async fn get_request(bytes: Vec<u8>) -> Result<Request<Body>, SendableError> {
    dev_print!("bytes len: {:?}", &bytes.len());

    let (header, bytes) = bytes.as_slice().split_header_body();
    let headers_string: String = String::from_utf8_lossy(&header).into();

    dev_print!("headers_string: {:?}", &headers_string);
    dev_print!("headers_string len: {:?}", &headers_string.len());

    let len: usize = bytes.len();

    let mut method_option = None;
    let mut uri_option = None;
    let mut version_option = None;
    let mut headers: Vec<(String, String)> = Vec::new();

    if !headers_string.is_empty() {
        let line_split = headers_string.split("\r\n");

        line_split.enumerate().for_each(|(index, line)| {
            dev_print!("{}", line);
            if line == "" {
                return;
            }
            if index == 0 {
                let mut line_split_sub = line.split(" ");
                match line_split_sub.next() {
                    Some(method) => {
                        if let Ok(method) = method.parse::<http::Method>() {
                            method_option = Some(method);
                        }
                    }
                    None => {
                        dev_print!("method is None");
                    }
                }
                match line_split_sub.next() {
                    Some(uri) => {
                        if let Ok(uri) = uri.parse::<http::Uri>() {
                            uri_option = Some(uri);
                        }
                    }
                    None => {
                        dev_print!("uri is None");
                    }
                }
                match line_split_sub.next() {
                    Some(version) => {
                        let version = match version {
                            "HTTP/0.9" => http::Version::HTTP_09,
                            "HTTP/1.0" => http::Version::HTTP_10,
                            "HTTP/1.1" => http::Version::HTTP_11,
                            "HTTP/2.0" => http::Version::HTTP_2,
                            "HTTP/3.0" => http::Version::HTTP_3,
                            _ => http::Version::HTTP_11,
                        };
                        version_option = Some(version);
                    }
                    None => {
                        version_option = Some(http::Version::HTTP_11);
                        dev_print!("version is None");
                    }
                }
            } else {
                let mut size_split = line.trim().split(": ");
                let key = size_split.next();
                let value = size_split.next();

                match key.is_some() && value.is_some() {
                    true => {
                        headers.push((key.unwrap().to_lowercase().into(), value.unwrap().into()));
                    }
                    false => {
                        dev_print!("key or value is None");
                    }
                }
            }
        });
    }
    let version = match version_option {
        Some(version) => version,
        None => http::Version::HTTP_11,
    };

    let mut request = Request::builder();
    if headers.len() > 0 {
        for (key, value) in headers {
            request = request.header(key, value);
        }
    }

    let request = request
        .method(match method_option {
            Some(method) => method,
            None => http::Method::GET,
        })
        .uri(match uri_option {
            Some(uri) => uri,
            None => "/".parse()?,
        })
        .version(version)
        .body(Body {
            body: String::new(),
            bytes,
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
        options: &Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArena for TcpStream {
    async fn parse_request_arena(
        self,
        options: &Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<Writer>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, options, herd).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena(request, stream, options)?)
    }
}

#[cfg(feature = "arena")]
async fn get_bytes_arena_direct(
    mut stream: TcpStream,
    options: &Options,
    herd: Arc<Herd>,
) -> Result<(ArenaBody, TcpStream), SendableError> {
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    const MAX_HEADER_SIZE: usize = 64 * 1024;
    const INITIAL_ARENA_SIZE: usize = 8192;
    const HEADER_END_MARKER: &[u8] = b"\r\n\r\n";

    let member = herd.get();
    let buffer_size = match options.read_buffer_size {
        0 => INITIAL_ARENA_SIZE,
        _ => options.read_buffer_size,
    }.max(MAX_HEADER_SIZE);

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
                    content_length = extract_content_length_simple(&temp_header_buf[..header_end_pos.unwrap()]);
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

    // 2단계: 정확한 크기로 Arena 할당
    let arena_data = member.alloc_slice_fill_default::<u8>(total_size);

    // 3단계: 이미 읽은 헤더 데이터를 Arena로 복사 (한 번만)
    let already_read = temp_header_buf.len().min(total_size);
    arena_data[..already_read].copy_from_slice(&temp_header_buf[..already_read]);

    // 4단계: 남은 바디 데이터를 Arena에 직접 읽기
    let mut total_read = already_read;
    retry_count = 0;

    while total_read < total_size && retry_count < options.read_max_retry {
        let remaining = total_size - total_read;
        let chunk_size = remaining.min(options.read_buffer_size);

        match tokio::time::timeout(
            read_timeout,
            stream.read(&mut arena_data[total_read..total_read + chunk_size]),
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

    // ArenaBody 생성
    let body_start = header_end;
    let arena_body = ArenaBody::new(member, &arena_data[..total_read], header_end, body_start);

    dev_print!(
        "Arena HTTP 파싱 완료: 헤더={}B, 바디={}B, 총={}B (Arena 할당)",
        header_end,
        total_read.saturating_sub(header_end),
        total_read
    );

    Ok((arena_body, stream))
}

#[cfg(feature = "arena")]
fn parse_http_request_arena(mut body: ArenaBody) -> Result<Request<ArenaBody>, SendableError> {
    let headers_bytes = body.get_headers();
    let headers_str = std::str::from_utf8(headers_bytes)?;

    let mut method_option = None;
    let mut uri_option = None;
    let mut version_option = None;
    let mut headers: Vec<(String, String)> = Vec::new();

    if !headers_str.is_empty() {
        let lines: Vec<&str> = headers_str.split("\r\n").collect();

        for (index, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }

            if index == 0 {
                // 요청 라인 파싱: "GET /path HTTP/1.1"
                let parts: Vec<&str> = line.split(' ').collect();
                if let Some(method_str) = parts.get(0) {
                    if let Ok(method) = method_str.parse::<http::Method>() {
                        method_option = Some(method);
                    }
                }
                if let Some(uri_str) = parts.get(1) {
                    if let Ok(uri) = uri_str.parse::<http::Uri>() {
                        uri_option = Some(uri);
                    }
                }
                if let Some(version_str) = parts.get(2) {
                    let version = match *version_str {
                        "HTTP/0.9" => http::Version::HTTP_09,
                        "HTTP/1.0" => http::Version::HTTP_10,
                        "HTTP/1.1" => http::Version::HTTP_11,
                        "HTTP/2.0" => http::Version::HTTP_2,
                        "HTTP/3.0" => http::Version::HTTP_3,
                        _ => http::Version::HTTP_11,
                    };
                    version_option = Some(version);
                }
            } else if let Some(colon_pos) = line.find(": ") {
                // 헤더 파싱: "Content-Type: application/json"
                let key = &line[..colon_pos];
                let value = &line[colon_pos + 2..];
                headers.push((key.to_lowercase(), value.to_string()));
            }
        }
    }

    let mut request_builder = Request::builder()
        .method(method_option.unwrap_or(http::Method::GET))
        .uri(uri_option.unwrap_or_else(|| "/".parse().unwrap()))
        .version(version_option.unwrap_or(http::Version::HTTP_11));

    for (key, value) in headers {
        request_builder = request_builder.header(key, value);
    }

    body.ip = None; // 이후에 설정됨
    let request = request_builder.body(body)?;
    Ok(request)
}

#[cfg(feature = "arena")]
fn get_parse_result_arena(
    mut request: Request<ArenaBody>,
    stream: TcpStream,
    options: &Options,
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
                options: options.clone(),
            })?,
    ))
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait StreamHttpArenaWriter {
    async fn parse_request_arena_writer(
        self,
        options: &Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl StreamHttpArenaWriter for TcpStream {
    async fn parse_request_arena_writer(
        self,
        options: &Options,
        herd: Arc<Herd>,
    ) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
        self.set_nodelay(options.no_delay)?;

        let (arena_body, stream) = get_bytes_arena_direct(self, options, herd.clone()).await?;
        let request = parse_http_request_arena(arena_body)?;

        Ok(get_parse_result_arena_writer(
            request, stream, options, herd,
        )?)
    }
}

#[cfg(feature = "arena")]
fn get_parse_result_arena_writer(
    mut request: Request<ArenaBody>,
    stream: TcpStream,
    options: &Options,
    herd: Arc<Herd>,
) -> Result<(Request<ArenaBody>, Response<ArenaWriter>), SendableError> {
    let version = request.version();
    request.body_mut().ip = options.current_client_addr;

    Ok((
        request,
        Response::builder()
            .version(version)
            .header(CONTENT_TYPE, "application/json")
            .status(400)
            .body(ArenaWriter::new(stream, herd, options.clone()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_header_end_detection() {
        let test_cases = vec![
            // 작은 헤더
            (b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n".as_slice(), Some(35)),
            
            // 일반적인 헤더
            (b"POST /api HTTP/1.1\r\nHost: example.com\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n".as_slice(), Some(84)),
            
            // 헤더 끝이 없는 경우
            (b"GET / HTTP/1.1\r\nHost: example.com\r\n".as_slice(), None),
            
            // 빈 데이터
            (b"".as_slice(), None),
        ];
        
        for (data, expected) in test_cases {
            let result = find_header_end_optimized(data);
            assert_eq!(result, expected, "Failed for data: {:?}", std::str::from_utf8(data));
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
            large_cookie, "x".repeat(100)
        );
        
        // 실제 스트림을 시뮬레이션하기 위해 Cursor 사용
        use std::io::Cursor;
        Cursor::new(large_header.as_bytes());
        
        // 헤더 끝 위치 확인
        let header_end = find_header_end_optimized(large_header.as_bytes());
        assert!(header_end.is_some());

        let content_length = extract_content_length_simple(large_header.as_bytes());
        assert_eq!(content_length, Some(100));
        
        println!("✅ 큰 헤더 테스트 통과: 헤더={}B, 바디=100B", 
                header_end.unwrap() - 4);
    }
}