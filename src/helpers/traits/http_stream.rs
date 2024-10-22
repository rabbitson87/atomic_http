use async_trait::async_trait;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
use std::error::Error;
use std::time::Duration;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, Interest};
use tokio::net::TcpStream;

use crate::helpers::traits::bytes::SplitBytes;
use crate::{Body, Options, Writer};

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
    ) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>>;
}

#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(
        self,
        options: &Options,
    ) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        self.set_nodelay(options.no_delay)?;

        let (bytes, stream) = get_bytes_from_reader(self, options).await?;

        let request = get_request(bytes).await?;

        Ok(get_parse_result_from_request(request, stream, options)?)
    }
}

fn get_parse_result_from_request(
    request: Request<Body>,
    stream: TcpStream,
    options: &Options,
) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
    let version = request.version();

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
) -> Result<(Vec<u8>, TcpStream), Box<dyn Error>> {
    let mut bytes: Vec<u8> = vec![];
    let buffer_size = 4096;
    let mut buf = vec![0; buffer_size];

    let mut headers_done = false;
    let mut content_length = None;
    loop {
        let mut _count = 0;
        match options.use_normal_read {
            true => match stream.read(&mut buf).await {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);

                    if !headers_done {
                        if let Some(headers_end) = find_headers_end(&bytes) {
                            headers_done = true;
                            content_length = parse_content_length(&bytes[..headers_end]);

                            if let Some(length) = content_length {
                                if bytes.len() >= headers_end + length {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    } else if let Some(length) = content_length {
                        if bytes.len() >= length {
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
            false => {
                loop {
                    let ready = stream
                        .ready(Interest::READABLE | Interest::ERROR | Interest::WRITABLE)
                        .await?;
                    if ready.is_error() || ready.is_read_closed() || ready.is_empty() {
                        stream.flush().await?;
                        return Err("error".into());
                    }
                    if ready.is_readable() {
                        break;
                    }
                    if ready.is_writable() {
                        tokio::time::sleep(Duration::from_nanos(1)).await;
                        if _count > options.try_read_limit {
                            stream.flush().await?;
                            return Err("timeout".into());
                        }
                        _count += 1;
                        continue;
                    }
                }
                dev_print!("writable count: {}", _count);
                match stream.try_read(&mut buf) {
                    Ok(n) => {
                        if n == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&buf[..n]);

                        if !headers_done {
                            if let Some(headers_end) = find_headers_end(&bytes) {
                                headers_done = true;
                                content_length = parse_content_length(&bytes[..headers_end]);

                                if let Some(length) = content_length {
                                    if bytes.len() >= headers_end + length {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                        } else if let Some(length) = content_length {
                            if bytes.len() >= length {
                                break;
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(e) => {
                        stream.flush().await?;
                        return Err(e.into());
                    }
                }
            }
        }
    }
    if bytes.len() == 0 {
        stream.flush().await?;
        return Err("no data".into());
    }

    Ok((bytes, stream))
}

async fn get_request(bytes: Vec<u8>) -> Result<Request<Body>, Box<dyn Error>> {
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
            None => "/".parse().unwrap(),
        })
        .version(version)
        .body(Body {
            body: String::new(),
            bytes,
            len,
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
