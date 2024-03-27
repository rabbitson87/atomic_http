use std::error::Error;

use async_trait::async_trait;
use http::HeaderMap;
use http::Request;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::helpers::{
    common::get_static_str,
    traits::{
        bytes::SplitBytes,
        http_stream::{Form, Part},
        GetHeaderChild,
    },
};
use crate::Body;

#[async_trait]
pub trait RequestUtils {
    async fn get_json(&mut self) -> Result<serde_json::Value, Box<dyn Error>>;
    async fn get_text(&mut self) -> Result<String, Box<dyn Error>>;
    async fn get_multi_part(&mut self) -> Result<Option<Form>, Box<dyn Error>>;
}

#[async_trait]
impl RequestUtils for Request<Body> {
    async fn get_json(&mut self) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut body = String::new();

        if self.body().len > 0 {
            body = String::from_utf8_lossy(self.body().body.as_slice()).into();
        }

        let body: serde_json::Value = match body.as_str() {
            "" => serde_json::json!({}),
            _ => serde_json::from_str(body.as_str()).unwrap(),
        };
        Ok(body)
    }
    async fn get_text(&mut self) -> Result<String, Box<dyn Error>> {
        let mut body = String::new();

        if self.body().len > 0 {
            body = String::from_utf8_lossy(self.body().body.as_slice()).into();
        }

        Ok(body)
    }
    async fn get_multi_part(&mut self) -> Result<Option<Form>, Box<dyn Error>> {
        let content_type = self.headers().get("content-type");
        if content_type.is_some()
            && content_type
                .unwrap()
                .to_str()?
                .contains("multipart/form-data")
        {
            let boundary = content_type
                .unwrap()
                .to_str()?
                .split("boundary=")
                .last()
                .unwrap()
                .to_owned();
            let mut form = Form {
                text: ("".into(), "".into()),
                parts: Vec::new(),
            };

            for part_data in self
                .body()
                .body
                .as_slice()
                .split_bytes(format!("--{}", &boundary).as_bytes())
            {
                println!("part_data: {:?}", &part_data.len());
                let mut part_string = String::new();
                let mut part_bytes = BufReader::new(part_data.as_slice());
                while let Ok(n) = part_bytes.read_line(&mut part_string).await {
                    if n == 0 {
                        break;
                    }
                }
                let line_split = get_static_str(part_string).split("\r\n");

                let mut part = Part {
                    name: "".into(),
                    file_name: "".into(),
                    headers: HeaderMap::new(),
                    body: Vec::new(),
                };
                for line in line_split {
                    println!("{}", line);
                    if line.is_empty() {
                        continue;
                    }
                    if line.contains("Content-Disposition") {
                        let size_split = line.split(": ");
                        let value = size_split.last();

                        if value.is_some() && value.unwrap().contains("filename=") {
                            let headers = value.unwrap().get_header_child();
                            let name = headers.get("name").unwrap();
                            let file_name = headers.get("filename").unwrap();
                            part.name = name.into();
                            part.file_name = file_name.into();
                        } else if value.is_some() && value.unwrap().contains("name=") {
                            let headers = value.unwrap().get_header_child();
                            let name = headers.get("name").unwrap();

                            form.text = (name.into(), "".into());
                        }
                    } else if !line.contains(": ") {
                        form.text.1 = line.into();
                    } else {
                        let mut size_split = line.split(": ");
                        let key = size_split.next();
                        let value = size_split.next();

                        part.headers.insert(
                            get_static_str(key.unwrap().to_lowercase()),
                            http::header::HeaderValue::from_static(value.unwrap()),
                        );
                    }
                }

                if !part.file_name.is_empty() {
                    part_bytes.read_to_end(&mut part.body).await?;
                    form.parts.push(part);
                }
            }
            return Ok(Some(form));
        }
        Ok(None)
    }
}
