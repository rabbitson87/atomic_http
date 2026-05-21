use std::net::SocketAddr;
use std::sync::Arc;

use http::{Request, Response};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::WebSocketStream;

use crate::helpers::traits::http_stream::{
    find_header_end_optimized, get_bytes_from_reader, get_parse_result_from_request, get_request,
};
use crate::{Body, Options, SendableError, Writer};

#[cfg(feature = "arena")]
use crate::helpers::traits::http_stream::{
    get_bytes_arena_direct, get_parse_result_arena_writer, parse_http_request_arena,
};
#[cfg(feature = "arena")]
use crate::{ArenaBody, ArenaWriter};

/// Result of a WebSocket upgrade attempt.
pub enum StreamResult {
    /// Regular HTTP request — parsed into standard Request/Response.
    Http(Request<Body>, Response<Writer>),
    /// WebSocket upgrade completed — stream is ready for WebSocket frames.
    /// The `Request<()>` contains the original upgrade request metadata
    /// (URI, method, headers) for routing and authentication.
    WebSocket(WebSocketStream<TcpStream>, Request<()>, SocketAddr),
}

/// Result of a WebSocket upgrade attempt (arena variant).
#[cfg(feature = "arena")]
pub enum StreamResultArena {
    /// Regular HTTP request — parsed into arena-based Request/Response.
    Http(Request<ArenaBody>, Response<ArenaWriter>),
    /// WebSocket upgrade completed — stream is ready for WebSocket frames.
    /// The `Request<()>` contains the original upgrade request metadata
    /// (URI, method, headers) for routing and authentication.
    WebSocket(WebSocketStream<TcpStream>, Request<()>, SocketAddr),
}

/// Parse the HTTP request line and headers from raw bytes via `httparse`.
/// If this is a WebSocket upgrade request, returns `Some((websocket_key, Request<()>))`.
/// Otherwise returns `None`.
fn parse_upgrade_request(header_bytes: &[u8]) -> Option<(String, Request<()>)> {
    const MAX_HEADERS: usize = 64;
    let mut headers_buf = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers_buf);
    // Partial이어도 best-effort 진행 (이미 \r\n\r\n까지 잘려 들어옴)
    let _ = req.parse(header_bytes).ok()?;

    let method = req.method.unwrap_or("GET");
    let uri = req.path.unwrap_or("/");
    let version = match req.version {
        Some(0) => http::Version::HTTP_10,
        Some(1) => http::Version::HTTP_11,
        _ => http::Version::HTTP_11,
    };

    let mut has_upgrade_connection = false;
    let mut has_upgrade_websocket = false;
    let mut websocket_key: Option<&[u8]> = None;

    // 한 번의 스캔으로 업그레이드 시그널 검증 + 키 캡처
    for h in req.headers.iter() {
        if h.name.is_empty() {
            break;
        }
        if h.name.eq_ignore_ascii_case("connection") {
            // value bytes가 "upgrade"를 case-insensitive로 포함하는지 검사 (할당 없음)
            if contains_ascii_ci(h.value, b"upgrade") {
                has_upgrade_connection = true;
            }
        } else if h.name.eq_ignore_ascii_case("upgrade") {
            if h.value.eq_ignore_ascii_case(b"websocket") {
                has_upgrade_websocket = true;
            }
        } else if h.name.eq_ignore_ascii_case("sec-websocket-key") {
            websocket_key = Some(h.value);
        }
    }

    if !has_upgrade_connection || !has_upgrade_websocket {
        return None;
    }
    let ws_key_bytes = websocket_key?;
    let ws_key = std::str::from_utf8(ws_key_bytes).ok()?.to_string();

    // Build Request<()> with all metadata
    let mut builder = Request::builder().method(method).uri(uri).version(version);

    for h in req.headers.iter() {
        if h.name.is_empty() {
            break;
        }
        builder = builder.header(h.name, h.value);
    }

    let request = builder.body(()).ok()?;
    Some((ws_key, request))
}

/// `haystack`에 `needle`이 ASCII case-insensitive로 포함되어 있는지 검사 (할당 없음).
fn contains_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    let nlen = needle.len();
    for i in 0..=haystack.len() - nlen {
        let mut matched = true;
        for j in 0..nlen {
            if !haystack[i + j].eq_ignore_ascii_case(&needle[j]) {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

/// Compute the `Sec-WebSocket-Accept` value per RFC 6455 Section 4.2.2.
fn compute_accept_key(client_key: &str) -> String {
    tokio_tungstenite::tungstenite::handshake::derive_accept_key(client_key.as_bytes())
}

/// Send the 101 Switching Protocols response and create a `WebSocketStream`.
async fn perform_upgrade(
    mut stream: TcpStream,
    client_key: &str,
) -> Result<WebSocketStream<TcpStream>, SendableError> {
    let accept_key = compute_accept_key(client_key);

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept_key}\r\n\
         \r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;

    Ok(WebSocketStream::from_raw_socket(stream, Role::Server, None).await)
}

/// Attempt a WebSocket upgrade on the given stream.
///
/// Reads headers from the stream. If the request is a WebSocket upgrade,
/// sends the 101 Switching Protocols response and returns a `WebSocketStream`
/// along with the original `Request<()>` for routing.
/// Otherwise, parses the request normally and returns `Http`.
pub(crate) async fn try_upgrade(
    stream: TcpStream,
    options: Arc<Options>,
    peer: SocketAddr,
) -> Result<StreamResult, SendableError> {
    let (bytes, stream) = get_bytes_from_reader(stream, &options).await?;

    // Find header boundary (\r\n\r\n)
    let header_end = find_header_end_optimized(&bytes)
        .map(|p| p + 4)
        .unwrap_or(bytes.len());

    // Check for WebSocket upgrade
    if let Some((client_key, request)) = parse_upgrade_request(&bytes[..header_end]) {
        let ws_stream = perform_upgrade(stream, &client_key).await?;
        Ok(StreamResult::WebSocket(ws_stream, request, peer))
    } else {
        let request = get_request(bytes).await?;
        let (req, res) = get_parse_result_from_request(request, stream, options, peer)?;
        Ok(StreamResult::Http(req, res))
    }
}

/// Attempt a WebSocket upgrade on the given stream (arena variant).
///
/// Same as `try_upgrade`, but the HTTP fallback path uses arena-allocated
/// `ArenaBody` / `ArenaWriter` for zero-copy request parsing.
#[cfg(feature = "arena")]
pub(crate) async fn try_upgrade_arena(
    stream: TcpStream,
    options: Arc<Options>,
    peer: SocketAddr,
) -> Result<StreamResultArena, SendableError> {
    let (arena_body, stream) = get_bytes_arena_direct(stream, &options).await?;

    // Check headers via ArenaBody
    if let Some((client_key, request)) = parse_upgrade_request(arena_body.get_headers()) {
        drop(arena_body);
        let ws_stream = perform_upgrade(stream, &client_key).await?;
        Ok(StreamResultArena::WebSocket(ws_stream, request, peer))
    } else {
        let request = parse_http_request_arena(arena_body)?;
        let (req, res) = get_parse_result_arena_writer(request, stream, options, peer)?;
        Ok(StreamResultArena::Http(req, res))
    }
}
