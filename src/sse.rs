//! Server-Sent Events (`text/event-stream`) over the raw connection.
//!
//! atomic_http writes a response in one piece, with a `content-length`. A stream has no length,
//! so [`ResponseSse::into_sse`] takes the socket's write half instead and returns an
//! [`SseWriter`] that sends the head once and then any number of frames.
//!
//! ```no_run
//! use atomic_http::{ResponseSse, SendableError, SseEvent};
//!
//! # async fn handler(response: http::Response<atomic_http::ArenaWriter>) -> Result<(), SendableError> {
//! let mut sse = response.into_sse().await?;
//! sse.send_event("token", "안녕").await?;
//! sse.send(&SseEvent::data("multi\nline").id("42")).await?;
//! sse.finish().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Notes:
//! * The status is always `200 OK`. Send errors with `responser`/`responser_arena` instead.
//! * There is no chunked encoding here: the head carries `connection: close` and the stream ends
//!   when the socket closes, so the connection is never reused afterwards.
//! * A write to a client that went away fails; test with [`is_disconnect`].

use std::future::Future;
use std::io;
use std::time::Duration;

use async_trait::async_trait;
use http::header::{
    CACHE_CONTROL, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING,
};
use http::{HeaderMap, Response};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;

#[cfg(feature = "arena")]
use crate::ArenaWriter;
use crate::{SendableError, Writer};

/// Rejected input. Nothing is written when a frame fails validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SseError {
    /// A single-line field (`event`, `id`) contained a line break, or an `id` contained NUL.
    #[error("SSE `{field}` must not contain CR, LF{}", if *.field == "id" { " or NUL" } else { "" })]
    InvalidField { field: &'static str },
}

/// One event frame. Build with [`SseEvent::data`] (or [`SseEvent::retry`]) and the chained setters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SseEvent<'a> {
    event: Option<&'a str>,
    data: Option<&'a str>,
    id: Option<&'a str>,
    retry_ms: Option<u64>,
}

impl<'a> SseEvent<'a> {
    /// An event carrying `data`. Line breaks (`\n`, `\r\n`, `\r`) become separate `data:` lines,
    /// which the browser joins back with `\n`.
    pub fn data(data: &'a str) -> Self {
        Self {
            data: Some(data),
            ..Self::default()
        }
    }

    /// A frame that only tells the client how long to wait before reconnecting.
    pub fn retry(ms: u64) -> Self {
        Self {
            retry_ms: Some(ms),
            ..Self::default()
        }
    }

    /// Sets the event name (`addEventListener(name, ...)` on the client).
    pub fn event(mut self, name: &'a str) -> Self {
        self.event = Some(name);
        self
    }

    /// Sets the event id (echoed back as `Last-Event-ID` on reconnect).
    pub fn id(mut self, id: &'a str) -> Self {
        self.id = Some(id);
        self
    }

    /// Adds a reconnection delay in milliseconds to this event.
    pub fn with_retry(mut self, ms: u64) -> Self {
        self.retry_ms = Some(ms);
        self
    }

    /// Appends the wire form of this event, ending with the blank line that dispatches it.
    /// `out` is left untouched when validation fails.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), SseError> {
        if self.event.is_some_and(has_line_break) {
            return Err(SseError::InvalidField { field: "event" });
        }
        if self
            .id
            .is_some_and(|id| has_line_break(id) || id.contains('\0'))
        {
            return Err(SseError::InvalidField { field: "id" });
        }
        if let Some(ms) = self.retry_ms {
            out.extend_from_slice(format!("retry: {ms}\n").as_bytes());
        }
        if let Some(id) = self.id {
            out.extend_from_slice(b"id: ");
            out.extend_from_slice(id.as_bytes());
            out.push(b'\n');
        }
        if let Some(name) = self.event {
            out.extend_from_slice(b"event: ");
            out.extend_from_slice(name.as_bytes());
            out.push(b'\n');
        }
        if let Some(data) = self.data {
            for_each_line(data, |line| {
                out.extend_from_slice(b"data: ");
                out.extend_from_slice(line.as_bytes());
                out.push(b'\n');
            });
        }
        out.push(b'\n');
        Ok(())
    }

    /// The wire form as a new buffer.
    pub fn encode(&self) -> Result<Vec<u8>, SseError> {
        let mut out = Vec::with_capacity(self.data.map_or(0, str::len) + 32);
        self.encode_into(&mut out)?;
        Ok(out)
    }
}

