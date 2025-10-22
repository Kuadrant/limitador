# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Limitador is a generic rate-limiter written in Rust that can be used as a library or as a service. It exposes HTTP endpoints and implements the Envoy Rate Limit Service (RLS) v3 protocol via gRPC for integration with Envoy proxy.

## Repository Structure

This is a Cargo workspace with two main crates:

- **`limitador/`** - Core rate-limiting library that can be embedded in other applications
- **`limitador-server/`** - Server binary that exposes HTTP and gRPC (Envoy RLS) interfaces

Key directories:
- `limitador/src/storage/` - Storage backend implementations (in-memory, Redis, disk)
- `limitador/src/limit/` - Limit definition and CEL condition evaluation logic
- `limitador-server/src/envoy_rls/` - Envoy RLS v3 protocol implementation
- `limitador-server/src/http_api/` - HTTP REST API endpoints
- `doc/` - Architecture and configuration documentation

## Development Commands

### Build
```bash
cargo build
```

For release builds (with LTO optimizations):
```bash
cargo build --release
```

### Running Tests

**Important**: Some tests require Redis running on `localhost:6379`. Start Redis first:
```bash
docker run --rm -p 6379:6379 -it redis:7
```

Run all tests with all features:
```bash
cargo test --all-features
```

Run tests for the library without Redis dependency:
```bash
cd limitador && cargo test --no-default-features
```

Run a single test:
```bash
cargo test --all-features <test_name>
```

### Running the Server Locally

```bash
cargo run --release --bin limitador-server -- <LIMITS_FILE> [STORAGE]
```

Example with the sample limits file:
```bash
cargo run --release --bin limitador-server -- ./limitador-server/examples/limits.yaml
```

To validate a limits file without starting the server:
```bash
cargo run --bin limitador-server -- --validate ./limitador-server/examples/limits.yaml
```

### Storage Options

When running the server, you can specify different storage backends:

- `memory` (default) - In-memory, ephemeral counters
- `disk <PATH>` - RocksDB persistent storage
- `redis <URL>` - Redis backend (e.g., `redis://localhost:6379`)
- `redis_cached <URL>` - Redis with in-memory caching layer

### Benchmarks

Run benchmarks:
```bash
cargo bench
```

## Architecture

### Rate Limiting Model

Limitador evaluates limits based on:
1. **Namespace** - Logical grouping (typically the domain from Envoy's RateLimitRequest)
2. **Conditions** - CEL expressions that must all evaluate to `true` for the limit to apply
3. **Variables** - Keys from descriptors used to qualify/partition counters
4. **max_value** - Maximum allowed requests
5. **seconds** - Time window duration

**Evaluation logic**: All matching limits have their counters incremented. If ANY counter exceeds its max_value, the request is rate-limited (most restrictive wins).

### CEL Conditions

Conditions are [CEL (Common Expression Language)](https://cel.dev) expressions operating on:
- `descriptors` - Array of maps from Envoy's RateLimitRequest
- Limit metadata (`id`, `name` fields)

Example: `descriptors[0]['req.method'] == 'POST'`

### Storage Architecture

Storage backends implement a common trait interface. Key implementations:
- **In-memory** (`storage/in_memory.rs`) - Fastest, ephemeral, uses Moka cache with configurable eviction
- **Redis** (`storage/redis/`) - Persistent, atomic counter increments in Redis, supports multiple Limitador instances
- **Redis Cached** - Redis backend with in-memory caching and batched updates for lower latency at cost of some accuracy
- **Disk** - RocksDB-based persistent storage

### Limit Files

Limits are defined in YAML files with this structure:
```yaml
- namespace: example.org
  max_value: 10
  seconds: 60
  conditions:
    - "descriptors[0].req_method == 'GET'"
  variables:
    - descriptors[0].user_id
  name: optional-limit-name  # optional
```

The server monitors the limits file for changes and hot-reloads valid updates.

### Envoy Integration

The server implements Envoy's RateLimitService v3 protocol:
- gRPC service listens on port 8081 by default (configurable via `--rls-port` or `ENVOY_RLS_HOST`/`ENVOY_RLS_PORT`)
- HTTP API listens on port 8080 by default (configurable via `--http-port` or `HTTP_API_HOST`/`HTTP_API_PORT`)
- See `limitador-server/examples/envoy.yaml` for a sample Envoy configuration

## Features and Cargo Configuration

### Library Features (`limitador`)
- `redis_storage` - Enables Redis backend (included in default)
- `disk_storage` - Enables RocksDB backend (included in default)
- `distributed_storage` - Enables distributed storage support

### Server Features (`limitador-server`)
- `distributed_storage` - Enables distributed storage features

## Testing Patterns

- Integration tests are in `limitador/tests/`
- Tests requiring Redis use the `redis-test` crate for ephemeral Redis instances
- Use `serial_test` for tests that cannot run in parallel (e.g., shared Redis state)

## Key Dependencies

- **cel-interpreter/cel-parser** - CEL expression evaluation for conditions
- **moka** - High-performance concurrent cache for in-memory storage
- **tonic/prost** - gRPC/protobuf for Envoy RLS protocol
- **actix-web** - HTTP API framework
- **redis** - Redis client with TLS and connection pooling
- **rocksdb** - Embedded persistent key-value store

## Configuration

Command-line arguments take precedence over environment variables. Key configurations:
- Ports and hosts for HTTP/gRPC servers
- Verbosity levels (`-v`, `-vv`, `-vvv`)
- Rate limit response headers (`--rate-limit-headers DRAFT_VERSION_03`)
- Prometheus label inclusion (`--limit-name-in-labels`)
- OpenTelemetry tracing endpoint (`--tracing-endpoint`)

See `doc/server/configuration.md` for complete environment variable reference.

## Important Notes

- The workspace uses `resolver = "2"` and optimizes release builds with LTO and single codegen unit
- Default features include both `redis_storage` and `disk_storage`
- When running multiple Limitador instances against shared Redis, be aware of race conditions with "stacked limits" (limits over different periods)
- The `redis_cached` storage trades accuracy for performance - understand the caching behavior before use
- CEL expressions reference descriptors as `descriptors[0]` (array index notation)
