use std::error::Error;

pub use helpers::traits::http_request::RequestUtils;
pub use helpers::traits::http_response::ResponseUtil;
pub use helpers::traits::http_stream::StreamHttp;
pub use http::{Request, Response};

#[cfg(not(feature = "tokio_rustls"))]
use tokio::net::TcpListener;

use tokio::{io::WriteHalf, net::TcpStream};
#[cfg(feature = "tokio_rustls")]
use tokio_rustls::server::TlsStream;

mod helpers;
pub struct Server {
    #[cfg(not(feature = "tokio_rustls"))]
    listener: TcpListener,
}

impl Server {
    #[cfg(not(feature = "tokio_rustls"))]
    pub async fn new(address: &str) -> Result<Server, Box<dyn Error>> {
        let listener = TcpListener::bind(address).await?;
        Ok(Server { listener })
    }
    #[cfg(not(feature = "tokio_rustls"))]
    pub async fn accept(&self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream.parse_request().await?)
    }
    #[cfg(feature = "tokio_rustls")]
    pub async fn parse_from_tls(
        stream: TlsStream<TcpStream>,
    ) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        Ok(stream.parse_request().await?)
    }
}

pub struct Body {
    pub body: Vec<u8>,
    pub len: usize,
}
pub struct Writer {
    #[cfg(feature = "tokio_rustls")]
    pub writer: WriteHalf<TlsStream<TcpStream>>,
    #[cfg(not(feature = "tokio_rustls"))]
    pub writer: WriteHalf<TcpStream>,
    pub body: String,
    pub bytes: Vec<u8>,
}
