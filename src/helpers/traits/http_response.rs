use std::error::Error;

use async_trait::async_trait;
use http::Response;
use tokio::io::AsyncWriteExt;

use crate::Writer;

impl Writer {
    pub async fn write_bytes(&mut self) -> Result<(), Box<dyn Error>> {
        let body = self.bytes.as_slice();
        self.writer.write_all(body).await?;
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
        let status_line = format!("{:?} {}\r\n", self.version(), self.status());
        send_string.push_str(&status_line);

        if cfg!(feature = "response_file") && self.body().use_file {
            use http::StatusCode;
            use tokio::{
                fs,
                io::{self, AsyncReadExt},
            };

            *self.status_mut() = StatusCode::from_u16(200)?;
            #[cfg(feature = "response_file")]
            {
                use http::header::CONTENT_TYPE;
                self.headers_mut().remove(CONTENT_TYPE);
                send_string.push_str(&format!(
                    "Content-Type: {}\r\n",
                    get_content_type(&self.body().body)
                ));
            }

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }

            let file = fs::File::open(&self.body().body).await?;

            send_string.push_str(
                format!(
                    "content-length: {}\r\n",
                    file.metadata().await.unwrap().len()
                )
                .as_str(),
            );

            send_string.push_str("\r\n");
            self.body_mut()
                .writer
                .write_all(send_string.as_bytes())
                .await?;

            let mut reader = io::BufReader::new(file);
            let mut buffer = vec![0; 1048576 * 5];
            while let Ok(len) = reader.read(&mut buffer).await {
                if len == 0 {
                    break;
                }
                self.body_mut().writer.write_all(&buffer[0..len]).await?;
            }
        } else if !self.body().bytes.is_empty() {
            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            send_string.push_str("\r\n");
            self.body_mut()
                .writer
                .write_all(&send_string.as_bytes())
                .await?;

            self.body_mut().write_bytes().await?;
        } else {
            let (body, content_string) = get_body(self.body().body.as_str()).await;
            send_string.push_str(&content_string);

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            println!("headers: {}", &send_string);
            send_string.push_str("\r\n");

            send_string.push_str(&body);

            self.body_mut()
                .writer
                .write_all(&send_string.as_bytes())
                .await?;
        }
        self.body_mut().writer.flush().await?;
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
    println!("content-length: {}\n", &content_length);
    (body.into(), content_length)
}
