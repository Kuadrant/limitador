# Limitador configuration

## Command line configuration

The preferred way of starting and configuring the Limitador server is using the command line:

```
Rate Limiting Server

Usage: limitador-server [OPTIONS] <LIMITS_FILE> [STORAGE]

STORAGES:
  memory        Counters are held in Limitador (ephemeral)
  disk          Counters are held on disk (persistent)
  redis         Uses Redis to store counters
  redis_cached  Uses Redis to store counters, with an in-memory cache

Arguments:
  <LIMITS_FILE>  The limit file to use

Options:
  -b, --rls-ip <ip>
          The IP to listen on for RLS [default: 0.0.0.0]
  -p, --rls-port <port>
          The port to listen on for RLS [default: 8081]
  -B, --http-ip <http_ip>
          The IP to listen on for HTTP [default: 0.0.0.0]
  -P, --http-port <http_port>
          The port to listen on for HTTP [default: 8080]
  -l, --limit-name-in-labels
          Include the Limit Name in prometheus label
  -v...
          Sets the level of verbosity
      --validate
          Validates the LIMITS_FILE and exits
  -H, --rate-limit-headers <rate_limit_headers>
          Enables rate limit response headers [default: NONE] [possible values: NONE, DRAFT_VERSION_03]
      --grpc-reflection-service
          Enables gRPC server reflection service
  -h, --help
          Print help
  -V, --version
          Print version
```

