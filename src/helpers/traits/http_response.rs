use async_trait::async_trait;
use http::Response;
use tokio::io::AsyncWriteExt;

#[cfg(feature = "arena")]
use crate::ArenaWriter;
use crate::{SendableError, Writer};
#[cfg(feature = "response_file")]
use std::path::Path;

use crate::helpers::traits::zero_copy::ZeroCopyCache;

impl Writer {
    pub async fn write_bytes(&mut self) -> Result<(), SendableError> {
        self.stream.send_bytes(self.bytes.as_slice()).await?;
        Ok(())
    }

    #[cfg(feature = "response_file")]
    pub fn response_file<P>(&mut self, path: P) -> Result<(), SendableError>
    where
        P: AsRef<Path>,
    {
        let root_path = &self.options.root_path;
        let file_path = root_path.join(path);

        // 제로카피 기능이 활성화된 경우 memmap2 사용 시도
        // 파일 크기 확인
        if let Ok(metadata) = std::fs::metadata(&file_path) {
            let file_size = metadata.len() as usize;

            // 작은 파일들은 제로카피 캐시 사용
            if file_size <= 10 * 1024 * 1024 {
                // 10MB 이하
                crate::dev_print!(
                    "Using zero-copy for file: {:?} ({}KB)",
                    file_path,
                    file_size / 1024
                );
                self.body = format!("__ZERO_COPY_FILE__:{}", file_path.to_str().unwrap());
                self.use_file = true;
                return Ok(());
            }
        }

        // 기존 방식 (대용량 파일 또는 zero_copy 기능 비활성화)
        self.body = file_path.to_str().unwrap().to_string();
        self.use_file = true;
        Ok(())
    }
}

#[async_trait]
pub trait ResponseUtil {
    async fn responser(&mut self) -> Result<(), SendableError>;

    async fn send_zero_copy_file(&mut self, mut send_string: String) -> Result<(), SendableError>;
}

#[async_trait]
impl ResponseUtil for Response<Writer> {
    async fn responser(&mut self) -> Result<(), SendableError> {
        let mut send_string = String::new();
        if cfg!(feature = "response_file") && self.body().use_file {
            use http::StatusCode;
            *self.status_mut() = StatusCode::from_u16(200)?;
        }
        let status_line = format!("{:?} {}\r\n", self.version(), self.status());
        send_string.push_str(&status_line);

        if cfg!(feature = "response_file") && self.body().use_file {
            use tokio::{
                fs,
                io::{self, AsyncReadExt},
            };

            #[cfg(feature = "response_file")]
            {
                use http::header::CONTENT_TYPE;
                self.headers_mut().remove(CONTENT_TYPE);

                // 제로카피 파일 처리 확인
                if self.body().body.starts_with("__ZERO_COPY_FILE__:") {
                    return self.send_zero_copy_file(send_string).await;
                }

                // 기존 파일 처리 방식
                match self.body().body.split('.').last().unwrap() {
                    "zip" => {
                        send_string.push_str("Content-Type: application/zip\r\n");
                        send_string.push_str(&format!(
                            "content-disposition: attachment; filename={}\r\n",
                            self.body().body
                        ));
                    }
                    _ => {
                        send_string.push_str(&format!(
                            "Content-Type: {}\r\n",
                            get_content_type(&self.body().body)
                        ));
                    }
                }
            }

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }

            let file = fs::File::open(&self.body().body).await?;
            let content_length = file.metadata().await?.len();
            send_string.push_str(format!("content-length: {}\r\n", content_length).as_str());

            send_string.push_str("\r\n");

            // 여기서 mutable borrow 문제 해결: body_mut()을 한 번만 호출
            let body = self.body_mut();
            body.stream.send_bytes(send_string.as_bytes()).await?;

            let mut reader = io::BufReader::new(file);
            let mut buffer = match content_length < 1048576 * 5 {
                true => vec![0; content_length as usize],
                false => vec![0; 1048576 * 5],
            };
            while let Ok(len) = reader.read(&mut buffer).await {
                if len == 0 {
                    break;
                }
                body.stream.send_bytes(&buffer[0..len]).await?;
            }
        } else if !self.body().bytes.is_empty() {
            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            send_string.push_str("\r\n");
            let mut send_bytes = send_string.as_bytes().to_vec();
            send_bytes.extend(self.body().bytes.clone());

            // mutable borrow 문제 해결
            let body = self.body_mut();
            body.bytes = send_bytes;
            body.write_bytes().await?;
        } else {
            let (body_str, content_string) = get_body(self.body().body.as_str()).await;
            send_string.push_str(&content_string);

            for (key, value) in self.headers().iter() {
                send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
            }
            crate::dev_print!("headers: {}", &send_string);
            send_string.push_str("\r\n");

            send_string.push_str(&body_str);

            // mutable borrow 문제 해결
            self.body_mut()
                .stream
                .send_bytes(send_string.as_bytes())
                .await?;
        }

