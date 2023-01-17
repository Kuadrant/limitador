# Change Log

Notable changes to Limitador will be tracked in this document.

## 0.3.0 - 2022-12-20

- Added Infinispan limits storage (first version) [#38](https://github.com/Kuadrant/limitador/pull/38) [#43](https://github.com/Kuadrant/limitador/pull/43) [#44](https://github.com/Kuadrant/limitador/pull/44) [#45](https://github.com/Kuadrant/limitador/pull/45) 
- Perform semver-compatible dependency updates and move to edition 2021 [#51](https://github.com/Kuadrant/limitador/pull/51)
- Update Limitador to Tokio 1 async reactor [#54](https://github.com/Kuadrant/limitador/pull/54)
- Impl `From<&str>` for `Namespace` instead of `FromStr` as it can't fail [#76](https://github.com/Kuadrant/limitador/pull/76)
- `RaterLimiter` to take a `&Namespace` [#77](https://github.com/Kuadrant/limitador/pull/77)
- Keeps all Limits in memory only [#78](https://github.com/Kuadrant/limitador/pull/78)
- Removed lifecycles from public types [#80](https://github.com/Kuadrant/limitador/pull/80)
- `Limit`'s `max_value` and `name` fields play no role in their identity… [#93](https://github.com/Kuadrant/limitador/pull/93)
- Scanner based parsing of `Limit`'s conditions [#109](https://github.com/Kuadrant/limitador/pull/109)
- Add support for `!=` in conditions [#113](https://github.com/Kuadrant/limitador/pull/113)
- Bubble error up when connecting to Redis fails [#130](https://github.com/Kuadrant/limitador/pull/130)

## 0.2.0 - 2021-03-08

### Added

- New "name" attribute for `Limit`.
- Limitador now exposes Prometheus metrics. In particular, it collects the
number of authorized and limited requests classified by namespace. Optionally,
it can also classify the limited requests by limit name. Limitador also exposes
the `limitador_up` metric that can be used as a healthcheck.
- Added `RateLimiterBuilder` and `AsyncRateLimiterBuilder` that provide a
convenient way to instantiate `RateLimiter` and `AsyncRateLimiter`.

### Changed

- Improved the `CachedRedisStorage` storage implementation.

### Deleted

## 0.1.3 - 2020-10-29

First working release.