fn has_line_break(s: &str) -> bool {
    s.contains(['\r', '\n'])
}

/// Calls `f` once per line, treating `\r\n`, `\n` and a lone `\r` as terminators (the SSE rules).
/// The text after the last terminator counts as a line, so `""` yields one empty line and
/// `"a\n"` yields `"a"` and `""`.
fn for_each_line(s: &str, mut f: impl FnMut(&str)) {
    let bytes = s.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                f(&s[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                f(&s[start..i]);
                i += 1;
                if bytes.get(i) == Some(&b'\n') {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    f(&s[start..]);
}

/// True when the error means the client closed the connection (a normal way for a stream to end).
pub fn is_disconnect(err: &SendableError) -> bool {
    err.downcast_ref::<io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected
        )
    })
}

/// Writes an event stream to one connection. Created by [`ResponseSse::into_sse`] or
/// [`SseWriter::start`]. Every frame is a single write followed by a flush.
pub struct SseWriter {
    stream: OwnedWriteHalf,
}

impl SseWriter {
    /// Writes the status line and headers. `headers` are the caller's extra headers (for example
    /// `set-cookie` or CORS). `content-type`, `content-length`, `transfer-encoding`,
    /// `content-encoding` and `connection` are ignored because they would break the stream;
    /// `cache-control` (default `no-cache`) can be overridden.
    pub async fn start(
        mut stream: OwnedWriteHalf,
        headers: &HeaderMap,
    ) -> Result<Self, SendableError> {
        stream.write_all(&head(headers)).await?;
        stream.flush().await?;
        Ok(Self { stream })
    }

    /// Sends a fully built event.
    pub async fn send(&mut self, event: &SseEvent<'_>) -> Result<(), SendableError> {
        let buf = event.encode()?;
        self.write_frame(&buf).await
    }

    /// Sends `event: <name>` with `data`.
    pub async fn send_event(&mut self, name: &str, data: &str) -> Result<(), SendableError> {
        self.send(&SseEvent::data(data).event(name)).await
    }

    /// Sends a nameless event (`onmessage` on the client).
    pub async fn send_data(&mut self, data: &str) -> Result<(), SendableError> {
        self.send(&SseEvent::data(data)).await
    }

    /// Sends a comment. Clients ignore it; it keeps idle connections open and pushes bytes through
    /// proxies that buffer. Line breaks in `text` become separate comment lines.
    pub async fn send_comment(&mut self, text: &str) -> Result<(), SendableError> {
        let mut buf = Vec::with_capacity(text.len() + 8);
        for_each_line(text, |line| {
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
        });
        buf.push(b'\n');
        self.write_frame(&buf).await
    }

