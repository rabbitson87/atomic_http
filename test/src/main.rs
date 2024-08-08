use std::error::Error;

use atomic_http::{Body, ResponseUtil, Server, Writer};
use http::{Request, Response};
use tokio::fs::try_exists;

#[tokio::main]
async fn main() {
    let address: String = format!("0.0.0.0:{}", 9000);
    let server = Server::new(&address).await.unwrap();

    println!("start server on: {}", address);
    loop {
        if let Ok((request, response)) = server.accept().await {
            tokio::spawn(async move {
                www_service(request, response).await.unwrap_or_else(|e| {
                    println!("an error occured; error = {:?}", e);
                });
            });
        } else {
            println!("failed to accept connection");
            continue;
        }
    }
}

async fn www_service(
    request: Request<Body>,
    mut response: Response<Writer>,
) -> Result<(), Box<dyn Error>> {
    if request.headers().get("host") != None && request.uri().path() != "/" {
        let path = request.uri().path()[1..].to_owned();

        if path.contains(".") {
            let path: String = urlencoding::decode(&path)?.into();
            let dir = std::env::current_dir()?;
            let path = dir.join(&path);

            let exist = try_exists(&path).await?;

            if exist {
                response.body_mut().response_file(path)?;
            } else {
                let dir = std::env::current_dir()?;
                let path = dir.join("app/index.html");
                response.body_mut().response_file(path)?;
            }
        } else {
            let dir = std::env::current_dir()?;
            let path = dir.join("app/index.html");
            response.body_mut().response_file(path)?;
        }
    }

    response.responser().await?;
    Ok(())
}
