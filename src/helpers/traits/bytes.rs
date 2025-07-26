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
