# Limitador (library)

[![Crates.io](https://img.shields.io/crates/v/limitador)](https://crates.io/crates/limitador)
[![docs.rs](https://docs.rs/limitador/badge.svg)](https://docs.rs/limitador)

An embeddable rate-limiter library supporting in-memory, Redis and Infinispan data stores.
Limitador can also be compiled to WebAssembly.

For the complete documentation of the crate's API, please refer to [docs.rs](https://docs.rs/limitador/latest/limitador/)

## Features

* `redis_storage`: support for using Redis as the data storage backend.
* `infinispan_storage`: support for using Infinispan as the data storage backend.
* `lenient_conditions`: support for the deprecated syntax of `Condition`s
* `default`: `redis_storage`.

### WebAssembly support

To use Limitador in a project that compiles to WASM, there are some features
that need to be disabled. Add this to your `Cargo.toml` instead:

```toml
[dependencies]
limitador = { version = "0.3.0", default-features = false }
```
