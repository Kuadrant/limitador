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
| Infinispan | `make deploy-infinispan` | Uses Infinispan to store counters |

### Limitador's admin HTTP endpoint

```bash
curl -i http://127.0.0.1:18080/limits/test_namespace
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
