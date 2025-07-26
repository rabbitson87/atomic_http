use async_trait::async_trait;
use http::Response;
use tokio::io::AsyncWriteExt;

#[cfg(feature = "arena")]
use crate::ArenaWriter;
use crate::{SendableError, Writer};
#[cfg(feature = "response_file")]
use std::path::Path;

impl Writer {
    pub async fn write_bytes(&mut self) -> Result<(), SendableError> {
        self.stream.send_bytes(self.bytes.as_slice()).await?;
        Ok(())
    }

    #[cfg(feature = "response_file")]
    pub fn response_file<P>(&mut self, path: P) -> Result<(), SendableError>
    where
        P: AsRef<Path>,
    {
        let root_path = &self.options.root_path;
        let path = root_path.join(path);
        self.body = path.to_str().unwrap().to_string();
        self.use_file = true;
        Ok(())
    }
}

#[async_trait]
pub trait ResponseUtil {
    async fn responser(&mut self) -> Result<(), SendableError>;
}

#[async_trait]
impl ResponseUtil for Response<Writer> {
    async fn responser(&mut self) -> Result<(), SendableError> {
        let mut send_string = String::new();
        if cfg!(feature = "response_file") && self.body().use_file {
            use http::StatusCode;
            *self.status_mut() = StatusCode::from_u16(200)?;
        }
        let status_line = format!("{:?} {}\r\n", self.version(), self.status());
        send_string.push_str(&status_line);

        if cfg!(feature = "response_file") && self.body().use_file {
            use tokio::{
                fs,
                io::{self, AsyncReadExt},
            };

            #[cfg(feature = "response_file")]
            {
                use http::header::CONTENT_TYPE;
                self.headers_mut().remove(CONTENT_TYPE);
                match self.body().body.split('.').last().unwrap() {
                    "zip" => {
                        send_string.push_str("Content-Type: application/zip\r\n");
                        send_string.push_str(&format!(
                            "content-disposition: attachment; filename={}\r\n",
                            self.body().body
                        ));
                    }
                    _ => {
                        send_string.push_str(&format!(
                            "Content-Type: {}\r\n",
                            get_content_type(&self.body().body)
                        ));
                    }
                }
            }

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }

            let file = fs::File::open(&self.body().body).await?;
            let content_length = file.metadata().await?.len();
            send_string.push_str(format!("content-length: {}\r\n", content_length).as_str());

            send_string.push_str("\r\n");
            self.body_mut()
                .stream
                .send_bytes(send_string.as_bytes())
                .await?;

            let mut reader = io::BufReader::new(file);
            let mut buffer = match content_length < 1048576 * 5 {
                true => vec![0; content_length as usize],
                false => vec![0; 1048576 * 5],
            };
            while let Ok(len) = reader.read(&mut buffer).await {
                if len == 0 {
                    break;
                }
                self.body_mut().stream.send_bytes(&buffer[0..len]).await?;
            }
        } else if !self.body().bytes.is_empty() {
            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            send_string.push_str("\r\n");
            let mut send_string = send_string.as_bytes().to_vec();
            send_string.extend(self.body().bytes.clone());
            self.body_mut().bytes = send_string;
            self.body_mut().write_bytes().await?;
        } else {
            let (body, content_string) = get_body(self.body().body.as_str()).await;
            send_string.push_str(&content_string);

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            dev_print!("headers: {}", &send_string);
            send_string.push_str("\r\n");

            send_string.push_str(&body);
            self.body_mut()
                .stream
                .send_bytes(send_string.as_bytes())
                .await?;
        }
        self.body_mut().stream.flush().await?;
        Ok(())
    }
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait ResponseUtilArena {
    async fn responser_arena(&mut self) -> Result<(), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl ResponseUtilArena for Response<ArenaWriter> {
    async fn responser_arena(&mut self) -> Result<(), SendableError> {
        let mut send_string = String::new();

        if cfg!(feature = "response_file") && self.body().use_file {
            use http::StatusCode;
            *self.status_mut() = StatusCode::from_u16(200)?;
        }

        let status_line = format!("{:?} {}\r\n", self.version(), self.status());
        send_string.push_str(&status_line);

        if cfg!(feature = "response_file") && self.body().use_file {
            #[cfg(feature = "response_file")]
            {
                use http::header::CONTENT_TYPE;
                use tokio::{
                    fs,
                    io::{self, AsyncReadExt},
                };
                self.headers_mut().remove(CONTENT_TYPE);

                if !self.body().response_data_len > 0 {
                    let file_path = unsafe {
                        let data = std::slice::from_raw_parts(
                            self.body().response_data_ptr,
                            self.body().response_data_len,
                        );
                        std::str::from_utf8(data)?
                    };

                    match file_path.split('.').last().unwrap() {
                        "zip" => {
                            send_string.push_str("Content-Type: application/zip\r\n");
                            send_string.push_str(&format!(
                                "content-disposition: attachment; filename={}\r\n",
                                file_path
                            ));
                        }
                        _ => {
                            send_string.push_str(&format!(
                                "Content-Type: {}\r\n",
                                get_content_type(file_path)
                            ));
                        }
                    }

                    for (key, value) in self.headers().iter() {
                        send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                    }

                    let file = fs::File::open(file_path).await?;
                    let content_length = file.metadata().await?.len();
                    send_string
                        .push_str(format!("content-length: {}\r\n", content_length).as_str());

                    send_string.push_str("\r\n");
                    self.body_mut()
                        .stream
                        .send_bytes(send_string.as_bytes())
                        .await?;

                    let mut reader = io::BufReader::new(file);
                    let mut buffer = match content_length < 1048576 * 5 {
                        true => vec![0; content_length as usize],
                        false => vec![0; 1048576 * 5],
                    };
                    while let Ok(len) = reader.read(&mut buffer).await {
                        if len == 0 {
                            break;
                        }
                        self.body_mut().stream.send_bytes(&buffer[0..len]).await?;
                    }
                }
            }
        } else {
            if !self.body().response_data_len > 0 {
                // Arena 메모리로 할당된 응답 데이터 사용
                for (key, value) in self.headers().iter() {
                    send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                }

                let content_length =
                    format!("content-length: {}\r\n", self.body().response_data_len);
                send_string.push_str(&content_length);
                send_string.push_str("\r\n");

                // 헤더 전송
                self.body_mut()
                    .stream
                    .send_bytes(send_string.as_bytes())
                    .await?;

                // Arena 데이터 직접 전송 (제로카피)
                let response_data = unsafe {
                    std::slice::from_raw_parts(
                        self.body().response_data_ptr,
                        self.body().response_data_len,
                    )
                };
                self.body_mut().stream.send_bytes(response_data).await?;
            } else {
                // 빈 응답
                for (key, value) in self.headers().iter() {
                    send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                }
                send_string.push_str("content-length: 0\r\n");
                send_string.push_str("\r\n");

                self.body_mut()
                    .stream
                    .send_bytes(send_string.as_bytes())
                    .await?;
            }
        }

        self.body_mut().stream.flush().await?;
        Ok(())
    }
}

pub trait SendBytes {
    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), SendableError>;
}

impl SendBytes for tokio::net::TcpStream {
    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), SendableError> {
        self.write_all(bytes).await?;
        Ok(())
    }
}

#[cfg(feature = "response_file")]
fn get_content_type(file_name: &str) -> String {
    let guess = mime_guess::from_path(file_name);

    if let Some(mime) = guess.first() {
        mime.to_string()
    } else {
        "text/plain".to_string()
    }
}

async fn get_body(body: &str) -> (String, String) {
    let length = body.len();

    let content_length = format!("content-length: {}\r\n", length);
    dev_print!("content-length: {}\n", &content_length);
    (body.into(), content_length)
}
