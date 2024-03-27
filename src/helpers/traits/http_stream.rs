use async_trait::async_trait;
use http::{header::CONTENT_TYPE, HeaderMap, Request, Response};
use std::error::Error;
use tokio::io::{split, AsyncReadExt, WriteHalf};
use tokio::net::TcpStream;

use crate::helpers::common::get_static_str;
use crate::helpers::traits::bytes::SplitBytes;

pub struct Body {
    pub body: Vec<u8>,
    pub len: usize,
}

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

pub struct Writer {
    pub writer: WriteHalf<TcpStream>,
    pub body: String,
}

#[async_trait]
pub trait StreamHttp {
    async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>>;
}

#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (mut reader, writer) = split(self);

        let mut bytes: Vec<u8> = vec![];

        let buffer_size = 1024;
        let mut buf = vec![0; buffer_size];
        loop {
            match reader.read(&mut buf).await {
                Ok(n) => {
                    if n != 0 {
                        bytes.extend_from_slice(&buf[..n]);
                    }
                    if n < buffer_size {
                        break;
                    }
                }
                Err(err) => {
                    println!("an error occurred; error = {:?}", err);
                }
            }
        }

        let (header, body) = bytes.as_slice().split_header_body();
        let headers_string: String = String::from_utf8_lossy(&header).into();

        println!("headers_string: {:?}", &headers_string);
        println!("headers_string len: {:?}", &headers_string.len());

        let len = body.len();
        let mut request: Request<Body> = Request::builder().body(Body { body, len })?;

        if !headers_string.is_empty() {
            let line_split = get_static_str(headers_string).split("\r\n");

            for line in line_split {
                println!("{}", line);
                if line == "" {
                    continue;
                }
                if line.to_uppercase().contains("HTTP/") {
                    let mut line_split_sub = line.split(" ");
                    match line_split_sub.next() {
                        Some(method) => {
                            *request.method_mut() = method.parse::<http::Method>()?;
                        }
                        None => {
                            println!("method is None");
                        }
                    }
                    match line_split_sub.next() {
                        Some(uri) => {
                            *request.uri_mut() = uri.parse::<http::Uri>()?;
                        }
                        None => {
                            println!("uri is None");
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
                            *request.version_mut() = version;
                        }
                        None => {
                            *request.version_mut() = http::Version::HTTP_11;
                            println!("version is None");
                        }
                    }
                } else {
                    let mut size_split = line.trim().split(": ");
                    let key = size_split.next();
                    let value = size_split.next();

                    match key.is_some() && value.is_some() {
                        true => {
                            request.headers_mut().insert(
                                get_static_str(key.unwrap().to_lowercase()),
                                http::header::HeaderValue::from_static(value.unwrap()),
                            );
                        }
                        false => {
                            println!("key or value is None");
                        }
                    }
                }
            }
        }
        let version = request.version();

        Ok((
            request,
            Response::builder()
                .version(version)
                .header(CONTENT_TYPE, "application/json")
                .status(400)
                .body(Writer {
                    writer,
                    body: "".into(),
                })?,
        ))
    }
}
