pub mod bytes;
pub mod http_request;
pub mod http_response;
pub mod http_stream;
pub mod zero_copy;

// GetHeaderChild trait 정의
use std::collections::HashMap;

pub trait GetHeaderChild {
    fn get_header_child(&self) -> HashMap<&str, &str>;
}

impl GetHeaderChild for &str {
    fn get_header_child(&self) -> HashMap<&str, &str> {
        let mut headers = HashMap::new();

        // Content-Disposition 파싱: form-data; name="field"; filename="file.txt"
        for part in self.split(';') {
            let part = part.trim();
            if let Some(eq_pos) = part.find('=') {
                let key = part[..eq_pos].trim();
                let value = part[eq_pos + 1..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                headers.insert(key, value);
            }
        }

        headers
    }
}

// StringUtil trait 정의
pub trait StringUtil {
    fn copy_string(&self) -> String;
}

impl StringUtil for String {
    fn copy_string(&self) -> String {
        self.clone()
    }
}

pub fn find_header_end_optimized(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

pub fn extract_content_length_fast(headers: &[u8]) -> Option<usize> {
    let headers_str = std::str::from_utf8(headers).ok()?;
    headers_str
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split(':').nth(1)?.trim().parse().ok())
}
