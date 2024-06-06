# Changes

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