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

// 헤더 끝 찾기와 Content-Length 추출은 `bytes::find_header_end` 와
// `http_stream::extract_content_length_simple` 로 통합되었습니다.
// 외부 호환을 위해 thin wrapper만 유지.
pub use crate::helpers::traits::bytes::find_header_end as find_header_end_optimized;
