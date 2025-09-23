pub trait SplitBytes {
    fn split_bytes(&self, delimiter: &[u8]) -> Vec<Vec<u8>>;
    fn split_header_body(&self) -> (Vec<u8>, Vec<u8>);
}

#[cfg(feature = "simd")]
mod simd {
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    /// SIMD를 사용하여 \r\n\r\n 패턴을 빠르게 찾기
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse2")]
    pub unsafe fn find_header_end_sse2(data: &[u8]) -> Option<usize> {
        if data.len() < 16 {
            return find_header_end_scalar(data);
        }

        let pattern = [b'\r', b'\n', b'\r', b'\n'];
        let _pattern_vec = _mm_set_epi8(
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            pattern[3] as i8, pattern[2] as i8, pattern[1] as i8, pattern[0] as i8
        );

        let mut i = 0;
        let end = data.len() - 15;

        while i < end {
            let chunk = _mm_loadu_si128(data.as_ptr().add(i) as *const __m128i);

            // \r 검색
            let cr_cmp = _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\r' as i8));
            let cr_mask = _mm_movemask_epi8(cr_cmp) as u16;

            if cr_mask != 0 {
                // \r이 발견된 위치들을 확인
                for bit_pos in 0..16 {
                    if (cr_mask & (1 << bit_pos)) != 0 {
                        let pos = i + bit_pos;
                        if pos + 4 <= data.len() && data[pos..pos + 4] == pattern {
                            return Some(pos);
                        }
                    }
                }
            }
            i += 16;
        }

        // 남은 부분은 스칼라로 처리
        find_header_end_scalar(&data[i..])
            .map(|pos| pos + i)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    pub unsafe fn find_header_end_avx2(data: &[u8]) -> Option<usize> {
        if data.len() < 32 {
            return find_header_end_sse2(data);
        }

        let pattern = [b'\r', b'\n', b'\r', b'\n'];

        let mut i = 0;
        let end = data.len() - 31;

        while i < end {
            let chunk = _mm256_loadu_si256(data.as_ptr().add(i) as *const __m256i);

            // \r 검색
            let cr_cmp = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\r' as i8));
            let cr_mask = _mm256_movemask_epi8(cr_cmp) as u32;

            if cr_mask != 0 {
                // \r이 발견된 위치들을 확인
                for bit_pos in 0..32 {
                    if (cr_mask & (1 << bit_pos)) != 0 {
                        let pos = i + bit_pos;
                        if pos + 4 <= data.len() && data[pos..pos + 4] == pattern {
                            return Some(pos);
                        }
                    }
                }
            }
            i += 32;
        }

        // 남은 부분은 SSE2로 처리
        find_header_end_sse2(&data[i..])
            .map(|pos| pos + i)
    }

    fn find_header_end_scalar(data: &[u8]) -> Option<usize> {
        let pattern = b"\r\n\r\n";
        for i in 0..=data.len().saturating_sub(4) {
            if &data[i..i + 4] == pattern {
                return Some(i);
            }
        }
        None
    }

    pub fn find_header_end_simd(data: &[u8]) -> Option<usize> {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                return unsafe { find_header_end_avx2(data) };
            } else if is_x86_feature_detected!("sse2") {
                return unsafe { find_header_end_sse2(data) };
            }
        }

        // Fallback to scalar implementation
        find_header_end_scalar(data)
    }
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
        #[cfg(feature = "simd")]
        {
            if let Some(pos) = simd::find_header_end_simd(self) {
                let header = self[..pos].to_vec();
                let body = self[pos + 4..].to_vec();
                return (header, body);
            }
        }

        #[cfg(not(feature = "simd"))]
        {
            // 기존 스칼라 방식
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
            return (header, body);
        }

        // 패턴을 찾지 못한 경우 전체를 헤더로 처리
        (self.to_vec(), Vec::new())
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
        #[cfg(feature = "simd")]
        {
            if let Some(pos) = simd::find_header_end_simd(self) {
                let header = &self[..pos];
                let body = &self[pos + 4..];
                return (header, body);
            }
        }

        #[cfg(not(feature = "simd"))]
        {
            // "\r\n\r\n" 찾기 (기존 스칼라 방식)
            let delimiter = b"\r\n\r\n";

            for i in 0..self.len() {
                if i + delimiter.len() <= self.len() && &self[i..i + delimiter.len()] == delimiter {
                    let header = &self[..i];
                    let body = &self[i + delimiter.len()..];
                    return (header, body);
                }
            }
        }

        // 구분자를 찾지 못한 경우 전체를 헤더로 처리
        (self, &[])
    }
}
