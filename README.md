# Limitador

[![Limitador GH Workflow](https://github.com/Kuadrant/limitador/actions/workflows/rust.yml/badge.svg)](https://github.com/Kuadrant/limitador/actions/workflows/rust.yml)
[![docs.rs](https://docs.rs/limitador/badge.svg)](https://docs.rs/limitador)
[![Crates.io](https://img.shields.io/crates/v/limitador)](https://crates.io/crates/limitador)
[![Docker Repository on Quay](https://quay.io/repository/kuadrant/limitador/status
"Docker Repository on Quay")](https://quay.io/repository/kuadrant/limitador)
[![codecov](https://codecov.io/gh/Kuadrant/limitador/branch/main/graph/badge.svg?token=CE9LD3XCJT)](https://codecov.io/gh/Kuadrant/limitador)

Limitador is a generic rate-limiter written in Rust. It can be used as a
library, or as a service. The service exposes HTTP endpoints to apply and observe
limits. Limitador can be used with Envoy because it also exposes a grpc service, on a different
port, that implements the Envoy Rate Limit protocol (v3).

- [**Getting started**](#getting-started)
- [**How it works**](/doc/how-it-works.md)
- [**Development**](#development)
- [**Testing Environment**](limitador-server/docs/sandbox.md)
- [**Kubernetes**](limitador-server/kubernetes/)
- [**License**](#license)

Limitador is under active development, and its API has not been stabilized yet.

## Getting started

- [Rust library](#rust-library)
- [Server](#server)

### Rust library

Add this to your `Cargo.toml`:
```toml
[dependencies]
limitador = { version = "0.3.0" }
```

For more information, see the [`README` of the crate](limitador/README.md)

### Server

Run with Docker (replace `latest` with the version you want):
```bash
docker run --rm --net=host -it quay.io/kuadrant/limitador:v1.0.0
```

Run locally:
```bash
cargo run --release --bin limitador-server -- --help
```

Refer to the help message on how to start up the server. More information are available
in the [server's `README.md`](limitador-server/README.md)

## Development

### Build

```bash
cargo build
```

### Run the tests

Some tests need a redis deployed in `localhost:6379`. You can run it in Docker with:
```bash
docker run --rm -p 6379:6379 -it redis
```

Some tests need a infinispan deployed in `localhost:11222`. You can run it in Docker with:
```bash
docker run --rm -p 11222:11222 -it -e USER=username -e PASS=password infinispan/server:11.0.9.Final
```

Then, run the tests:

```bash
cargo test --all-features
```

or you can run tests disabling the "redis storage" feature:
```bash
cd limitador; cargo test --no-default-features
```

## License

[Apache 2.0 License](LICENSE)
