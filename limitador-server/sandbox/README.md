## Testing Environment

### Requirements

* *docker* v24+

### Setup

Clone the project

```bash
git clone https://github.com/Kuadrant/limitador.git
cd limitador/limitador-server/sandbox
```

Check out `make help` for all the targets.

### Deployment options

| Limitador's configuration | Command | Info |
| ------------- | ----- | ----- |
| In-memory configuration | `make deploy-in-memory` | Counters are held in Limitador (ephemeral) |
| Redis | `make deploy-redis` | Uses Redis to store counters |
| Redis Secured | `make deploy-redis-tls` | Uses Redis with TLS and password protected to store counters |
| Redis Cached | `make deploy-redis-cached` | Uses Redis to store counters, with an in-memory cache |
| Redis Otel Instrumented | `make deploy-redis-otel` | Uses redis to store counters, [instrumented with opentelemetry](redis-otel/README.md) |
| Infinispan | `make deploy-infinispan` | Uses Infinispan to store counters |
| Disk | `make deploy-disk` | Uses disk to store counters |

### Limitador's admin HTTP endpoint

Limits

```bash
curl -i http://127.0.0.1:18080/limits/test_namespace
```

Counters

```bash
curl -i http://127.0.0.1:18080/counters/test_namespace
```

Metrics

```bash
curl -i http://127.0.0.1:18080/metrics
```

### Limitador's GRPC RateLimitService endpoint

Get `grpcurl`. You need [Go SDK](https://golang.org/doc/install) installed.

Golang version >= 1.18 (from [fullstorydev/grpcurl](https://github.com/fullstorydev/grpcurl/blob/v1.8.9/go.mod#L3))

```bash
make grpcurl
```

Inspect `RateLimitService` GRPC service

```bash
bin/grpcurl -plaintext 127.0.0.1:18081 describe envoy.service.ratelimit.v3.RateLimitService
```

Make a custom request

```bash
bin/grpcurl -plaintext -d @ 127.0.0.1:18081 envoy.service.ratelimit.v3.RateLimitService.ShouldRateLimit <<EOM
{
    "domain": "test_namespace",
    "hits_addend": 1,
    "descriptors": [
        {
            "entries": [
                {
                    "key": "req.method",
                    "value": "POST"
                }
            ]
        }
    ]
}
EOM
```

Do repeated requests. As the limit is set to max 5 request for 60 seconds,
you should see `OVER_LIMIT` response after 5 requests.

```bash
while :; do bin/grpcurl -plaintext -d @ 127.0.0.1:18081 envoy.service.ratelimit.v3.RateLimitService.ShouldRateLimit <<EOM; sleep 1; done
{
    "domain": "test_namespace",
    "hits_addend": 1,
    "descriptors": [
        {
            "entries": [
                {
                    "key": "req.method",
                    "value": "POST"
                }
            ]
        }
    ]
}
EOM
```

### Downstream traffic

**Upstream** service implemented by [httpbin.org](https://httpbin.org/)

```bash
curl -i -H "Host: example.com" http://127.0.0.1:18000/get
```

### Limitador Image

By default, the sandbox will run Limitador's `limitador-testing:latest` image.

**Building `limitador-testing:latest` image**

You can easily build the limitador's image from the current workspace code base with:

```bash
make build
```

The image will be tagged with `limitador-testing:latest`

**Using custom Limitador's image**

The `LIMITADOR_IMAGE` environment variable overrides the default image. For example:

```bash
make deploy-in-memory LIMITADOR_IMAGE=quay.io/kuadrant/limitador:latest
```

### Clean env

```bash
make clean
```
