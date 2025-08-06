use std::borrow::Cow;

use async_trait::async_trait;
use http::HeaderName;
use http::Request;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::dev_print;
#[cfg(feature = "arena")]
use crate::helpers::traits::http_stream::{
    ArenaForm, ArenaHeaderRef, ArenaPartRef, ArenaTextFieldRef,
};
use crate::helpers::traits::{
    bytes::SplitBytes,
    http_stream::{Form, Part},
    GetHeaderChild,
};
#[cfg(feature = "arena")]
use crate::ArenaBody;
use crate::Body;
use crate::SendableError;

use super::StringUtil;

#[async_trait]
pub trait RequestUtils {
    fn get_json<'a, T>(&'a mut self) -> Result<T, SendableError>
    where
        T: Deserialize<'a>;
    fn get_text(&mut self) -> Result<String, SendableError>;
    async fn get_multi_part(&mut self) -> Result<Option<Form>, SendableError>;
}

#[async_trait]
impl RequestUtils for Request<Body> {
    fn get_json<'a, T>(&'a mut self) -> Result<T, SendableError>
    where
        T: Deserialize<'a>,
    {
        if self.body().len > 0 {
            self.body_mut().body = String::from_utf8_lossy(self.body().bytes.as_slice()).into();
        }

        let body: T = match self.body().body.as_str() {
            "" => return Err("Empty body".into()),
            body => serde_json::from_str(body)?,
        };
        Ok(body)
    }

    fn get_text(&mut self) -> Result<String, SendableError> {
        if self.body().len > 0 {
            self.body_mut().body = String::from_utf8_lossy(self.body().bytes.as_slice()).into();
        }

        Ok(self.body().body.copy_string())
    }

    async fn get_multi_part(&mut self) -> Result<Option<Form>, SendableError> {
        if let Some(content_type) = self.headers().get("content-type") {
            let content_type: Cow<str> = content_type.to_str()?.into();
            if !content_type.contains("multipart/form-data") {
                return Ok(None);
            }
            let boundary: Cow<str> = content_type.split("boundary=").last().unwrap().into();
            let mut form = Form::new();

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
                let mut line_split = part_string.split("\r\n").collect::<Vec<_>>();

                let mut part = Part::new();

                for line in line_split.iter_mut() {
                    dev_print!("{}", line);
                    if line.is_empty() {
                        continue;
                    }
                    if line.to_lowercase().contains("content-disposition") {
                        let size_split = line.split(": ");
                        if let Some(value) = size_split.last() {
                            if value.contains("filename=") {
                                let mut headers = value.get_header_child();
                                if let Some(name) = headers.get_mut("name") {
                                    part.set_name(name);
                                }

                                if let Some(file_name) = headers.get_mut("filename") {
                                    part.set_file_name(file_name);
                                }
                            } else if value.contains("name=") {
                                let mut headers = value.get_header_child();
                                if let Some(name) = headers.get_mut("name") {
                                    form.text = (std::mem::take(name).into(), "".into());
                                }
                            }
                        }
                    } else if !line.contains(": ") {
                        form.text.1 = std::mem::take(line).into();
                    } else {
                        let mut size_split = line.split(": ").collect::<Vec<_>>();
                        if size_split.len() != 2 {
                            continue;
                        }
                        part.headers.insert(
                            HeaderName::from_lowercase(
                                std::mem::take(&mut size_split[0]).to_lowercase().as_bytes(),
                            )?,
                            std::mem::take(&mut size_split[1]).parse().unwrap(),
                        );
                    }
                }

                if !part.file_name.is_empty() {
                    part_bytes.read_to_end(&mut part.body).await?;
                    form.parts.push(part);
                }
            }
            return Ok(Some(form));
        } else {
            Ok(None)
        }
    }
}

#[cfg(feature = "arena")]
pub trait RequestUtilsArena {
    fn get_json_arena<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::Deserialize<'de>;

    fn get_text_arena(&self) -> Result<&str, SendableError>;
    fn get_multi_part_arena(&self) -> Result<Option<ArenaForm>, SendableError>;
}

#[cfg(feature = "arena")]
impl RequestUtilsArena for Request<ArenaBody> {
    fn get_json_arena<T>(&self) -> Result<T, SendableError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let body_bytes = self.body().get_body_bytes();
        if body_bytes.is_empty() {
            return Err("Empty body".into());
        }

