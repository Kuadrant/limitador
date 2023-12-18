### Testing  redis security

Execute bash shell in redis pod

```
docker compose -p sandbox exec redis /bin/bash
```

Connect to this Redis server with redis-cli:

```
root@e024a29b74ba:/data# redis-cli --tls --cacert /usr/local/etc/redis/certs/ca.crt -a foobared
```
