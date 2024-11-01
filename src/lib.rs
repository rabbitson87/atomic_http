use std::{env::current_dir, error::Error, path::PathBuf};

#[cfg(feature = "env")]
use std::str::FromStr;

pub use helpers::traits::http_request::RequestUtils;
pub use helpers::traits::http_response::ResponseUtil;
pub use helpers::traits::http_stream::StreamHttp;

pub mod external {
    pub use async_trait;
    #[cfg(feature = "env")]
    pub use dotenv;
    pub use http;
    #[cfg(feature = "response_file")]
    pub use mime_guess;
    pub use tokio;
}

use http::{Request, Response};

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
    pub listener: TcpListener,
    #[cfg(feature = "tokio_rustls")]
    pub stream: TlsStream<TcpStream>,
    pub options: Options,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub no_delay: bool,
    pub try_read_limit: i32,
    pub try_write_limit: i32,
    pub use_normal_read: bool,
    pub use_send_write_all: bool,
    pub root_path: PathBuf,
}

impl Options {
    pub fn new() -> Options {
        let mut _options = Options {
            no_delay: true,
            try_read_limit: 80,
            try_write_limit: 80,
            use_normal_read: false,
            use_send_write_all: true,
            root_path: current_dir().unwrap(),
        };

        #[cfg(feature = "env")]
        {
            use std::env;
            if let Ok(data) = env::var("NO_DELAY") {
                // true, false
                if let Ok(data) = data.parse::<bool>() {
                    _options.no_delay = data;
                }
            }
            if let Ok(data) = env::var("TRY_READ_LIMIT") {
                if let Ok(data) = data.parse::<i32>() {
                    _options.try_read_limit = data;
                }
            }

            if let Ok(data) = env::var("TRY_WRITE_LIMIT") {
                if let Ok(data) = data.parse::<i32>() {
                    _options.try_write_limit = data;
                }
            }

            if let Ok(data) = env::var("USE_NORMAL_READ") {
                // true, false
                if let Ok(data) = data.parse::<bool>() {
                    _options.use_normal_read = data;
                }
            }

            if let Ok(data) = env::var("USE_SEND_WRITE_ALL") {
                // true, false
                if let Ok(data) = data.parse::<bool>() {
                    _options.use_send_write_all = data;
                }
            }

            if let Ok(data) = env::var("ROOT_PATH") {
                _options.root_path = PathBuf::from_str(&data).unwrap();
            }
        }

        _options
    }
}

impl Server {
    #[cfg(not(feature = "tokio_rustls"))]
    pub async fn new(address: &str) -> Result<Server, Box<dyn Error>> {
        let listener = TcpListener::bind(address).await?;
        Ok(Server {
            listener,
            options: Options::new(),
        })
    }
    #[cfg(feature = "tokio_rustls")]
    pub async fn new(stream: TlsStream<TcpStream>) -> Result<Server, Box<dyn Error>> {
        Ok(Server {
            stream,
            options: Options::new(),
        })
    }
    #[cfg(not(feature = "tokio_rustls"))]
    pub async fn accept(&self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream.parse_request(&self.options).await?)
    }
    #[cfg(feature = "tokio_rustls")]
    pub async fn accept(self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let options = self.options.clone();
        let (stream, _connect) = self.stream.into_inner();
        Ok(stream.parse_request(&options).await?)
    }
    pub fn set_no_delay(&mut self, no_delay: bool) {
        self.options.no_delay = no_delay;
    }
}

pub struct Body {
    pub bytes: Vec<u8>,
    pub body: String,
    pub len: usize,
}

pub struct Writer {
    pub stream: TcpStream,
    pub body: String,
    pub bytes: Vec<u8>,
    pub use_file: bool,
    pub options: Options,
}
