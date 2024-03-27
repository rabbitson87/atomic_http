use std::error::Error;

pub use helpers::traits::http_response::ResponseUtil;
pub use helpers::traits::http_stream::StreamHttp;
pub use http::{Request, Response};
use tokio::{
    io::WriteHalf,
    net::{TcpListener, TcpStream},
};

mod helpers;
pub struct Server {
    listener: TcpListener,
}

impl Server {
    pub async fn new(address: String) -> Result<Server, Box<dyn Error>> {
        let listener = TcpListener::bind(&address).await?;
        Ok(Server { listener })
    }
    pub async fn accept(&self) -> Result<(Request<Body>, Response<Writer>), Box<dyn Error>> {
        let (stream, _) = self.listener.accept().await?;
        Ok(stream.parse_request().await?)
    }
}

pub struct Body {
    pub body: Vec<u8>,
    pub len: usize,
}
pub struct Writer {
    pub writer: WriteHalf<TcpStream>,
    pub body: String,
}
