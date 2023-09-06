# Deployment topologies

## In-memory

## Redis

### Redis active-active storage

The RedisLabs version of Redis supports [active-active
replication](https://docs.redislabs.com/latest/rs/concepts/intercluster-replication/).
Limitador is compatible with that deployment mode, but there are a few things to
take into account regarding limit accuracy.

#### Considerations

With an active-active deployment, the data needs to be replicated between
instances. An update in an instance takes a short time to be reflected in the
other. That time lag depends mainly on the network speed between the Redis
instances, and it affects the accuracy of the rate-limiting performed by
Limitador because it can go over limits while the updates of the counters are
being replicated.

The impact of that greatly depends on the use case. With limits of a few
seconds, and a low number of hits, we could easily go over limits. On the other
hand, if we have defined limits with a high number of hits and a long period,
the effect will be basically negligible. For example, if we define a limit of
one hour, and we know that the data takes around one second to be replicated,
the accuracy loss is going to be negligible.

#### Set up

In order to try active-active replication, you can follow this [tutorial from
RedisLabs](https://docs.redislabs.com/latest/rs/getting-started/getting-started-active-active/).

## Disk

Disk storage using [RocksDB](https://rocksdb.org/). Counters are held on disk (persistent).
