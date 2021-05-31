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


## INFINISPAN_CACHE_NAME

- The name of the Infinispan cache that Limitador will use to store limits and
counters. This variable applies only when [INFINISPAN_URL](#infinispan_url) is
set.
- Optional. By default, Limitador creates a cache called "limitador" and
configured as "local".
- Format: string.


## INFINISPAN_URL

- Infinispan URL. Required only when you want to use Infinispan to store the
  limits.
- Optional. By default, Limitador stores the limits in memory and does not
  require Infinispan.
- Format: URL like `http://username:password@127.0.0.1:11222`.


## LIMITS_FILE

- YAML file that contains the limits to create when limitador boots. If the
limits specified already exist, limitador will not delete them and will keep
their counters. If the limits in the file do not exist, Limitador will create
them. These limits can be modified with the HTTP api.
- Optional. By default, Limitador does not create or delete any limits when it
boots.
- Format: file path.


## LIMIT_NAME_IN_PROMETHEUS_LABELS

- Enables using limit names as labels in Prometheus metrics. This is disabled by
default because for a few limits it should be fine, but it could become a
problem when defining lots of limits. See the caution note in the Prometheus
docs: https://prometheus.io/docs/practices/naming/#labels
- Optional. Disabled by default.
- Format: set to "1" to enable.

## REDIS_LOCAL_CACHE_ENABLED

- Enables a storage implementation that uses Redis, but also caches some data in
memory. The idea is to improve throughput and latencies by caching the counters
in memory to reduce the number of accesses to Redis. To achieve that, this mode
sacrifices some rate-limit accuracy. This mode does two things:
    - Batches counter updates. Instead of updating the counters on every
    request, it updates them in memory and commits them to Redis in batches. The
    flushing interval can be configured with the
    [REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS](#redis_local_cache_flushing_period_ms)
    env. The trade-off is that when running several instances of Limitador,
    other instances will not become aware of the counter updates until they're
    committed to Redis.
    - Caches counters. Instead of fetching the value of a counter every time
    it's needed, the value is cached for a configurable period. The trade off is
    that when running several instances of Limitador, an instance will not
    become aware of the counter updates other instances do while the value is
    cached. When a counter is already at 0 (limit exceeded), it's cached until
    it expires in Redis. In this case, no matter what other instances do, we
    know that the quota will not be reestablished until the key expires in
    Redis, so in this case, rate-limit accuracy is not affected. When a counter
    has still some quota remaining the situation is different, that's why we can
    tune for how long it will be cached. The formula is as follows:
    MIN(ttl_in_redis/[REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS](#redis_local_cache_ttl_ratio_cached_counters),
    [REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS](#redis_local_cache_max_ttl_cached_counters_ms)).
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


## REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS

- Used to configure the local cache when using Redis. See
[REDIS_LOCAL_CACHE_ENABLED](#redis_local_cache_enabled). This env only applies
when "REDIS_LOCAL_CACHE_ENABLED" == 1.
- Optional. Defaults to 1000.
- Format: integer. Number of milliseconds.


## REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS

- Used to configure the local cache when using Redis. See
[REDIS_LOCAL_CACHE_ENABLED](#redis_local_cache_enabled). This env only applies
when "REDIS_LOCAL_CACHE_ENABLED" == 1.
- Optional. Defaults to 5000.
- Format: integer. Number of milliseconds.


## REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS

- Used to configure the local cache when using Redis. See
[REDIS_LOCAL_CACHE_ENABLED](#redis_local_cache_enabled). This env only applies
when "REDIS_LOCAL_CACHE_ENABLED" == 1.
- Optional. Defaults to 10.
- Format: integer.


## REDIS_URL

- Redis URL. Required only when you want to use Redis to store the limits.
- Optional. By default, Limitador stores the limits in memory and does not
require Redis.
- Format: URL like "redis://127.0.0.1:6379".


## RUST_LOG

- Defines the log level.
- Optional. Defaults to "error".
- Format: "debug", "error", "info", "warn", or "trace".