        // flush는 별도로 처리
        self.body_mut().stream.flush().await?;
        Ok(())
    }

    async fn send_zero_copy_file(&mut self, mut send_string: String) -> Result<(), SendableError> {
        use http::header::CONTENT_TYPE;
        let file_path = &self.body().body[19..]; // "__ZERO_COPY_FILE__:" 이후의 경로

        // 캐시를 사용한 파일 로드
        let cache = ZeroCopyCache::global();
        let file_result = cache.load_file(file_path)?;
        let file_data = file_result.as_bytes();
        let content_length = file_data.len();

        let load_method = if file_result.is_memory_cached() {
            "memory_cache"
        } else {
            "mmap"
        };
        crate::dev_print!(
            "Zero-copy file serving: {} ({} bytes, method: {})",
            file_path,
            content_length,
            load_method
        );

        // Content-Type 설정
        match file_path.split('.').last().unwrap() {
            "zip" => {
                send_string.push_str("Content-Type: application/zip\r\n");
                send_string.push_str(&format!(
                    "content-disposition: attachment; filename={}\r\n",
                    file_path.split('/').last().unwrap_or(file_path)
                ));
            }
            "json" => {
                send_string.push_str("Content-Type: application/json\r\n");
            }
            _ => {
                send_string.push_str(&format!(
                    "Content-Type: {}\r\n",
                    get_content_type(file_path)
                ));
            }
        }

        self.headers_mut().remove(CONTENT_TYPE);

        // 추가 헤더들
        for (key, value) in self.headers().iter() {
            send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
        }

        // Content-Length와 헤더 끝
        send_string.push_str(&format!("content-length: {}\r\n", content_length));
        send_string.push_str("\r\n");

        let body = self.body_mut();

        #[cfg(feature = "vectored_io")]
        {
            // Vectored I/O로 헤더와 파일 데이터를 한 번에 전송
            use std::io::IoSlice;
            let header_slice = IoSlice::new(send_string.as_bytes());
            let file_slice = IoSlice::new(file_data);
            let bufs = [header_slice, file_slice];

            body.stream.send_vectored(&bufs).await?;
            crate::dev_print!(
                "Vectored I/O: sent header+file in single syscall ({} + {} bytes)",
                send_string.len(),
                content_length
            );
        }

        #[cfg(not(feature = "vectored_io"))]
        {
            // 기존 방식: 별도 전송
            body.stream.send_bytes(send_string.as_bytes()).await?;
            body.stream.send_bytes(file_data).await?;
        }

        crate::dev_print!(
            "Zero-copy file sent successfully: {} bytes ({})",
            content_length,
            load_method
        );
        Ok(())
    }
}

#[cfg(feature = "arena")]
#[async_trait]
pub trait ResponseUtilArena {
    async fn responser_arena(&mut self) -> Result<(), SendableError>;

    #[cfg(feature = "arena")]
    async fn send_arena_zero_copy_file(
        &mut self,
        file_path: &str,
        mut send_string: String,
    ) -> Result<(), SendableError>;
}

