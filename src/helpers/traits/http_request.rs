use std::error::Error;
use std::str::FromStr;

use async_trait::async_trait;
use http::HeaderMap;
use http::HeaderName;
use http::Request;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::helpers::traits::{
    bytes::SplitBytes,
    http_stream::{Form, Part},
    GetHeaderChild,
};
use crate::Body;

use super::StringUtil;

#[async_trait]
pub trait RequestUtils {
    async fn get_json<'a, T>(&'a mut self) -> Result<T, Box<dyn Error>>
    where
        T: Deserialize<'a>;
    async fn get_text(&mut self) -> Result<String, Box<dyn Error>>;
    async fn get_multi_part(&mut self) -> Result<Option<Form>, Box<dyn Error>>;
}

#[async_trait]
impl RequestUtils for Request<Body> {
    async fn get_json<'a, T>(&'a mut self) -> Result<T, Box<dyn Error>>
    where
        T: Deserialize<'a>,
    {
        if self.body().len > 0 {
            self.body_mut().body = String::from_utf8_lossy(self.body().bytes.as_slice()).into();
        }

        let body: T = match self.body().body.as_str() {
            "" => return Err("Empty body".into()),
            body => serde_json::from_str(body).unwrap(),
        };
        Ok(body)
    }
    async fn get_text(&mut self) -> Result<String, Box<dyn Error>> {
        if self.body().len > 0 {
            self.body_mut().body = String::from_utf8_lossy(self.body().bytes.as_slice()).into();
        }

        Ok(self.body().body.copy_string())
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
                .bytes
                .as_slice()
                .split_bytes(format!("--{}", &boundary).as_bytes())
            {
                dev_print!("part_data: {:?}", &part_data.len());
                let mut part_string = String::new();
                let mut part_bytes = BufReader::new(part_data.as_slice());
                while let Ok(n) = part_bytes.read_line(&mut part_string).await {
                    if n == 0 {
                        break;
                    }
                }
                let line_split = part_string.split("\r\n");

                let mut part = Part {
                    name: "".into(),
                    file_name: "".into(),
                    headers: HeaderMap::new(),
                    body: Vec::new(),
                };

                let mut headers: Vec<(String, String)> = Vec::new();
                line_split.for_each(|line| {
                    dev_print!("{}", line);
                    if line.is_empty() {
                        return;
                    }
                    if line.to_lowercase().contains("content-disposition") {
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

                        headers.push((key.unwrap().to_lowercase(), value.unwrap().into()));
                    }
                });

                part.headers = HeaderMap::from_iter(headers.into_iter().map(|(key, value)| {
                    (
                        HeaderName::from_str(key.as_str()).unwrap(),
                        value.parse().unwrap(),
                    )
                }));

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
