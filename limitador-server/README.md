# Limitador (server)

[![Docker Repository on Quay](https://quay.io/repository/kuadrant/limitador/status
"Docker Repository on Quay")](https://quay.io/repository/kuadrant/limitador)

By default, Limitador starts the HTTP server in `localhost:8080` and the grpc
service that implements the Envoy Rate Limit protocol in `localhost:8081`. That
can be configured with these ENVs: `ENVOY_RLS_HOST`, `ENVOY_RLS_PORT`,
`HTTP_API_HOST`, and `HTTP_API_PORT`.

Or using the command line arguments:

```commandline
Limitador Server
The Kuadrant team - github.com/Kuadrant
Rate Limiting Server

USAGE:
    limitador-server [OPTIONS] <LIMITS_FILE> [STORAGE]

ARGS:
    <LIMITS_FILE>    The limit file to use

OPTIONS:
    -b, --rls-ip <ip>              The IP to listen on for RLS [default: 0.0.0.0]
    -p, --rls-port <port>          The port to listen on for RLS [default: 8081]
    -B, --http-ip <http_ip>        The IP to listen on for HTTP [default: 0.0.0.0]
    -P, --http-port <http_port>    The port to listen on for HTTP [default: 8080]
    -l, --limit-name-in-labels     Include the Limit Name in prometheus label
    -v                             Sets the level of verbosity
        --validate                 Validates the LIMITS_FILE and exits
    -h, --help                     Print help information
    -V, --version                  Print version information

STORAGES:
    memory          Counters are held in Limitador (ephemeral)
    redis           Uses Redis to store counters
    redis_cached    Uses Redis to store counters, with an in-memory cache
```

When using environment variables, these will override the defaults. While environment variable are themselves
overridden by the command line arguments provided. See the individual `STORAGES` help for more options relative to
each of the storages.

The OpenAPI spec of the HTTP service is
[here](docs/http_server_spec.json).

Limitador has to be started with a YAML file that has some limits defined. There's an [example
file](examples/limits.yaml) that allows 10 requests per minute
and per `user_id` when the HTTP method is `"GET"` and 5 when it is a `"POST"`. You can
run it with Docker (replace `latest` with the version you want):
```bash
docker run --rm --net=host -it -v $(pwd)/examples/limits.yaml:/home/limitador/my_limits.yaml:ro quay.io/kuadrant/limitador:latest limitador-server /home/limitador/my_limits.yaml
```

You can also use the YAML file when running locally:
```bash
cargo run --release --bin limitador-server ./examples/limits.yaml
```

If you want to use Limitador with Envoy, there's a minimal Envoy config for
testing purposes [here](examples/envoy.yaml). The config
forwards the "userid" header and the request method to Limitador. It assumes
that there's an upstream API deployed on port 1323. You can use
[echo](https://github.com/labstack/echo), for example.

Limitador has several options that can be configured via ENV. This
[doc](../doc/server/configuration.md) specifies them.

## Limits storage

Limitador can store its limits and counters in-memory or in Redis. In-memory is
faster, but the limits are applied per instance. When using Redis, multiple
instances of Limitador can share the same limits, but it's slower.
