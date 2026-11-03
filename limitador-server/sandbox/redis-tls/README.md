### Testing Redis security

Run `make deploy-redis-tls` from the sandbox directory. It generates a CA and
a server certificate with `DNS:redis` in its subject alternative names.
Limitador trusts that CA through `SSL_CERT_FILE` and verifies the Redis hostname.
Regenerate existing sandbox certificates after upgrading to rustls.

Execute bash shell in redis pod

```
docker compose -p sandbox exec redis /bin/bash
```

Connect to this Redis server with redis-cli:

```
redis-cli -h redis --tls --cacert /usr/local/etc/redis/certs/ca.crt -a foobared
```
