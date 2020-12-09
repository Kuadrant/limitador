# Change Log

Notable changes to the Limitador server will be tracked in this document.

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
