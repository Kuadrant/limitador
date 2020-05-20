# Limitador

[![Docker Repository on Quay](https://quay.io/repository/3scale/limitador/status
"Docker Repository on Quay")](https://quay.io/repository/3scale/limitador)

**Status: Experimental**

Limitador is a generic rate-limiter written in Rust. It can be used as a library
or as a service that implements Envoy's Rate Limit protocol.

## Run the grpc server

After cloning the repo:
```bash
git submodule update --init
```

To run Limitador, you need to provide a YAML file with the limits. There's an
example file that allows 10 requests per minute and per user_id when the HTTP
method is "GET" and 5 when it is a "POST":
```bash
LIMITS_FILE=./examples/limits.yaml cargo run --release
```

There's a minimal Envoy config to try limitador in `examples/envoy.yaml`. The
config forwards the "userid" header and the request method to Limitador. It
assumes that there's an upstream API deployed in the port 1323. You can use
[echo](https://github.com/labstack/echo), for example.


## Develop

### Pull the git submodules

After cloning the repo, pull the git submodules with the Envoy grpc dependencies:

```bash
git submodule update --init
```

### Build

```bash
cargo build
```

### Run the tests

Some tests need a redis deployed in `localhost:6379`. You can run it in Docker with:
```bash
docker run --rm --net=host -it redis
```

Then, run the tests:

```bash
cargo test
```

