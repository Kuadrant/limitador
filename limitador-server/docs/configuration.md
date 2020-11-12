# CONFIGURATION

The Limitador server has some options that can be configured with environment
variables:

## ENVOY_RLS_HOST

- Host where the Envoy RLS server listens.
- Optional. Defaults to "0.0.0.0".
- Format: string.


## ENVOY_RLS_PORT

- Port where the Envoy RLS server listens.
- Optional. Defaults to 8081.
- Format: integer.


## HTTP_API_HOST

- Host where the HTTP server listens.
- Optional. Defaults to "0.0.0.0".
- Format: string.


## HTTP_API_PORT

- Port where the HTTP API listens.
- Optional. Defaults to 8080.
- Format: integer.


## LIMITS_FILE

- YAML file that contains the limits to create when limitador boots. If the
limits specified already exist, limitador will not delete them and will keep
their counters. If the limits in the file do not exist, Limitador will create
them. These limits can be modified with the HTTP api.
- Optional. By default, Limitador does not create or delete any limits when it
boots.
- Format: file path.


## REDIS_URL

- Redis URL. Required only when you want to use a Redis to store the limits.
- Optional. By default, Limitador stores the limits in memory and does not
require Redis.
- Format: URL like "redis://127.0.0.1:6379".


## RUST_LOG

- Defines the log level.
- Optional. Defaults to "error".
- Format: "debug", "error", "info", "warn", or "trace".
