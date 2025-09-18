use atomic_http::{
    external::dotenv::dotenv, ArenaBody, ArenaWriter, ResponseUtilArena, SendableError, Server,
};
use http::{Request, Response};
use tokio::fs::try_exists;

#[tokio::main]
async fn main() {
    dotenv().ok();
    let address: String = format!("0.0.0.0:{}", 9000);
    let mut server = Server::new(&address).await.unwrap();

    println!("start server on: {}", address);
    loop {
        match server.accept().await {
            Ok(accept) => tokio::spawn(async move {
                let ip = accept.option.get_request_ip();
                println!("ip: {:?}", ip);
                let (request, response) = match accept.parse_request_arena_writer().await {
                    Ok(data) => data,
                    Err(e) => {
                        println!("failed to parse request: {e:?}");
                        return;
                    }
                };
                www_service(request, response).await.unwrap_or_else(|e| {
                    println!("an error occured; error = {:?}", e);
                });
            }),
            Err(e) => {
                println!("failed to accept connection: {e:?}");
                continue;
            }
        };
    }
}

async fn www_service(
    request: Request<ArenaBody>,
    mut response: Response<ArenaWriter>,
) -> Result<(), SendableError> {
    println!("ip: {:?}", request.body().ip);
    println!(
        "request: {:?}\n",
        String::from_utf8_lossy(request.body().get_raw_data())
    );
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

    response.responser_arena().await?;
    Ok(())
}
