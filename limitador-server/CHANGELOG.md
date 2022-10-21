# Change Log

Notable changes to the Limitador server will be tracked in this document.

## 0.5.1 - 2022-05-25

### Changed

- Update paperclip dep to latest actix4 branch head [#59](https://github.com/kuadrant/limitador/pull/59)
- Only use utility functions when needed [#61](https://github.com/kuadrant/limitador/pull/61)
- Use Alpine for building instead of cross-compiling to musl [#61](https://github.com/kuadrant/limitador/pull/61)
- Cargo.toml: add a profile release with LTO and CGU=1 for limitador-server [#61](https://github.com/kuadrant/limitador/pull/61)
- Bump alpine base image to 3.16 [#65](https://github.com/kuadrant/limitador/pull/65)

## 0.5.0 - 2022-01-31

### Added

- Support for Infinispan (tested with 11.0.9.Final). You can enable it via the
`INFINISPAN_URL` env variable. [#38](https://github.com/kuadrant/limitador/pull/38),
[#40](https://github.com/kuadrant/limitador/pull/40), [#44](https://github.com/kuadrant/limitador/pull/44),
[#45](https://github.com/kuadrant/limitador/pull/45).

### Changed

- Moved to the Tokio 1 async reactor. [#54](https://github.com/kuadrant/limitador/pull/54).

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
doc](../doc/server/configuration.md) contains the details on how to use it.


## 0.2.0 - 2020-11-06

First working release.
