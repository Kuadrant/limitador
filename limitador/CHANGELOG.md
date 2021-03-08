# Change Log

Notable changes to Limitador will be tracked in this document.

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