#[cfg(feature = "arena")]
#[async_trait]
impl ResponseUtilArena for Response<ArenaWriter> {
    async fn responser_arena(&mut self) -> Result<(), SendableError> {
        let mut send_string = String::new();

        if cfg!(feature = "response_file") && self.body().use_file {
            use http::StatusCode;
            *self.status_mut() = StatusCode::from_u16(200)?;
        }

        let status_line = format!("{:?} {}\r\n", self.version(), self.status());
        send_string.push_str(&status_line);

        if cfg!(feature = "response_file") && self.body().use_file {
            #[cfg(feature = "response_file")]
            {
                use http::header::CONTENT_TYPE;
                use tokio::{
                    fs,
                    io::{self, AsyncReadExt},
                };
                self.headers_mut().remove(CONTENT_TYPE);

                if self.body().response_data_len > 0 {
                    let file_path = unsafe {
                        let data = std::slice::from_raw_parts(
                            self.body().response_data_ptr,
                            self.body().response_data_len,
                        );
                        std::str::from_utf8(data)?
                    };

                    // Arena + 제로카피 조합 처리
                    if file_path.starts_with("__ZERO_COPY_FILE__:") {
                        let actual_path = &file_path[19..];
                        return self
                            .send_arena_zero_copy_file(actual_path, send_string)
                            .await;
                    }

                    match file_path.split('.').last().unwrap() {
                        "zip" => {
                            send_string.push_str("Content-Type: application/zip\r\n");
                            send_string.push_str(&format!(
                                "content-disposition: attachment; filename={}\r\n",
                                file_path
                            ));
                        }
                        _ => {
                            send_string.push_str(&format!(
                                "Content-Type: {}\r\n",
                                get_content_type(file_path)
                            ));
                        }
                    }

                    for (key, value) in self.headers().iter() {
                        send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                    }

                    let file = fs::File::open(file_path).await?;
                    let content_length = file.metadata().await?.len();
                    send_string
                        .push_str(format!("content-length: {}\r\n", content_length).as_str());

                    send_string.push_str("\r\n");

                    // mutable borrow 문제 해결
                    let body = self.body_mut();
                    body.stream.send_bytes(send_string.as_bytes()).await?;

                    let mut reader = io::BufReader::new(file);
                    let mut buffer = match content_length < 1048576 * 5 {
                        true => vec![0; content_length as usize],
                        false => vec![0; 1048576 * 5],
                    };
                    while let Ok(len) = reader.read(&mut buffer).await {
                        if len == 0 {
                            break;
                        }
                        body.stream.send_bytes(&buffer[0..len]).await?;
                    }
                }
            }
        } else {
            if self.body().response_data_len > 0 {
                // Arena 메모리로 할당된 응답 데이터 사용
                for (key, value) in self.headers().iter() {
                    send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                }

                let content_length =
                    format!("content-length: {}\r\n", self.body().response_data_len);
                send_string.push_str(&content_length);
                send_string.push_str("\r\n");

                // mutable borrow 문제 해결: body_mut()을 한 번만 호출
                let body = self.body_mut();

                // Arena 데이터 직접 전송 (제로카피)
                let response_data = unsafe {
                    std::slice::from_raw_parts(body.response_data_ptr, body.response_data_len)
                };

                #[cfg(feature = "vectored_io")]
                {
                    // Vectored I/O로 헤더와 응답 데이터를 한 번에 전송
                    use std::io::IoSlice;
                    let header_slice = IoSlice::new(send_string.as_bytes());
                    let data_slice = IoSlice::new(response_data);
                    let bufs = [header_slice, data_slice];

                    body.stream.send_vectored(&bufs).await?;
                    crate::dev_print!(
                        "Arena Vectored I/O: sent header+data in single syscall ({} + {} bytes)",
                        send_string.len(),
                        response_data.len()
                    );
                }

                #[cfg(not(feature = "vectored_io"))]
                {
                    // 기존 방식: 별도 전송
                    body.stream.send_bytes(send_string.as_bytes()).await?;
                    body.stream.send_bytes(response_data).await?;
                }
            } else {
                // 빈 응답
                for (key, value) in self.headers().iter() {
                    send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
                }
                send_string.push_str("content-length: 0\r\n");
                send_string.push_str("\r\n");

                self.body_mut()
                    .stream
                    .send_bytes(send_string.as_bytes())
                    .await?;
            }
        }

        // flush는 별도로 처리
        self.body_mut().stream.flush().await?;
        Ok(())
    }

