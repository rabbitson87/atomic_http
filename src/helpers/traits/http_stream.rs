use async_trait::async_trait;
#[cfg(feature = "arena")]
use bumpalo_herd::Herd;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
#[cfg(feature = "arena")]
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::helpers::traits::bytes::SplitBytes;
#[cfg(feature = "arena")]
use crate::{ArenaBody, ArenaWriter};
use crate::{Body, Options, SendableError, Writer};

pub struct Form {
    pub text: (String, String),
    pub parts: Vec<Part>,
}

pub struct Part {
    pub name: String,
    pub file_name: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

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
    let mut bytes: Vec<u8> = vec![];
    let buffer_size = match options.read_buffer_size {
        0 => 4096,
        _ => options.read_buffer_size,
    };
    let mut buf = vec![0; buffer_size];
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;

    let mut headers_done = false;
    let mut _content_length = None;
    let mut expected_total_length = None;

    while retry_count < max_retry {
        match tokio::time::timeout(
            Duration::from_millis(options.read_timeout_miliseconds),
            stream.read(&mut buf),
        )
        .await
        {
            Ok(read_result) => match read_result {
                Ok(n) => {
                    if n == 0 {
                        // 연결이 끊겼지만 데이터가 부족한 경우
                        if let Some(expected) = expected_total_length {
                            if bytes.len() < expected {
                                dev_print!(
                                    "Connection closed but data incomplete: {}/{} bytes",
                                    bytes.len(),
                                    expected
                                );
                                retry_count += 1;
                                continue;
                            }
                        }
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);

                    if !headers_done {
                        if let Some(headers_end) = find_headers_end(&bytes) {
                            headers_done = true;
                            _content_length = parse_content_length(&bytes[..headers_end]);

                            if let Some(length) = _content_length {
                                expected_total_length = Some(headers_end + length);
                                dev_print!("Expected total length: {}", headers_end + length);
                                if let Some(expected) = expected_total_length {
                                    if bytes.len() >= expected {
                                        break;
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                    } else if let Some(expected) = expected_total_length {
                        if bytes.len() >= expected {
                            break;
                        }
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            },
            Err(_) => {
                retry_count += 1;
                continue;
            }
        }
    }

    if bytes.len() == 0 {
        stream.flush().await?;
        return Err("no data".into());
    }

    // 최종 데이터 검증
    if let Some(expected) = expected_total_length {
        if bytes.len() < expected {
            stream.flush().await?;
            return Err(format!(
                "Incomplete data after {} retries: got {}/{} bytes{}",
                max_retry,
                bytes.len(),
                expected,
                match options.read_imcomplete_size {
                    0 => "".into(),
                    _ => format!(
                        ", Data:{}",
                        match find_headers_end(&bytes) {
                            Some(headers_end) => String::from_utf8_lossy(
                                &bytes[headers_end..options.read_imcomplete_size]
                            ),
                            None => String::from_utf8_lossy(&bytes[..options.read_imcomplete_size]),
                        }
                    ),
                }
            )
            .into());
        }
    }

    Ok((bytes, stream))
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

fn find_headers_end(data: &[u8]) -> Option<usize> {
    data.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let headers_str = String::from_utf8_lossy(headers);
    headers_str
        .lines()
        .find(|line| line.to_lowercase().starts_with("content-length:"))
        .and_then(|line| {
            line.split(':')
                .nth(1)
                .and_then(|len| len.trim().parse().ok())
        })
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

    let member = herd.get();

    let buffer_size = match options.read_buffer_size {
        0 => 4096,
        _ => options.read_buffer_size,
    };

    let mut temp_data = Vec::new();
    let mut temp_buf = vec![0; buffer_size];
    let mut retry_count = 0;
    let max_retry = options.read_max_retry;

    let mut headers_done = false;
    let mut header_end_pos = None;
    let mut expected_total_length = None;

    while retry_count < max_retry {
        match tokio::time::timeout(
            Duration::from_millis(options.read_timeout_miliseconds),
            stream.read(&mut temp_buf),
        )
        .await
        {
            Ok(read_result) => match read_result {
                Ok(n) => {
                    if n == 0 {
                        if let Some(expected) = expected_total_length {
                            if temp_data.len() < expected {
                                dev_print!(
                                    "Connection closed but data incomplete: {}/{} bytes",
                                    temp_data.len(),
                                    expected
                                );
                                retry_count += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    temp_data.extend_from_slice(&temp_buf[..n]);

                    if !headers_done {
                        if let Some(pos) = find_headers_end(&temp_data) {
                            headers_done = true;
                            header_end_pos = Some(pos);

                            let content_length = parse_content_length(&temp_data[..pos]);
                            if let Some(length) = content_length {
                                expected_total_length = Some(pos + length);
                                if temp_data.len() >= pos + length {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    } else if let Some(expected) = expected_total_length {
                        if temp_data.len() >= expected {
                            break;
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            },
            Err(_) => {
                retry_count += 1;
                continue;
            }
        }
    }

    if temp_data.is_empty() {
        return Err("no data".into());
    }

    if let Some(expected) = expected_total_length {
        if temp_data.len() < expected {
            return Err(format!(
                "Incomplete data after {} retries: got {}/{} bytes{}",
                max_retry,
                temp_data.len(),
                expected,
                match options.read_imcomplete_size {
                    0 => "".to_string(),
                    _ => format!(
                        ", Data: {}",
                        match header_end_pos {
                            Some(headers_end) => String::from_utf8_lossy(
                                &temp_data[headers_end
                                    ..options.read_imcomplete_size.min(temp_data.len())]
                            ),
                            None => String::from_utf8_lossy(
                                &temp_data[..options.read_imcomplete_size.min(temp_data.len())]
                            ),
                        }
                    ),
                }
            )
            .into());
        }
    }

    let allocated_data = member.alloc_slice_copy(&temp_data);

    let header_end = header_end_pos.unwrap_or(allocated_data.len());
    let body_start = if header_end_pos.is_some() {
        header_end
    } else {
        allocated_data.len()
    };

    let arena_body = ArenaBody::new(member, allocated_data, header_end, body_start);

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
