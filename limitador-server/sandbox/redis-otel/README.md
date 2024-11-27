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
                    "key": "req_method",
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

![Screenshot 2024-03-21 at 17-08-35 Jaeger UI](https://github.com/Kuadrant/limitador/assets/881529/585bb3bf-15e2-49ee-9ed8-423d33fe4df6)

> Recommended to start looking at `check_and_update` operation.

### Tear down sandbox

```bash
make clean
```