    /// Runs `fut` while sending a comment every `interval`, so a slow step (a model call, a
    /// database query) does not let the connection go idle. Returns the future's output.
    ///
    /// `fut` cannot borrow this writer. If a keep-alive write fails because the client left, the
    /// error is returned and `fut` is dropped. A zero `interval` is treated as 1 ms.
    pub async fn keepalive_while<F: Future>(
        &mut self,
        interval: Duration,
        fut: F,
    ) -> Result<F::Output, SendableError> {
        let interval = interval.max(Duration::from_millis(1));
        let mut fut = std::pin::pin!(fut);
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                out = &mut fut => return Ok(out),
                _ = tick.tick() => self.send_comment("keepalive").await?,
            }
        }
    }

    /// Ends the stream: sends FIN so the client sees the response complete.
    pub async fn finish(mut self) -> Result<(), SendableError> {
        self.stream.flush().await?;
        self.stream.shutdown().await?;
        Ok(())
    }

    /// Gives the socket's write half back, for callers that need to write something custom.
    pub fn into_inner(self) -> OwnedWriteHalf {
        self.stream
    }

    async fn write_frame(&mut self, buf: &[u8]) -> Result<(), SendableError> {
        self.stream.write_all(buf).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

fn head(extra: &HeaderMap) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\n");
    if !extra.contains_key(CACHE_CONTROL) {
        out.extend_from_slice(b"cache-control: no-cache\r\n");
    }
    // Tells nginx-style proxies not to buffer the stream.
    if !extra.contains_key("x-accel-buffering") {
        out.extend_from_slice(b"x-accel-buffering: no\r\n");
    }
    out.extend_from_slice(b"connection: close\r\n");
    for (name, value) in extra {
        if [
            CONTENT_TYPE,
            CONTENT_LENGTH,
            TRANSFER_ENCODING,
            CONTENT_ENCODING,
            CONNECTION,
        ]
        .contains(name)
        {
            continue;
        }
        out.extend_from_slice(name.as_str().as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// Turns a response into an [`SseWriter`], sending the SSE head with the response's own headers.
#[async_trait]
pub trait ResponseSse {
    /// Consumes the response. Set extra headers first with `headers_mut()`.
    async fn into_sse(self) -> Result<SseWriter, SendableError>;
}

#[async_trait]
impl ResponseSse for Response<Writer> {
    async fn into_sse(self) -> Result<SseWriter, SendableError> {
        let (parts, body) = self.into_parts();
        SseWriter::start(body.stream, &parts.headers).await
    }
}

#[cfg(feature = "arena")]
#[async_trait]
impl ResponseSse for Response<ArenaWriter> {
    async fn into_sse(self) -> Result<SseWriter, SendableError> {
        let (parts, body) = self.into_parts();
        SseWriter::start(body.stream, &parts.headers).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;
    use http::HeaderValue;
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::tcp::OwnedReadHalf;
    use tokio::net::{TcpListener, TcpStream};

    /// A connected pair: the client socket, plus the server's write half (and its read half,
    /// kept alive so the socket is not torn down early).
    async fn pair() -> (TcpStream, OwnedWriteHalf, OwnedReadHalf) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (client, server) = tokio::join!(TcpStream::connect(addr), listener.accept());
        let (r, w) = server.unwrap().0.into_split();
        (client.unwrap(), w, r)
    }

    async fn read_all(mut client: TcpStream) -> String {
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn split_head(raw: &str) -> (&str, &str) {
        raw.split_once("\r\n\r\n").expect("head terminator")
    }

    /// A reference decoder that follows the browser's event-stream parsing rules.
    fn decode(body: &str) -> Vec<(Option<String>, Option<String>, String)> {
        let mut events = Vec::new();
        for block in body.split("\n\n").filter(|b| !b.is_empty()) {
            let (mut name, mut id, mut data, mut has_data) = (None, None, String::new(), false);
            for line in block.split('\n') {
                if line.starts_with(':') {
                    continue;
                }
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => name = Some(value.to_string()),
                    "id" => id = Some(value.to_string()),
                    "data" => {
                        data.push_str(value);
                        data.push('\n');
                        has_data = true;
                    }
                    _ => {}
                }
            }
            if has_data {
                data.pop();
                events.push((name, id, data));
            }
        }
        events
    }

    #[test]
    fn encodes_fields_in_order_and_splits_lines() {
        let e = SseEvent::data("a\r\nb\rc\nd")
            .event("tok")
            .id("7")
            .with_retry(3000);
        assert_eq!(
            String::from_utf8(e.encode().unwrap()).unwrap(),
            "retry: 3000\nid: 7\nevent: tok\ndata: a\ndata: b\ndata: c\ndata: d\n\n"
        );
        assert_eq!(
            SseEvent::retry(500).encode().unwrap(),
            b"retry: 500\n\n".to_vec()
        );
    }

    #[test]
    fn data_round_trips_through_the_reference_decoder() {
        let cases = [
            "",
            "plain",
            "안녕하세요 예약을 도와드릴게요",
            "a\nb",
            "a\r\nb",
            "a\rb",
            "trailing\n",
            "\nleading",
            "  spaces kept  ",
            "colon: inside",
            "event: not-a-field",
            ": not a comment",
        ];
        for data in cases {
            let wire = String::from_utf8(SseEvent::data(data).encode().unwrap()).unwrap();
            let got = decode(&wire);
            assert_eq!(got.len(), 1, "{data:?} -> {wire:?}");
            // The client normalises every line break to \n.
            let want = data.replace("\r\n", "\n").replace('\r', "\n");
            assert_eq!(got[0].2, want, "{data:?}");
        }
    }

    #[test]
    fn rejects_line_breaks_and_nul_in_single_line_fields() {
        let mut out = b"keep".to_vec();
        for bad in ["a\nb", "a\rb", "a\r\nb"] {
            assert_eq!(
                SseEvent::data("x").event(bad).encode_into(&mut out),
                Err(SseError::InvalidField { field: "event" })
            );
            assert_eq!(
                SseEvent::data("x").id(bad).encode_into(&mut out),
                Err(SseError::InvalidField { field: "id" })
            );
        }
        assert_eq!(
            SseEvent::data("x").id("a\0b").encode_into(&mut out),
            Err(SseError::InvalidField { field: "id" })
        );
        // NUL is only special in ids.
        assert!(SseEvent::data("x").event("a\0b").encode().is_ok());
        assert_eq!(out, b"keep", "failed validation must not write");
    }

    #[test]
    fn error_message_names_the_field() {
        let id = SseError::InvalidField { field: "id" }.to_string();
        let ev = SseError::InvalidField { field: "event" }.to_string();
        assert!(id.contains("`id`") && id.contains("NUL"));
        assert!(ev.contains("`event`") && !ev.contains("NUL"));
    }

    #[tokio::test]
    async fn head_has_stream_headers_and_no_length() {
        let (client, w, _r) = pair().await;
        let sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        sse.finish().await.unwrap();
        let raw = read_all(client).await;
        let (head, body) = split_head(&raw);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains("content-type: text/event-stream; charset=utf-8"));
        assert!(head.contains("cache-control: no-cache"));
        assert!(head.contains("x-accel-buffering: no"));
        assert!(head.contains("connection: close"));
        assert!(!head.to_lowercase().contains("content-length"));
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn caller_headers_pass_through_but_cannot_break_the_stream() {
        let mut extra = HeaderMap::new();
        extra.append("set-cookie", HeaderValue::from_static("sid=1; HttpOnly"));
        extra.append("set-cookie", HeaderValue::from_static("theme=dark"));
        extra.insert("access-control-allow-origin", HeaderValue::from_static("*"));
        extra.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        extra.insert(CONTENT_LENGTH, HeaderValue::from_static("5"));
        extra.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        extra.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
        extra.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        extra.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        let (client, w, _r) = pair().await;
        SseWriter::start(w, &extra)
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        let raw = read_all(client).await;
        let (head, _) = split_head(&raw);
        assert_eq!(head.matches("set-cookie:").count(), 2);
        assert!(head.contains("set-cookie: sid=1; HttpOnly"));
        assert!(head.contains("access-control-allow-origin: *"));
        assert_eq!(head.matches("cache-control:").count(), 1);
        assert!(head.contains("cache-control: no-store"));
        for banned in [
            "content-length",
            "text/plain",
            "keep-alive",
            "transfer-encoding",
            "content-encoding",
        ] {
            assert!(!head.contains(banned), "{banned} leaked into {head}");
        }
        assert_eq!(head.matches("content-type:").count(), 1);
        assert_eq!(head.matches("connection:").count(), 1);
    }

    #[tokio::test]
    async fn frames_arrive_in_order_and_the_stream_ends_on_finish() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        sse.send_comment("open").await.unwrap();
        sse.send_event("reply", "안").await.unwrap();
        sse.send_event("reply", "녕").await.unwrap();
        sse.send_data("multi\nline").await.unwrap();
        sse.send(&SseEvent::data("{}").event("done").id("9"))
            .await
            .unwrap();
        sse.finish().await.unwrap();

        let raw = read_all(client).await;
        let (_, body) = split_head(&raw);
        assert!(body.starts_with(": open\n\n"));
        let got = decode(body);
        assert_eq!(
            got,
            vec![
                (Some("reply".into()), None, "안".into()),
                (Some("reply".into()), None, "녕".into()),
                (None, None, "multi\nline".into()),
                (Some("done".into()), Some("9".into()), "{}".into()),
            ]
        );
    }

    #[tokio::test]
    async fn invalid_field_writes_nothing_and_the_stream_stays_usable() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        let err = sse.send_event("bad\nname", "x").await.unwrap_err();
        assert_eq!(
            err.downcast_ref::<SseError>(),
            Some(&SseError::InvalidField { field: "event" })
        );
        assert!(!is_disconnect(&err));
        sse.send_data("ok").await.unwrap();
        sse.finish().await.unwrap();
        let raw = read_all(client).await;
        assert_eq!(split_head(&raw).1, "data: ok\n\n");
    }

    #[tokio::test]
    async fn a_large_event_is_written_completely() {
        let big = "가".repeat(400_000);
        let (client, w, _r) = pair().await;
        let reader = tokio::spawn(read_all(client));
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        sse.send_data(&big).await.unwrap();
        sse.finish().await.unwrap();
        let raw = reader.await.unwrap();
        assert_eq!(decode(split_head(&raw).1)[0].2, big);
    }

    #[tokio::test]
    async fn keepalive_while_pings_during_a_slow_future_and_returns_its_output() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        let out = sse
            .keepalive_while(Duration::from_millis(30), async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                42
            })
            .await
            .unwrap();
        assert_eq!(out, 42);
        sse.finish().await.unwrap();
        let raw = read_all(client).await;
        let pings = raw.matches(": keepalive\n\n").count();
        assert!((3..=7).contains(&pings), "{pings} pings");
    }

    #[tokio::test]
    async fn keepalive_while_is_silent_when_the_future_is_fast() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        let out = sse
            .keepalive_while(Duration::from_secs(5), async { "quick" })
            .await
            .unwrap();
        assert_eq!(out, "quick");
        sse.finish().await.unwrap();
        assert!(!read_all(client).await.contains("keepalive"));
        // A zero interval is clamped instead of panicking.
        let (c2, w2, _r2) = pair().await;
        let mut sse = SseWriter::start(w2, &HeaderMap::new()).await.unwrap();
        sse.keepalive_while(Duration::ZERO, async {}).await.unwrap();
        sse.finish().await.unwrap();
        drop(c2);
    }

    #[tokio::test]
    async fn writes_to_a_departed_client_fail_as_a_disconnect() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        drop(client);
        let mut failure = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Err(e) = sse.send_comment("ping").await {
                failure = Some(e);
                break;
            }
        }
        let err = failure.expect("a write must fail once the peer is gone");
        assert!(is_disconnect(&err), "{err}");
    }

    #[tokio::test]
    async fn keepalive_while_surfaces_a_disconnect_and_drops_the_future() {
        let (client, w, _r) = pair().await;
        let mut sse = SseWriter::start(w, &HeaderMap::new()).await.unwrap();
        drop(client);
        let res = sse
            .keepalive_while(Duration::from_millis(10), std::future::pending::<()>())
            .await;
        assert!(is_disconnect(&res.unwrap_err()));
    }

    #[tokio::test]
    async fn into_sse_forces_200_and_uses_response_headers_for_writer() {
        let (client, w, _r) = pair().await;
        let writer = Writer {
            stream: w,
            body: String::new(),
            bytes: Vec::new(),
            use_file: false,
            options: Arc::new(Options::new()),
        };
        // atomic_http's parser leaves responses at 400 until the handler sets a status.
        let mut response = Response::builder().status(400).body(writer).unwrap();
        response
            .headers_mut()
            .insert("set-cookie", HeaderValue::from_static("sid=abc"));
        let mut sse = response.into_sse().await.unwrap();
        sse.send_data("hi").await.unwrap();
        sse.finish().await.unwrap();
        let raw = read_all(client).await;
        let (head, body) = split_head(&raw);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(head.contains("set-cookie: sid=abc"));
        assert_eq!(body, "data: hi\n\n");
    }

    #[cfg(feature = "arena")]
    #[tokio::test]
    async fn into_sse_works_for_arena_responses() {
        let (client, w, _r) = pair().await;
        let mut response = Response::builder()
            .status(400)
            .body(ArenaWriter::new(w, Arc::new(Options::new())))
            .unwrap();
        response
            .headers_mut()
            .insert("set-cookie", HeaderValue::from_static("sid=xyz"));
        let mut sse = response.into_sse().await.unwrap();
        sse.send_event("done", "{}").await.unwrap();
        sse.finish().await.unwrap();
        let raw = read_all(client).await;
        let (head, body) = split_head(&raw);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(head.contains("set-cookie: sid=xyz"));
        assert_eq!(body, "event: done\ndata: {}\n\n");
    }

    #[test]
    fn line_splitter_matches_the_sse_rules() {
        let lines = |s: &str| {
            let mut v = Vec::new();
            for_each_line(s, |l| v.push(l.to_string()));
            v
        };
        assert_eq!(lines(""), [""]);
        assert_eq!(lines("a"), ["a"]);
        assert_eq!(lines("a\n"), ["a", ""]);
        assert_eq!(lines("a\r\nb"), ["a", "b"]);
        assert_eq!(lines("a\rb"), ["a", "b"]);
        assert_eq!(lines("\r\n\r\n"), ["", "", ""]);
        assert_eq!(lines("한\n글"), ["한", "글"]);
    }
}
