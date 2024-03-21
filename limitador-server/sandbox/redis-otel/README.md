# Limitador instrumentation sandbox

Limitador is configured to push traces to an [opentelemetry collector](https://opentelemetry.io/docs/collector/).

### Run sandbox

```bash
make build
make deploy-redis-otel
```

### Run some traffic

```bash
make grpcurl
```

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

### See the trace in UI

```
firefox -private-window "localhost:16686"
```

> Recommended to start looking at `check_and_update` operation.

### Tear down sandbox

```bash
make clean
```

