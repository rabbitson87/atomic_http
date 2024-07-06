use async_trait::async_trait;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, Request, Response};
use std::error::Error;
use tokio::io::{split, AsyncReadExt, WriteHalf};
use tokio::net::TcpStream;

use crate::helpers::traits::bytes::SplitBytes;
use crate::{Body, Writer};

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
    async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>>;
}

#[cfg(not(feature = "tokio_rustls"))]
#[async_trait]
impl StreamHttp for TcpStream {
    async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (mut reader, writer) = split(self);

        let bytes: Vec<u8> = get_bytes_from_reader(&mut reader).await;

        let request = get_request(bytes)?;

        Ok(get_parse_result_from_request(request, writer)?)
    }
}

#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;

#[cfg(feature = "tokio_rustls")]
#[async_trait]
impl StreamHttp for TlsStream<TcpStream> {
    async fn parse_request(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (mut reader, writer) = split(self);

        let bytes: Vec<u8> = get_bytes_from_reader(&mut reader).await;

        let request = get_request(bytes)?;

        Ok(get_parse_result_from_request(request, writer)?)
    }
}

#[cfg(not(feature = "tokio_rustls"))]
type WriterType = WriteHalf<TcpStream>;

#[cfg(feature = "tokio_rustls")]
type WriterType = WriteHalf<TlsStream<TcpStream>>;

fn get_parse_result_from_request(
    request: Request<Body>,
    writer: WriterType,
) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
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
                bytes: vec![],
                use_file: false,
            })?,
    ))
}

async fn get_bytes_from_reader<T>(reader: &mut T) -> Vec<u8>
where
    T: AsyncReadExt + Unpin,
{
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

    bytes
}

fn get_request(bytes: Vec<u8>) -> Result<Request<Body>, Box<dyn Error>> {
    let (header, body) = bytes.as_slice().split_header_body();
    let headers_string: String = String::from_utf8_lossy(&header).into();

    println!("headers_string: {:?}", &headers_string);
    println!("headers_string len: {:?}", &headers_string.len());

    let len: usize = body.len();

    let mut method_option = None;
    let mut uri_option = None;
    let mut version_option = None;
    let mut headers: Vec<(String, String)> = Vec::new();

    if !headers_string.is_empty() {
        let line_split = headers_string.split("\r\n");

        line_split.enumerate().for_each(|(index, line)| {
            println!("{}", line);
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
                        println!("method is None");
                    }
                }
                match line_split_sub.next() {
                    Some(uri) => {
                        if let Ok(uri) = uri.parse::<http::Uri>() {
                            uri_option = Some(uri);
                        }
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
                        version_option = Some(version);
                    }
                    None => {
                        version_option = Some(http::Version::HTTP_11);
                        println!("version is None");
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
                        println!("key or value is None");
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
        .body(Body { body, len })?;

    Ok(request)
}
