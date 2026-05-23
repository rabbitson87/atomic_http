use async_trait::async_trait;
use http::HeaderName;
use http::Request;
use serde::Deserialize;

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
        // buffered_bytes(): parse_request로 들어온 요청은 body가 leftover에 모두 들어있음.
        // streaming 모드 (parse_request_streaming) 를 쓴 경우 핸들러가 먼저 body.bytes(cap).await? 호출 필요.
        let bytes = self.body().buffered_bytes();
        if bytes.is_empty() {
            return Err("Empty body".into());
        }
        let parsed: T = serde_json::from_slice(bytes)?;
        Ok(parsed)
    }

    fn get_text(&mut self) -> Result<String, SendableError> {
        // UTF-8 엄격 검증 — 잘못된 시퀀스면 에러.
        let bytes = self.body().buffered_bytes();
        if bytes.is_empty() {
            return Ok(String::new());
        }
        Ok(std::str::from_utf8(bytes)?.to_string())
    }

    async fn get_multi_part(&mut self) -> Result<Option<Form>, SendableError> {
        // 메모리에 이미 있는 &[u8]를 처리하므로 async BufReader는 불필요.
        // 헤더/바디는 \r\n\r\n 으로 직접 분리, 바디 슬라이스는 trim 후 사용.
        let content_type_str = match self.headers().get("content-type") {
            Some(ct) => ct.to_str()?,
            None => return Ok(None),
        };
        if !content_type_str.contains("multipart/form-data") {
            return Ok(None);
        }
        let boundary = match content_type_str.split("boundary=").nth(1) {
            Some(b) => b,
            None => return Ok(None),
        };

        let boundary_marker = format!("--{}", boundary);
        let boundary_bytes = boundary_marker.as_bytes();
        let mut form = Form::new();

        for part_data in self.body().buffered_bytes().split_bytes(boundary_bytes) {
            dev_print!("part_data: {:?}", part_data.len());

            // 헤더/바디 경계 (\r\n\r\n) 찾기
            let split_pos = match part_data.windows(4).position(|w| w == b"\r\n\r\n") {
                Some(p) => p,
                None => continue,
            };
            let header_bytes = &part_data[..split_pos];
            let body_bytes = &part_data[split_pos + 4..];

            // 헤더는 ASCII 텍스트라고 가정 (HTTP 표준)
            let header_str = match std::str::from_utf8(header_bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let mut part = Part::new();
            let mut text_field_name: Option<String> = None;

            for line in header_str.split("\r\n") {
                dev_print!("{}", line);
                if line.is_empty() {
                    continue;
                }
                // content-disposition 라인은 대소문자 무관 매칭
                if line.len() >= 19
                    && line.as_bytes()[..19].eq_ignore_ascii_case(b"content-disposition")
                {
                    if let Some(value) = line.split(": ").nth(1) {
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
                                text_field_name = Some(std::mem::take(name).into());
                            }
                        }
                    }
                } else if let Some((key, value)) = line.split_once(": ") {
                    if let Ok(hname) =
                        HeaderName::from_lowercase(key.to_ascii_lowercase().as_bytes())
                    {
                        if let Ok(hval) = value.parse() {
                            part.headers.insert(hname, hval);
                        }
                    }
                }
            }

            // 바디 끝 \r\n trim (boundary 라인 앞 구분자)
            let mut body_end = body_bytes.len();
            while body_end > 0
                && (body_bytes[body_end - 1] == b'\n' || body_bytes[body_end - 1] == b'\r')
            {
                body_end -= 1;
            }
            let body_trimmed = &body_bytes[..body_end];

            if !part.file_name.is_empty() {
                part.body = body_trimmed.to_vec();
                form.parts.push(part);
            } else if let Some(name) = text_field_name {
                form.text_fields
                    .push((name, String::from_utf8_lossy(body_trimmed).into_owned()));
            }
        }

        Ok(Some(form))
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
            // 텍스트 필드용 name 슬라이스(arena 내부 포인터)를 기억해 둠
            let mut name_bytes_for_text: Option<&[u8]> = None;

            // 헤더 파싱 - header_str은 arena_data에서 파생된 슬라이스이므로
            // line, header_name, header_value 모두 이미 arena 내부 포인터를 가짐.
            // 따라서 다시 검색할 필요 없이 그대로 사용.
            for line in header_str.split("\r\n") {
                if line.trim().is_empty() {
                    continue;
                }

                if let Some(colon_pos) = line.find(": ") {
                    let header_name = &line[..colon_pos];
                    let header_value = &line[colon_pos + 2..];

                    if header_name.eq_ignore_ascii_case("content-disposition") {
                        if let Some(name_value) =
                            extract_content_disposition_value(header_value, "name")
                        {
                            let slice = name_value.as_bytes();
                            part.set_name(slice);
                            name_bytes_for_text = Some(slice);
                        }

                        if let Some(filename_value) =
                            extract_content_disposition_value(header_value, "filename")
                        {
                            part.set_file_name(filename_value.as_bytes());
                            is_file = true;
                        }
                    } else if header_name.eq_ignore_ascii_case("content-type") {
                        part.set_content_type(header_value.as_bytes());
                    }

                    // 헤더 추가 - arena 내부 슬라이스 직접 사용
                    part.headers.push(ArenaHeaderRef::new(
                        header_name.as_bytes(),
                        header_value.as_bytes(),
                    ));
                }
            }

            if part.get_name().is_none() {
                continue;
            }

            if is_file {
                form.parts.push(part);
            } else if !body_bytes.is_empty() {
                if let Some(name_slice) = name_bytes_for_text {
                    form.text_fields
                        .push(ArenaTextFieldRef::new(name_slice, body_bytes));
                }
            }
        }

        Ok(Some(form))
    }
}

