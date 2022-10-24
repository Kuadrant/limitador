# Change Log

Notable changes to Limitador will be tracked in this document.

## 0.3.0 - 2022-10-21

### Added 

 - Infinispan as an alternate storage for counters `feature = "infinispan_storage"`
 - `lenient_conditions` feature to allow for deprecated `Condition` syntax
 - Added _not equal_ (`!=`) operator support in `Condition`s

### Changed

 - `Limit`s are now _only_ held in memory, `Counter`s for there are stored using the `Storage` used
 - `Limit` identity now ignores the `max_value` and `name` field. So that they can be replaced properly
 - Serialized form for `Limit`s, used to lookup `Counter`s, changed. Upgrading to this version, existing persisted `Counter`s are lost 
 - New `Condition` syntax within `Limit`s: `KEY_B == 'VALUE_B'`
 - Simplified some function signatures to avoid explicit lifetimes 
 - Functions now take `Into<Namespace>` instead of `TryInto` as the conversion can't ever fail
 - Only require a reference `&Namespace` when ownership over the value isn't needed
 - Defaults for `(Async)CounterStorage` configurations are now public
 - Errors when creating a `CounterStorage` or `AsyncCounterStorage` are returned, instead of `panic!`ing

### Deleted

- Merge pull request #130 from Kuadrant/issue_100

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