        let json: T = serde_json::from_slice(&body_bytes)?;
        Ok(json)
    }

    fn get_text_arena(&self) -> Result<&str, SendableError> {
        Ok(self.body().get_body_str()?)
    }

    fn get_multi_part_arena(&self) -> Result<Option<ArenaForm>, SendableError> {
        let content_type = match self.headers().get("content-type") {
            Some(ct) => ct.to_str()?,
            None => return Ok(None),
        };

        if !content_type.contains("multipart/form-data") {
            return Ok(None);
        }

        let boundary = content_type
            .split("boundary=")
            .nth(1)
            .ok_or("boundary not found")?;

        // ArenaBody의 실제 메모리 참조
        let arena_data = self.body().get_body_bytes();
        let mut form = ArenaForm::new(arena_data);

        // boundary로 파트 분할
        use crate::helpers::traits::bytes::SplitBytesArena;
        let boundary_bytes = format!("--{}", boundary).into_bytes();
        let parts = arena_data.split_bytes_arena(&boundary_bytes);

        for part_data in parts {
            if part_data.is_empty() {
                continue;
            }

            let (header_bytes, body_bytes) = part_data.split_header_body_arena();
            let header_str = std::str::from_utf8(header_bytes)?;

            let mut part = ArenaPartRef::new(body_bytes);
            let mut is_file = false;

            // 헤더 파싱 - raw pointer로 직접 처리
            for line in header_str.split("\r\n") {
                if line.trim().is_empty() {
                    continue;
                }

                if let Some(colon_pos) = line.find(": ") {
                    let header_name = &line[..colon_pos];
                    let header_value = &line[colon_pos + 2..];

                    if header_name.eq_ignore_ascii_case("content-disposition") {
                        // Content-Disposition 파싱
                        if let Some(name_value) =
                            extract_content_disposition_value(header_value, "name")
                        {
                            // arena_data 내에서 name_value의 위치 찾기
                            if let Some(name_slice) =
                                find_slice_in_arena(arena_data, name_value.as_bytes())
                            {
                                part.set_name(name_slice);
                            }
                        }

                        if let Some(filename_value) =
                            extract_content_disposition_value(header_value, "filename")
                        {
                            // arena_data 내에서 filename_value의 위치 찾기
                            if let Some(filename_slice) =
                                find_slice_in_arena(arena_data, filename_value.as_bytes())
                            {
                                part.set_file_name(filename_slice);
                                is_file = true;
                            }
                        }
                    } else if header_name.eq_ignore_ascii_case("content-type") {
                        if let Some(content_type_slice) =
                            find_slice_in_arena(arena_data, header_value.as_bytes())
                        {
                            part.set_content_type(content_type_slice);
                        }
                    }

                    // 헤더 추가
                    if let (Some(key_slice), Some(value_slice)) = (
                        find_slice_in_arena(arena_data, header_name.as_bytes()),
                        find_slice_in_arena(arena_data, header_value.as_bytes()),
                    ) {
                        part.headers
                            .push(ArenaHeaderRef::new(key_slice, value_slice));
                    }
                }
            }

            if part.get_name().is_none() {
                continue;
            }

            if is_file {
                form.parts.push(part);
            } else {
                // 텍스트 필드 처리
                if let (Some(name_slice), Some(body_slice)) = (
                    find_slice_in_arena(arena_data, part.get_name().unwrap_or("").as_bytes()),
                    if body_bytes.is_empty() {
                        None
                    } else {
                        Some(body_bytes)
                    },
                ) {
                    form.text_fields
                        .push(ArenaTextFieldRef::new(name_slice, body_slice));
                }
            }
        }

        Ok(Some(form))
    }
}

// Content-Disposition에서 특정 값 추출
#[cfg(feature = "arena")]
fn extract_content_disposition_value(header_value: &str, key: &str) -> Option<String> {
    let key_pattern = format!("{}=", key);

    for part in header_value.split(';') {
        let part = part.trim();
        if part.starts_with(&key_pattern) {
            let value = &part[key_pattern.len()..];
            // 따옴표 제거
            let value = value.trim_matches('"').trim_matches('\'');
            return Some(value.to_string());
        }
    }
    None
}

// Arena 데이터 내에서 특정 바이트 슬라이스 찾기
#[cfg(feature = "arena")]
fn find_slice_in_arena<'a>(arena_data: &'a [u8], target: &'a [u8]) -> Option<&'a [u8]> {
    if target.is_empty() {
        return None;
    }

    // arena_data에서 target과 같은 내용의 슬라이스 찾기
    for i in 0..=arena_data.len().saturating_sub(target.len()) {
        if &arena_data[i..i + target.len()] == target {
            return Some(&arena_data[i..i + target.len()]);
        }
    }
    None
}
