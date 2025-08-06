# Changes

## 0.6.1

* Add zero_copy module and zero_copy module to be default.

## 0.6.0

* Add arena mode and arena mode is default.

## 0.5.4

* Add get request ip with option.

## 0.5.3

* Add for request body with ipAddr.

## 0.5.2

* Fixed for read stream when first read.

## 0.5.1

* Remove unused several options.
  try_read_limit: u16;
  try_write_limit: u16;
  use_normal_read: bool;
  use_send_write_all: bool;

## 0.5.0

* Fixed for server accept split process because of thread.

## 0.4.22

* Change option for read stream's binary imcompleted.

## 0.4.21

* Add option for read stream when binary imcompleted.

## 0.4.20

* Recovery for read_stream when use_normal_read is false.

## 0.4.19

* Add option for tcpStream read_retry.

## 0.4.18

* Add option for tcpStream buffer size.

## 0.4.17

* Add delay for connecting tcpstream with unkown error.

## 0.4.16

* Add option for normal_read_timeout_miliseconds.

## 0.4.15

* Change for debug_print with (min, max) read count, default options.

## 0.4.14

* Fixed for unwrap function.

## 0.4.13

* Change method for request parse when content-length showing.

## 0.4.12

* Fixed for request parse when content-length showing.

## 0.4.11

* Add re export external list.

```
pub mod external {
    pub use async_trait;
    #[cfg(feature = "env")]
    pub use dotenv;
    pub use http;
    #[cfg(feature = "response_file")]
    pub use mime_guess;
    pub use tokio;
}
```

## 0.4.10

* Add for server to auto inject from .env file simply.

.env file example
```
NO_DELAY=true
TRY_READ_LIMIT=20
TRY_WRITE_LIMIT=20
USE_NORMAL_READ=false
USE_SEND_WRITE_ALL=false
ROOT_PATH=D:\\git\\atomic_http\\test
```

## 0.4.9

* Change for request to get_json simply.

## 0.4.8

* Add options for response file root path.

## 0.4.7

* Add options for controll request and response.

## 0.4.6

* Fixed for improve response performance in multithread.

## 0.4.5

* Change for improve request read performance in multithread.

## 0.4.4

* Fixed for accept try limit 1 to 200.

## 0.4.3

* Fixed for convert string from header with utf8_lossy. 

## 0.4.2

* Fixed for split with header and body. 

## 0.4.1

* Fixed for duplicated read request data.

## 0.4.0

* Fixed for accept request read, add debug feature(if you need log print!).

## 0.3.13

* Add set nodelay for tcpstream.

## 0.3.12

* Fixed for read request when read error.

## 0.3.11

* Add for zip response for response_file feature.

## 0.3.10

* Fixed for only parse the header format when utf-8.

## 0.3.9

* Fixed buffer size(1024 -> 4096) and wait for readable stream.

## 0.3.8

* Fixed status for response_file between plain.

## 0.3.7

* Fixed status for response_file.

## 0.3.6

* Add print request bytes len when parse header.

## 0.3.5

* Fixed none header with request when parse header.

## 0.3.4

* Fixed parse header.

## 0.3.3

* Fixed response_file for remove header.

## 0.3.2

* Fixed response_file for content-type, response status.

## 0.3.1

* Fixed response_file for content-type with mime_guess.

## 0.3.0

* Add response_file for response.response_file(features="response_file"), remove zip response.

## 0.2.0

* Add tokio_rustls for parse_request(features="tokio_rustls"), remove static_str.

## 0.1.7

* Update dependencies.
  tokio: 1.36.0 -> 1.38.0
  async-trait: 0.1.79 -> 0.1.80
  serde_json: 1.0.115 -> 1.0.117
  serde 1.0.197 -> 1.0.203

## 0.1.6

* Fixed blocking empty byte when split bytes.

## 0.1.5

* Add bytes response(don't use body).

## 0.1.4

* Change server address type(String -> &str).

## 0.1.3

* Add pub trait for request.

## 0.1.2

* Add pub trait for response.

## 0.1.1

* Add pub struct for request and response.

## 0.1.0

* First init simple server.