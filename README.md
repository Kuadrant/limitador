# Limitador

![Github Workflow](https://github.com/3scale/limitador/workflows/Rust/badge.svg)
[![docs.rs](https://docs.rs/limitador/badge.svg)](https://docs.rs/limitador)
[![Crates.io](https://img.shields.io/crates/v/limitador)](https://crates.io/crates/limitador)
[![Docker Repository on Quay](https://quay.io/repository/3scale/limitador/status
"Docker Repository on Quay")](https://quay.io/repository/3scale/limitador)

Limitador is a generic rate-limiter written in Rust. It can be used as a
library, as an HTTP service, or as a GRPC service that implements the Envoy Rate
Limit protocol.

- [**Getting started**](#getting-started)
- [**Limits storage**](#limits-storage)
- [**Development**](#development)
- [**License**](#license)

**Status: Experimental**

## Getting started

- [Rust library](#rust-library)
- [HTTP server](#http-service)
- [GRPC server for Envoy](#grpc-server-that-implements-envoys-rls)

### Rust library

Add this to your `Cargo.toml`:
```toml
[dependencies]
limitador = { version = "0.1.2" }
```

To use limitador in a project that compiles to WASM, there are some features
that need to be disabled. Add this to your `Cargo.toml` instead:
```toml
[dependencies]
limitador = { version = "0.1.2", default-features = false }
```

### HTTP service

The OpenAPI spec of the service is
[here](limitador-http-server/docs/http_server_spec.json).

Run with Docker (replace `latest` with the version you want):
```bash
docker run --rm --net=host -it quay.io/3scale/limitador:latest http-server
```

To use Redis, specify the URL with `REDIS_URL`:
```bash
docker run -e REDIS_URL=redis://127.0.0.1:6379 --rm --net=host -it quay.io/3scale/limitador:latest http-server
```

You can also run the service locally:
```bash
cargo run --release --bin limitador-http-server
```

You can change the host and port with the `HOST` and `PORT` envs.

### GRPC server that implements Envoy's RLS

To run Limitador, you need to provide a YAML file with the limits. There's an
[example file](limitador-envoy-rls/examples/limits.yaml) that allows 10 requests
per minute and per user_id when the HTTP method is "GET" and 5 when it is a
"POST". You can run it with Docker (replace `latest` with the version you want):
```bash
docker run -e LIMITS_FILE=/home/limitador/my_limits.yaml --rm --net=host -it -v $(pwd)/limitador-envoy-rls/examples/limits.yaml:/home/limitador/my_limits.yaml:ro quay.io/3scale/limitador:latest envoy-rls
```

To use Redis, specify the URL with `REDIS_URL`:
```bash
docker run -e LIMITS_FILE=/home/limitador/my_limits.yaml -e REDIS_URL=redis://127.0.0.1:6379 --rm --net=host -it -v $(pwd)/limitador-envoy-rls/examples/limits.yaml:/home/limitador/my_limits.yaml:ro quay.io/3scale/limitador:latest envoy-rls
```

You can also run the service locally:
```bash
LIMITS_FILE=./limitador-envoy-rls/examples/limits.yaml cargo run --release --bin limitador-envoy-rls
```

There's a minimal Envoy config to try limitador
[here](limitador-envoy-rls/examples/envoy.yaml). The config forwards the
"userid" header and the request method to Limitador. It assumes that there's an
upstream API deployed in the port 1323. You can use
[echo](https://github.com/labstack/echo), for example.

You can change the host and port with the `HOST` and `PORT` envs.

## Limits storage

Limitador can store its limits and counters in memory or in Redis. In memory is
faster, but the limits are applied per instance. When using Redis, multiple
instances of limitador can share the same limits, but it's slower.


## Development

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

or you can run tests disabling the "redis storage" feature:
```bash
cd limitador; cargo test --no-default-features
```

## License

[Apache 2.0 License](LICENSE)
