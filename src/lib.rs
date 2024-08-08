use std::error::Error;

pub use helpers::traits::http_request::RequestUtils;
pub use helpers::traits::http_response::ResponseUtil;
pub use helpers::traits::http_stream::StreamHttp;
pub use http::{Request, Response};

#[macro_export]
macro_rules! dev_print {
    ($($rest:tt)*) => {
        if (cfg!(feature = "debug")) {
            std::println!($($rest)*)
        }
    };
}

#[cfg(not(feature = "tokio_rustls"))]
use tokio::net::TcpListener;

use tokio::net::TcpStream;
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
    pub stream: TcpStream,
    pub body: String,
    pub bytes: Vec<u8>,
    pub use_file: bool,
}

#[cfg(feature = "response_file")]
use std::path::Path;

impl Writer {
    #[cfg(feature = "response_file")]
    pub fn response_file<P>(&mut self, path: P) -> Result<(), Box<dyn Error>>
    where
        P: AsRef<Path>,
    {
        use std::env;
        let current_dir = env::current_dir()?;
        let path = current_dir.join(path);
        self.body = path.to_str().unwrap().to_string();
        self.use_file = true;
        Ok(())
    }
}