The values used are authoritative over any [environment variables](#configuration-using-environment-variables) independently set.

### Limit definitions

The `LIMITS_FILE` provided is the source of truth for all the limits that will be enforced. The file location will be
monitored by the server for any changes and be hot reloaded. If the changes are invalid, they will be ignored on hot
reload, or the server will fail to start.

#### The `LIMITS_FILE`'s format

When starting the server, you point it to a `LIMITS_FILE`, which is expected to be a _yaml_ file with an array of
`limit` definitions, with the following format:

```yaml
---
"$schema": http://json-schema.org/draft-04/schema#
type: object
properties:
  name:
    type: string
  namespace:
    type: string
  seconds:
    type: integer
  max_value:
    type: integer
  conditions:
    type: array
    items:
      - type: string
  variables:
    type: array
    items:
      - type: string
required:
  - namespace
  - seconds
  - max_value
  - conditions
  - variables
```

Here is an example of such a limit definition:

```yaml
namespace: example.org
max_value: 10
seconds: 60
conditions:
  - "req.method == 'GET'"
variables:
  - user_id
```

 - `namespace` namespaces the limit, will generally be the domain, [see here](../how-it-works.md)
 - `seconds` is the duration for which the limit applies, in seconds: e.g. `60` is a span of time of one minute
 - `max_value` is the actual limit, e.g. `100` would limit to 100 requests
 - `name` lets the user _optionally_ name the limit
 - `variables` is an array of variables, which once resolved, will be used to qualify counters for the limit,
   e.g. `api_key` to limit per api keys
 - `conditions` is an array of conditions, which once evaluated will decide whether to apply the limit or not

#### `condition` syntax

Each `condition` is an expression producing a boolean value (`true` or `false`). All `conditions` _must_ evaluate to
`true` for the `limit` to be applied on a request.

Expressions follow the following syntax: `$IDENTIFIER $OP $STRING_LITERAL`, where:

 - `$IDENTIFIER` will be used to resolve the value at evaluation time, e.g. `role`
 - `$OP` is an operator, either `==` or `!=`
 - `$STRING_LITERAL` is a literal string value, `"` or `'` demarcated, e.g. `"admin"`

So that `role != "admin"` would apply the limit on request from all users, but `admin`'s.

### Counter storages

Limitador will load all the `limit` definitions from the `LIMITS_FILE` and keep these in memory. To enforce these
limits, Limitador needs to track requests in the form of counters. There would be at least one counter per limit, but
that number grows when `variables` are used to qualify counters per some arbitrary values.

#### `memory`

As the name implies, Limitador will keep all counters in memory. This yields the best results in terms of latency as
well as accuracy. By default, only up to `1000` "concurrent" counters will be kept around, evicting the oldest entries.
"Concurrent" in this context means counters that need to exist at the "same time", based of the period of the limit,
as "expired" counters are discarded.

This storage is ephemeral, as if the process is restarted, all the counters are lost and effectively "reset" all the
limits as if no traffic had been rate limited, which can be fine for short-lived limits, less for longer-lived ones.

#### `redis`

When you want persistence of your counters, such as for disaster recovery or across restarts, using `redis` will store
the counters in a redis instance using the provided `URL`. Increments to _individual_ counters is made within redis
itself, providing accuracy over these, races tho can occur when multiple Limitador servers are used against a single
redis and using "stacked" limits (i.e. over different periods). Latency is also impacted, as it results in one
additional hop to talk to redis and maintain the counters.

**TLS Support**

Connect to a redis instance using the `rediss://` URL scheme.

To enable insecure mode, append `#insecure` at the end of the URL. For example:

```
limitador-server <LIMITS_FILE> redis rediss://127.0.0.1/#insecure"
```

**Authentication**

To enable authentication, use the username and password properties of the URL scheme. For example:

```
limitador-server <LIMITS_FILE> redis redis://my-username:my-password@127.0.0.1"
```

when the username is omitted, redis assumes `default` user. For example:

```
limitador-server <LIMITS_FILE> redis redis://:my-password@127.0.0.1"
```

**Usage**

```
Uses Redis to store counters

Usage: limitador-server <LIMITS_FILE> redis <URL>

Arguments:
  <URL>  Redis URL to use

Options:
  -h, --help  Print help
```

#### `redis_cached`

In order to avoid some communication overhead to redis, `redis_cached` adds an in memory caching layer within the
Limitador servers. This lowers the latency, but sacrifices some accuracy as it will not only cache counters, but also
coalesce counters updates to redis over time. See [this configuration](#redis_local_cache_enabled) option for more
information.

**TLS Support**

Connect to a redis instance using the `rediss://` URL scheme.

To enable insecure mode, append `#insecure` at the end of the URL. For example:

```
limitador-server <LIMITS_FILE> redis rediss://127.0.0.1/#insecure"
```

**Authentication**

To enable authentication, use the username and password properties of the URL scheme. For example:

```
limitador-server <LIMITS_FILE> redis redis://my-username:my-password@127.0.0.1"
```

when the username is omitted, redis assumes `default` user. For example:

```
limitador-server <LIMITS_FILE> redis redis://:my-password@127.0.0.1"
```

**Usage**

```
Uses Redis to store counters, with an in-memory cache

Usage: limitador-server <LIMITS_FILE> redis_cached [OPTIONS] <URL>

Arguments:
  <URL>  Redis URL to use

Options:
      --ttl <TTL>             TTL for cached counters in milliseconds [default: 5000]
      --ratio <ratio>         Ratio to apply to the TTL from Redis on cached counters [default: 10000]
      --flush-period <flush>  Flushing period for counters in milliseconds [default: 1000]
      --max-cached <max>      Maximum amount of counters cached [default: 10000]
  -h, --help                  Print help
```

#### `disk`

Disk storage using [RocksDB](https://rocksdb.org/). Counters are held on disk (persistent).

```
Counters are held on disk (persistent)

Usage: limitador-server <LIMITS_FILE> disk [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to counter DB

Options:
      --optimize <OPTIMIZE>  Optimizes either to save disk space or higher throughput [default: throughput] [possible values: throughput, disk]
  -h, --help                 Print help
```

#### `infinispan` optional storage - _experimental_

The default binary will _not_ support [Infinispan](https://infinispan.org/) as a storage backend for counters. If you
want to give it a try, you would need to build your own binary of the server using:

```commandline
cargo build --release --features=infinispan
```

Which will add the `infinispan` to the supported `STORAGES`.

```
USAGE:
    limitador-server <LIMITS_FILE> infinispan [OPTIONS] <URL>

ARGS:
    <URL>    Infinispan URL to use

OPTIONS:
    -n, --cache-name <cache name>      Name of the cache to store counters in [default: limitador]
    -c, --consistency <consistency>    The consistency to use to read from the cache [default:
                                       Strong] [possible values: Strong, Weak]
    -h, --help                         Print help information
```

For an in-depth coverage of the different topologies supported and how they affect the behavior, see the
[topologies' document](../topologies.md).

## Configuration using environment variables

The Limitador server has some options that can be configured with environment variables. These will override the
_default_ values the server uses. [Any argument](#command-line-configuration) used when starting the server will prevail over the
environment variables.

#### `ENVOY_RLS_HOST`

- Host where the Envoy RLS server listens.
- Optional. Defaults to `"0.0.0.0"`.
- Format: `string`.


#### `ENVOY_RLS_PORT`

- Port where the Envoy RLS server listens.
- Optional. Defaults to `8081`.
- Format: `integer`.


#### `HTTP_API_HOST`

- Host where the HTTP server listens.
- Optional. Defaults to `"0.0.0.0"`.
- Format: `string`.


#### `HTTP_API_PORT`

- Port where the HTTP API listens.
- Optional. Defaults to `8080`.
- Format: `integer`.


#### `LIMITS_FILE`

- YAML file that contains the limits to create when Limitador boots. If the
limits specified already have counters associated, Limitador will not delete them.
Changes to the file will be picked up by the running server.
- *Required*. No default
- Format: `string`, file path.


#### `LIMIT_NAME_IN_PROMETHEUS_LABELS`

- Enables using limit names as labels in Prometheus metrics. This is disabled by
default because for a few limits it should be fine, but it could become a
problem when defining lots of limits. See the caution note in the [Prometheus
docs](https://prometheus.io/docs/practices/naming/#labels)
- Optional. Disabled by default.
- Format: `bool`, set to `"1"` to enable.

#### `REDIS_LOCAL_CACHE_ENABLED`

- Enables a storage implementation that uses Redis, but also caches some data in
memory. The idea is to improve throughput and latencies by caching the counters
in memory to reduce the number of accesses to Redis. To achieve that, this mode
sacrifices some rate-limit accuracy. This mode does two things:
    - Batches counter updates. Instead of updating the counters on every
    request, it updates them in memory and commits them to Redis in batches. The
    flushing interval can be configured with the
    [`REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS`](#redis_local_cache_flushing_period_ms)
    env. The trade-off is that when running several instances of Limitador,
    other instances will not become aware of the counter updates until they're
    committed to Redis.
    - Caches counters. Instead of fetching the value of a counter every time
    it's needed, the value is cached for a configurable period. The trade-off is
    that when running several instances of Limitador, an instance will not
    become aware of the counter updates other instances do while the value is
    cached. When a counter is already at 0 (limit exceeded), it's cached until
    it expires in Redis. In this case, no matter what other instances do, we
    know that the quota will not be reestablished until the key expires in
    Redis, so in this case, rate-limit accuracy is not affected. When a counter
    has still some quota remaining the situation is different, that's why we can
    tune for how long it will be cached. The formula is as follows:
    MIN(ttl_in_redis/[`REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS`](#redis_local_cache_ttl_ratio_cached_counters),
    [`REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS`](#redis_local_cache_max_ttl_cached_counters_ms)).
    For example, let's image that the current TTL (time remaining until the
    limit resets) in Redis for a counter is 10 seconds, and we set the ratio to
    2, and the max time for 30s. In this case, the counter will be cached for 5s
    (min(10/2, 30)). During those 5s, Limitador will not fetch the value of that
    counter from Redis, so it will answer faster, but it will also miss the
    updates done by other instances, so it can go over the limits in that 5s
    interval.
- Optional. Disabled by default.
- Format: set to "1" to enable.
- Note: "REDIS_URL" needs to be set.


#### `REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS`

- Used to configure the local cache when using Redis. See
[`REDIS_LOCAL_CACHE_ENABLED`](#redis_local_cache_enabled). This env only applies
when `"REDIS_LOCAL_CACHE_ENABLED" == 1`.
- Optional. Defaults to `1000`.
- Format: `integer`. Duration in milliseconds.


#### `REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS`

- Used to configure the local cache when using Redis. See
[`REDIS_LOCAL_CACHE_ENABLED`](#redis_local_cache_enabled). This env only applies
when `"REDIS_LOCAL_CACHE_ENABLED" == 1`.
- Optional. Defaults to `5000`.
- Format: `integer`. Duration in milliseconds.


#### `REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS`

- Used to configure the local cache when using Redis. See
[`REDIS_LOCAL_CACHE_ENABLED`](#redis_local_cache_enabled). This env only applies
when `"REDIS_LOCAL_CACHE_ENABLED" == 1`.
- Optional. Defaults to `10`.
- Format: `integer`.


#### `REDIS_URL`

- Redis URL. Required only when you want to use Redis to store the limits.
- Optional. By default, Limitador stores the limits in memory and does not
require Redis.
- Format: `string`, URL in the format of `"redis://127.0.0.1:6379"`.


#### `RUST_LOG`

- Defines the log level.
- Optional. Defaults to `"error"`.
- Format: `enum`: `"debug"`, `"error"`, `"info"`, `"warn"`, or `"trace"`.


### When built with the `infinispan` feature - _experimental_

#### `INFINISPAN_CACHE_NAME`

- The name of the Infinispan cache that Limitador will use to store limits and
  counters. This variable applies only when [`INFINISPAN_URL`](#infinispan_url) is
  set.
- Optional. By default, Limitador will use a cache called `"limitador"`.
- Format: `string`.


#### `INFINISPAN_COUNTERS_CONSISTENCY`

- Defines the consistency mode for the Infinispan counters created by Limitador.
  This variable applies only when [`INFINISPAN_URL`](#infinispan_url) is set.
- Optional. Defaults to `"strong"`.
- Format: `enum`: `"Strong"` or `"Weak"`.


#### `INFINISPAN_URL`

- Infinispan URL. Required only when you want to use Infinispan to store the
  limits.
- Optional. By default, Limitador stores the limits in memory and does not
  require Infinispan.
- Format: `URL`, in the format of `http://username:password@127.0.0.1:11222`.


#### `RATE_LIMIT_HEADERS`

- Enables rate limit response headers. Only supported by the RLS server.
- Optional. Defaults to `"NONE"`.
- Must be one of:
  - `"NONE"` - Does not add any additional headers to the http response.
  - `"DRAFT_VERSION_03"`.  Adds response headers per https://datatracker.ietf.org/doc/id/draft-polli-ratelimit-headers-03.html

