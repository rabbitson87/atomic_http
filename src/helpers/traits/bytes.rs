pub trait SplitBytes {
    fn split_bytes(&self, delimiter: &[u8]) -> Vec<Vec<u8>>;
    fn split_header_body(&self) -> (Vec<u8>, Vec<u8>);
}

impl SplitBytes for &[u8] {
    fn split_bytes(&self, delimiter: &[u8]) -> Vec<Vec<u8>> {
        let mut result = Vec::new();
        let mut start = 0;
        for (i, _) in self.iter().enumerate() {
            if self[i..].starts_with(delimiter) {
                let bytes = self[start..i].to_vec();
                if bytes.len() > 0 {
                    result.push(bytes);
                }
                start = i + delimiter.len();
            }
        }
        let last = self[start..].to_vec();
        if [45, 45, 13, 10] != last.as_slice() && last.len() > 0 {
            result.push(last);
        }
        result
    }

    fn split_header_body(&self) -> (Vec<u8>, Vec<u8>) {
        let mut header = Vec::new();
        let mut body = Vec::new();
        let mut is_header = true;
        let mut count = 0;
        for (i, _) in self.iter().enumerate() {
            if self[i..].starts_with(b"\r\n\r\n") {
                is_header = false;
            }
            if is_header {
                header.push(self[i]);
            } else {
                if count > 3 {
                    body.push(self[i]);
                }
                count += 1;
            }
        }
        (header, body)
    }
}

#[cfg(feature = "arena")]
pub trait SplitBytesArena {
    fn split_bytes_arena(&self, delimiter: &[u8]) -> Vec<&[u8]>;
    fn split_header_body_arena(&self) -> (&[u8], &[u8]);
}

#[cfg(feature = "arena")]
impl SplitBytesArena for &[u8] {
    fn split_bytes_arena(&self, delimiter: &[u8]) -> Vec<&[u8]> {
        let mut result = Vec::new();
        let mut start = 0;

        for i in 0..self.len() {
            if i + delimiter.len() <= self.len() && &self[i..i + delimiter.len()] == delimiter {
                if start < i {
                    let slice = &self[start..i];
                    if !slice.is_empty() {
                        result.push(slice);
                    }
                }
                start = i + delimiter.len();
            }
        }

        // 마지막 부분 처리
        if start < self.len() {
            let last_slice = &self[start..];
            // multipart 끝 마커 "--\r\n" 체크
            if last_slice != [45, 45, 13, 10] && !last_slice.is_empty() {
                result.push(last_slice);
            }
        }

        result
    }

    fn split_header_body_arena(&self) -> (&[u8], &[u8]) {
        // "\r\n\r\n" 찾기
        let delimiter = b"\r\n\r\n";

        for i in 0..self.len() {
            if i + delimiter.len() <= self.len() && &self[i..i + delimiter.len()] == delimiter {
                let header = &self[..i];
                let body = &self[i + delimiter.len()..];
                return (header, body);
            }
        }

        // 구분자를 찾지 못한 경우 전체를 헤더로 처리
        (self, &[])
    }
}