// Content-Disposition에서 특정 값 추출 - 입력 슬라이스의 부분 슬라이스를 반환 (할당 없음)
#[cfg(feature = "arena")]
fn extract_content_disposition_value<'a>(header_value: &'a str, key: &str) -> Option<&'a str> {
    let key_bytes = key.as_bytes();
    for part in header_value.split(';') {
        let part = part.trim();
        let pb = part.as_bytes();
        // "key=" 접두사 확인 (key는 대소문자 구분 없이)
        if pb.len() > key_bytes.len()
            && pb[key_bytes.len()] == b'='
            && pb[..key_bytes.len()].eq_ignore_ascii_case(key_bytes)
        {
            let value = &part[key_bytes.len() + 1..];
            // 따옴표 제거 (슬라이스 유지)
            let value = value.trim_matches('"').trim_matches('\'');
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Request;

    fn make_multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        // parts: (name, filename, body_bytes)
        let mut buf = Vec::new();
        for (name, filename, body) in parts {
            buf.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            match filename {
                Some(f) => buf.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                        name, f
                    )
                    .as_bytes(),
                ),
                None => buf.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n", name).as_bytes(),
                ),
            }
            buf.extend_from_slice(b"\r\n");
            buf.extend_from_slice(body);
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
        buf
    }

    fn build_req(body: Vec<u8>, boundary: &str) -> Request<Body> {
        Request::builder()
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from_bytes(body, None))
            .unwrap()
    }

    #[tokio::test]
    async fn multipart_collects_all_text_fields() {
        // 회귀 방지: 이전 Form.text 단일 튜플은 마지막 필드만 남는 버그가 있었음.
        let boundary = "BNDRY";
        let body = make_multipart_body(
            boundary,
            &[
                ("name", None, b"alice"),
                ("age", None, b"30"),
                ("city", None, b"seoul"),
            ],
        );
        let mut req = build_req(body, boundary);
        let form = req.get_multi_part().await.unwrap().expect("form expected");
        assert_eq!(form.text_fields.len(), 3);
        assert_eq!(form.find_text_field("name"), Some("alice"));
        assert_eq!(form.find_text_field("age"), Some("30"));
        assert_eq!(form.find_text_field("city"), Some("seoul"));
        assert_eq!(form.parts.len(), 0);
    }

    #[tokio::test]
    async fn multipart_separates_file_and_text_parts() {
        let boundary = "BNDRY";
        let body = make_multipart_body(
            boundary,
            &[
                ("description", None, b"a small file"),
                ("upload", Some("hello.bin"), b"\x00\x01\x02\x03"),
            ],
        );
        let mut req = build_req(body, boundary);
        let form = req.get_multi_part().await.unwrap().unwrap();
        assert_eq!(form.text_fields.len(), 1);
        assert_eq!(form.find_text_field("description"), Some("a small file"));
        assert_eq!(form.parts.len(), 1);
        let p = &form.parts[0];
        assert_eq!(p.name, "upload");
        assert_eq!(p.file_name, "hello.bin");
        assert_eq!(p.body, vec![0x00, 0x01, 0x02, 0x03]);
    }

    #[tokio::test]
    async fn multipart_returns_none_for_non_multipart() {
        let mut req: Request<Body> = Request::builder()
            .header("content-type", "application/json")
            .body(Body::from_bytes(b"{}".to_vec(), None))
            .unwrap();
        let r = req.get_multi_part().await.unwrap();
        assert!(r.is_none());
    }

    fn body_req(bytes: Vec<u8>) -> Request<Body> {
        Request::builder()
            .body(Body::from_bytes(bytes, None))
            .unwrap()
    }

    #[test]
    fn get_text_returns_string_for_valid_utf8() {
        // 정상 UTF-8 → from_utf8 통과, 새 String 반환.
        let mut req = body_req("안녕 hello".as_bytes().to_vec());
        let s = req.get_text().unwrap();
        assert_eq!(s, "안녕 hello");
    }

    #[test]
    fn get_text_returns_empty_for_empty_body() {
        // 0바이트 body는 빈 문자열 (에러 아님).
        let mut req = body_req(Vec::new());
        let s = req.get_text().unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn get_text_rejects_invalid_utf8() {
        // 핵심 회귀 방지: 0.14.0에서 from_utf8_lossy 제거 → invalid 시 Err.
        // (이전 lossy 동작은 0xFF/0xFE를 U+FFFD로 silent 치환했음.)
        let mut req = body_req(vec![0xFF, 0xFE, 0xFD]);
        assert!(req.get_text().is_err());
    }

    #[test]
    fn get_json_parses_valid_payload() {
        // from_slice 직접 호출: UTF-8 검증 + JSON 파싱 동시.
        let mut req = body_req(br#"{"k":1,"v":"hi"}"#.to_vec());
        let v: serde_json::Value = req.get_json().unwrap();
        assert_eq!(v["k"], 1);
        assert_eq!(v["v"], "hi");
    }

    #[test]
    fn get_json_rejects_empty_body() {
        // 빈 body는 명시적 에러 (이전엔 serde_json의 EOF 에러로 떨어졌음).
        let mut req = body_req(Vec::new());
        assert!(req.get_json::<serde_json::Value>().is_err());
    }

    #[test]
    fn get_json_rejects_invalid_utf8() {
        // serde_json::from_slice는 비-UTF8을 거부. 회귀 방지.
        let mut req = body_req(vec![0xFF, 0xFE, 0xFD]);
        assert!(req.get_json::<serde_json::Value>().is_err());
    }
}
