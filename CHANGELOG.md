# Changes

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