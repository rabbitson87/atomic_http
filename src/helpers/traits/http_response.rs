use std::error::Error;

use async_trait::async_trait;
use http::Response;
use tokio::{
    fs,
    io::{self, AsyncReadExt, AsyncWriteExt},
};

use super::http_stream::Writer;

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

        let content_type = self.headers().get("content-type");

        if content_type.is_some() && content_type.unwrap().to_str()?.contains("application/zip") {
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
        } else {
            let (body, content_string) = get_body(self.body().body.as_str()).await;
            send_string.push_str(&content_string);

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            println!("headers: {}", &send_string);
            send_string.push_str("\r\n");

            // println!("current version: v{}", VERSION);
            send_string.push_str(&body);
            // let path = get_temp_path()?;

            self.body_mut()
                .writer
                .write_all(&send_string.as_bytes())
                .await?;
        }
        self.body_mut().writer.flush().await?;
        Ok(())
    }
}

async fn get_body(body: &str) -> (String, String) {
    let length = body.len();

    let content_length = format!("content-length: {}\r\n", length);
    println!("content-length: {}\n", &content_length);
    (body.into(), content_length)
}
