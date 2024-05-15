# Limitador (library)

[![Crates.io](https://img.shields.io/crates/v/limitador)](https://crates.io/crates/limitador)
[![docs.rs](https://docs.rs/limitador/badge.svg)](https://docs.rs/limitador)

An embeddable rate-limiter library supporting in-memory, Redis and disk data stores.

For the complete documentation of the crate's API, please refer to [docs.rs](https://docs.rs/limitador/latest/limitador/)

## Features

* `redis_storage`: support for using Redis as the data storage backend.
* `disk_storage`: support for using RocksDB as a local disk storage backend.
* `lenient_conditions`: support for the deprecated syntax of `Condition`s
* `default`: `redis_storage`.
