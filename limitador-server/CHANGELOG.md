# Change Log

Notable changes to the Limitador server will be tracked in this document.

## 0.4.0 - 2021-03-08

### Added

- Option to classify limited calls by limit name. This option is disabled by
default and can be enabled with the `LIMIT_NAME_IN_PROMETHEUS_LABELS` env
[#26](https://github.com/kuadrant/limitador/pull/26).

### Changed

- Updated build image to rust v1.5.0 and run image to alpine v3.13
[#23](https://github.com/kuadrant/limitador/pull/23).
- Limitador no longer load any limits by default. This was only done for testing
purposes, and we forgot to change it
[#10](https://github.com/kuadrant/limitador/pull/10).


## 0.3.0 - 2020-12-09

### Added

- Includes templates to deploy in Kubernetes and kind.
- Takes into account the "hits_addend" attribute from Envoy.

### Changed

- [__Breaking__] Switched to version 3 of the Envoy RLS protocol.
- The "Cached Redis" storage implementation has been improved, and it's now
exposed via the `REDIS_LOCAL_CACHE_ENABLED` env. The [configuration
doc](docs/configuration.md) contains the details on how to use it.


## 0.2.0 - 2020-11-06

First working release.