    #[cfg(feature = "arena")]
    async fn send_arena_zero_copy_file(
        &mut self,
        file_path: &str,
        mut send_string: String,
    ) -> Result<(), SendableError> {
        use http::header::CONTENT_TYPE;

        // 캐시를 사용한 파일 로드
        let file_result = ZeroCopyCache::global().load_file(file_path)?;
        let file_data = file_result.as_bytes();
        let content_length = file_data.len();

        let load_method = if file_result.is_memory_cached() {
            "arena+memory_cache"
        } else {
            "arena+mmap"
        };
        crate::dev_print!(
            "Arena + Zero-copy file serving: {} ({} bytes, method: {})",
            file_path,
            content_length,
            load_method
        );

        self.headers_mut().remove(CONTENT_TYPE);

        // Content-Type 설정
        match file_path.split('.').last().unwrap() {
            "zip" => {
                send_string.push_str("Content-Type: application/zip\r\n");
                send_string.push_str(&format!(
                    "content-disposition: attachment; filename={}\r\n",
                    file_path.split('/').last().unwrap_or(file_path)
                ));
            }
            "json" => {
                send_string.push_str("Content-Type: application/json\r\n");
            }
            _ => {
                send_string.push_str(&format!(
                    "Content-Type: {}\r\n",
                    get_content_type(file_path)
                ));
            }
        }

        // 추가 헤더들
        for (key, value) in self.headers().iter() {
            send_string.push_str(&format!("{}: {}\r\n", key.as_str(), value.to_str()?));
        }

        // Content-Length와 헤더 끝
        send_string.push_str(&format!("content-length: {}\r\n", content_length));
        send_string.push_str("\r\n");

        // mutable borrow 문제 해결: body_mut()을 한 번만 호출
        let body = self.body_mut();

        #[cfg(feature = "vectored_io")]
        {
            // Vectored I/O로 헤더와 파일 데이터를 한 번에 전송
            use std::io::IoSlice;
            let header_slice = IoSlice::new(send_string.as_bytes());
            let file_slice = IoSlice::new(file_data);
            let bufs = [header_slice, file_slice];

            body.stream.send_vectored(&bufs).await?;
            crate::dev_print!(
                "Arena Vectored I/O: sent header+file in single syscall ({} + {} bytes)",
                send_string.len(),
                content_length
            );
        }

        #[cfg(not(feature = "vectored_io"))]
        {
            // 기존 방식: 별도 전송
            body.stream.send_bytes(send_string.as_bytes()).await?;
            body.stream.send_bytes(file_data).await?;
        }

        crate::dev_print!(
            "Arena + Zero-copy file sent: {} bytes ({})",
            content_length,
            load_method
        );
        Ok(())
    }
}

#[async_trait]
pub trait SendBytes {
    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), SendableError>;

    #[cfg(feature = "vectored_io")]
    async fn send_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> Result<(), SendableError>;
}

#[async_trait]
impl SendBytes for tokio::net::TcpStream {
    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), SendableError> {
        #[cfg(feature = "vectored_io")]
        {
            use tokio::io::AsyncWriteExt;
            self.write_vectored(&[std::io::IoSlice::new(bytes)]).await?;
        }

        #[cfg(not(feature = "vectored_io"))]
        {
            use tokio::io::AsyncWriteExt;
            self.write_all(bytes).await?;
        }
        Ok(())
    }

    #[cfg(feature = "vectored_io")]
    async fn send_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> Result<(), SendableError> {
        use tokio::io::AsyncWriteExt;
        self.write_vectored(bufs).await?;
        Ok(())
    }
}

pub fn get_content_type(file_name: &str) -> String {
    let guess = mime_guess::from_path(file_name);

    if let Some(mime) = guess.first() {
        mime.to_string()
    } else {
        use std::str::FromStr;

        String::from_str("text/plain").unwrap_or_default()
    }
}

async fn get_body(body: &str) -> (String, String) {
    let length = body.len();

    let content_length = format!("content-length: {}\r\n", length);
    crate::dev_print!("content-length: {}\n", &content_length);
    (body.into(), content_length)
}
