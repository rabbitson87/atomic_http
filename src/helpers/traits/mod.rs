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

use std::path::{Component, Path, PathBuf};

/// `root` 하위 안전한 경로 조합 (path traversal 방어).
/// 사용자 입력 경로의 모든 component를 검사하여
/// `..`, 절대 경로, Windows 드라이브 prefix를 거부.
/// 통과한 경우 `root.join(...)` 결과 반환.
///
/// 반환된 경로는 `root` 하위에 있음이 보장 (canonicalize 불필요 — 파일 없을 수 있음).
pub fn safe_path_join<R: AsRef<Path>, U: AsRef<Path>>(root: R, user_path: U) -> Option<PathBuf> {
    let mut result = root.as_ref().to_path_buf();
    for component in user_path.as_ref().components() {
        match component {
            Component::Normal(seg) => result.push(seg),
            Component::CurDir => {} // "." 무시 (안전)
            // ".." (ParentDir), 절대 RootDir, Windows Prefix(C:\) 는 모두 거부
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::safe_path_join;
    use std::path::PathBuf;

    #[test]
    fn safe_path_join_appends_normal_segments() {
        // 정상 케이스: 상대 경로의 모든 component가 Normal이면 join 그대로 반환.
        let got = safe_path_join("/srv/static", "css/app.css").unwrap();
        assert_eq!(
            got,
            PathBuf::from("/srv/static").join("css").join("app.css")
        );
    }

    #[test]
    fn safe_path_join_rejects_parent_dir() {
        // ".." 가 어디에 있어도 거부 (escape 시도).
        assert!(safe_path_join("/srv/static", "../etc/passwd").is_none());
        assert!(safe_path_join("/srv/static", "a/../../b").is_none());
        assert!(safe_path_join("/srv/static", "a/b/..").is_none());
    }

    #[test]
    fn safe_path_join_rejects_absolute_path() {
        // 절대 경로 시작 (Unix "/...", Windows에서는 RootDir 컴포넌트로 분해됨).
        assert!(safe_path_join("/srv/static", "/etc/passwd").is_none());
    }

    #[test]
    fn safe_path_join_allows_current_dir() {
        // "./foo" 같은 CurDir는 안전하게 무시되어야 함 — 결과는 그냥 root/foo.
        let got = safe_path_join("/srv/static", "./index.html").unwrap();
        assert_eq!(got, PathBuf::from("/srv/static").join("index.html"));
    }

    #[test]
    fn safe_path_join_handles_empty_user_path() {
        // 빈 경로 입력은 root 그대로 반환.
        let got = safe_path_join("/srv/static", "").unwrap();
        assert_eq!(got, PathBuf::from("/srv/static"));
    }

    #[cfg(windows)]
    #[test]
    fn safe_path_join_rejects_windows_prefix() {
        // Windows에서 "C:\..." 는 Prefix 컴포넌트 → 거부.
        assert!(safe_path_join("D:\\srv\\static", "C:\\Windows\\System32\\cmd.exe").is_none());
    }
}
