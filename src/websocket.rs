use std::net::SocketAddr;

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

/// Parse the HTTP request line and headers from raw bytes.
/// If this is a WebSocket upgrade request, returns `Some((websocket_key, Request<()>))`.
/// Otherwise returns `None`.
fn parse_upgrade_request(header_bytes: &[u8]) -> Option<(String, Request<()>)> {
    let headers_str = std::str::from_utf8(header_bytes).ok()?;
    let mut lines = headers_str.split("\r\n");

    // Parse request line: "GET /path HTTP/1.1"
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("GET");
    let uri = parts.next().unwrap_or("/");
    let version_str = parts.next().unwrap_or("HTTP/1.1");

    let version = match version_str {
        "HTTP/0.9" => http::Version::HTTP_09,
        "HTTP/1.0" => http::Version::HTTP_10,
        "HTTP/1.1" => http::Version::HTTP_11,
        "HTTP/2.0" => http::Version::HTTP_2,
        "HTTP/3.0" => http::Version::HTTP_3,
        _ => http::Version::HTTP_11,
    };

    let mut has_upgrade_connection = false;
    let mut has_upgrade_websocket = false;
    let mut websocket_key = None;
    let mut headers: Vec<(&str, &str)> = Vec::new();

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let colon_pos = match line.find(':') {
            Some(pos) => pos,
            None => continue,
        };
        let key = line[..colon_pos].trim();
        let value = line[colon_pos + 1..].trim();
        headers.push((key, value));

        if key.eq_ignore_ascii_case("connection") && value.to_ascii_lowercase().contains("upgrade")
        {
            has_upgrade_connection = true;
        } else if key.eq_ignore_ascii_case("upgrade") && value.eq_ignore_ascii_case("websocket") {
            has_upgrade_websocket = true;
        } else if key.eq_ignore_ascii_case("sec-websocket-key") {
            websocket_key = Some(value.to_string());
        }
    }

    if !has_upgrade_connection || !has_upgrade_websocket {
        return None;
    }

    let ws_key = websocket_key?;

    // Build Request<()> with all metadata
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .version(version);

    for (k, v) in headers {
        builder = builder.header(k, v);
    }

    let request = builder.body(()).ok()?;
    Some((ws_key, request))
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
    options: &Options,
) -> Result<StreamResult, SendableError> {
    let (bytes, stream) = get_bytes_from_reader(stream, options).await?;

    // Find header boundary (\r\n\r\n)
    let header_end = find_header_end_optimized(&bytes)
        .map(|p| p + 4)
        .unwrap_or(bytes.len());

    let peer = options
        .current_client_addr
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));

    // Check for WebSocket upgrade
    if let Some((client_key, request)) = parse_upgrade_request(&bytes[..header_end]) {
        let ws_stream = perform_upgrade(stream, &client_key).await?;
        Ok(StreamResult::WebSocket(ws_stream, request, peer))
    } else {
        let request = get_request(bytes).await?;
        let (req, res) = get_parse_result_from_request(request, stream, options)?;
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
    options: &Options,
) -> Result<StreamResultArena, SendableError> {
    let (arena_body, stream) = get_bytes_arena_direct(stream, options).await?;

    let peer = options
        .current_client_addr
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));

    // Check headers via ArenaBody
    if let Some((client_key, request)) = parse_upgrade_request(arena_body.get_headers()) {
        drop(arena_body);
        let ws_stream = perform_upgrade(stream, &client_key).await?;
        Ok(StreamResultArena::WebSocket(ws_stream, request, peer))
    } else {
        let request = parse_http_request_arena(arena_body)?;
        let (req, res) = get_parse_result_arena_writer(request, stream, options)?;
        Ok(StreamResultArena::Http(req, res))
    }
}
