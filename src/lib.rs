use std::error::Error;

use helpers::traits::http_stream::{Body, StreamHttp, Writer};
use http::{Request, Response};
use tokio::net::TcpListener;

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
