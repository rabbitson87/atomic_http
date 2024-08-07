use std::error::Error;

use async_trait::async_trait;
use http::Response;
use tokio::io::{self, AsyncWriteExt};

use crate::Writer;

impl Writer {
    pub async fn write_bytes(&mut self) -> Result<(), Box<dyn Error>> {
        let body = self.bytes.as_slice();
        loop {
            self.stream.writable().await?;
            match self.stream.try_write(body) {
                Ok(_n) => {
                    break;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    self.stream.flush().await?;
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait ResponseUtil {
    async fn responser(&mut self) -> Result<(), Box<dyn Error>>;
}

#[async_trait]
impl ResponseUtil for Response<Writer> {
    async fn responser(&mut self) -> Result<(), Box<dyn Error>> {
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
            let content_length = file.metadata().await.unwrap().len();
            send_string.push_str(format!("content-length: {}\r\n", content_length).as_str());

            send_string.push_str("\r\n");
            loop {
                self.body_mut().stream.writable().await?;
                match self.body_mut().stream.try_write(send_string.as_bytes()) {
                    Ok(_n) => {
                        break;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(e) => {
                        self.body_mut().stream.flush().await?;
                        return Err(e.into());
                    }
                }
            }

            let mut reader = io::BufReader::new(file);
            let mut buffer = match content_length < 1048576 * 5 {
                true => vec![0; content_length as usize],
                false => vec![0; 1048576 * 5],
            };
            while let Ok(len) = reader.read(&mut buffer).await {
                if len == 0 {
                    break;
                }
                loop {
                    self.body_mut().stream.writable().await?;
                    match self.body_mut().stream.try_write(&buffer[0..len]) {
                        Ok(_n) => {
                            break;
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            self.body_mut().stream.flush().await?;
                            return Err(e.into());
                        }
                    }
                }
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

            loop {
                self.body_mut().stream.writable().await?;
                match self.body_mut().stream.try_write(&send_string.as_bytes()) {
                    Ok(_n) => {
                        break;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(e) => {
                        self.body_mut().stream.flush().await?;
                        return Err(e.into());
                    }
                }
            }
        }
        self.body_mut().stream.flush().await?;
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
