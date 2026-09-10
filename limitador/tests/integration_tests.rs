#![deny(clippy::all)]

macro_rules! test_with_all_storage_impls {
    // This macro uses the "paste" crate to define the names of the functions.
    // Also, the Redis tests cannot be run in parallel. The "serial" tag from
    // the "serial-test" crate takes care of that.
    ($function:ident) => {
        paste::item! {
            #[tokio::test]
            async fn [<$function _in_memory_storage>]() {
                let rate_limiter =
                    RateLimiter::new_with_storage(Box::<InMemoryStorage>::default());
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "distributed_storage")]
            #[tokio::test]
            async fn [<$function _distributed_storage>]() {
                let rate_limiter =
                    RateLimiter::new_with_storage(Box::new(CrInMemoryStorage::new("test_node".to_owned(), 10_000, "127.0.0.1:19876".to_owned(), vec![])));
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "disk_storage")]
            #[tokio::test]
            async fn [<$function _disk_storage>]() {
                let dir = TempDir::new().expect("We should have a dir!");
                let rate_limiter =
                    RateLimiter::new_with_storage(Box::new(DiskStorage::open(dir.path(), OptimizeFor::Throughput).expect("Couldn't open temp dir")));
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_sync_redis>]() {
                let storage = RedisStorage::default();
                storage.clear().unwrap();
                let rate_limiter = RateLimiter::new_with_storage(
                    Box::new(storage)
                );
                RedisStorage::default().clear().unwrap();
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_async_redis>]() {
                let storage = AsyncRedisStorage::new("redis://127.0.0.1:6379").await.expect("We need a Redis running locally");
                storage.clear().await.unwrap();
                let rate_limiter = AsyncRateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_async_redis_and_local_cache>]() {
                let storage_builder = CachedRedisStorageBuilder::new("redis://127.0.0.1:6379").
                    flushing_period(Duration::from_millis(2)).
                    max_cached_counters(10000);
                let storage = storage_builder.build().await.expect("We need a Redis running locally");
                storage.clear().await.unwrap();
                let rate_limiter = AsyncRateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }
        }
    };
}

// Backends that have implemented `CounterStorage`/`AsyncCounterStorage`'s
// `reserve`/`release_reservation` so far - either shared (in-memory, both Redis storages) or,
// for disk/cached-Redis, a local-memory-only starting point (see `LocalReservationRegistry`).
// Distributed storage still relies on the traits' default (unsupported) impl and is
// deliberately left out, rather than run and fail - it's also not compiled by default
// (`distributed_storage` isn't in either crate's default features) and not production-ready.
macro_rules! test_with_reservation_capable_storage_impls {
    ($function:ident) => {
        paste::item! {
            #[tokio::test]
            async fn [<$function _in_memory_storage>]() {
                let rate_limiter =
                    RateLimiter::new_with_storage(Box::<InMemoryStorage>::default());
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "disk_storage")]
            #[tokio::test]
            async fn [<$function _disk_storage>]() {
                let dir = TempDir::new().expect("We should have a dir!");
                let rate_limiter =
                    RateLimiter::new_with_storage(Box::new(DiskStorage::open(dir.path(), OptimizeFor::Throughput).expect("Couldn't open temp dir")));
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_sync_redis>]() {
                let storage = RedisStorage::default();
                storage.clear().unwrap();
                let rate_limiter = RateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_async_redis>]() {
                let storage = AsyncRedisStorage::new("redis://127.0.0.1:6379").await.expect("We need a Redis running locally");
                storage.clear().await.unwrap();
                let rate_limiter = AsyncRateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_async_redis_and_local_cache>]() {
                let storage_builder = CachedRedisStorageBuilder::new("redis://127.0.0.1:6379").
                    flushing_period(Duration::from_millis(2)).
                    max_cached_counters(10000);
                let storage = storage_builder.build().await.expect("We need a Redis running locally");
                storage.clear().await.unwrap();
                let rate_limiter = AsyncRateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }
        }
    };
}

#[cfg(feature = "distributed_storage")]
async fn distributed_storage_factory(
    count: usize,
) -> Vec<crate::helpers::tests_limiter::TestsLimiter> {
    use crate::helpers::tests_limiter::TestsLimiter;
    use limitador::storage::distributed::CrInMemoryStorage;
    use limitador::RateLimiter;

    let addresses = (0..count)
        .map(|i| format!("127.0.0.1:{}", 5200 + i))
        .collect::<Vec<String>>();
    (0..count)
        .map(|i| {
            let node = format!("n{i}");
            let listen_address = addresses.get(i).unwrap().to_owned();
            let peer_urls = addresses
                .iter()
                .map(|x| format!("http://{x}"))
                .collect::<Vec<String>>();

            TestsLimiter::new_from_blocking_impl(RateLimiter::new_with_storage(Box::new(
                CrInMemoryStorage::new(node, 10_000, listen_address, peer_urls),
            )))
        })
        .collect::<Vec<TestsLimiter>>()
}

macro_rules! test_with_distributed_storage_impls {
    // This macro uses the "paste" crate to define the names of the functions.
    // Also, the Redis tests cannot be run in parallel. The "serial" tag from
    // the "serial-test" crate takes care of that.
    ($function:ident) => {
        paste::item! {
            #[cfg(feature = "distributed_storage")]
            #[tokio::test]
            async fn [<$function _distributed_storage>]() {
               $function(crate::distributed_storage_factory).await;
            }
        }
    };
}

mod helpers;

#[cfg(test)]
mod test {
    extern crate limitador;

    #[allow(dead_code)]
    async fn eventually<F>(
        timeout: Duration,
        tick: Duration,
        condition: impl Fn() -> F,
    ) -> Result<bool, Elapsed>
    where
        F: Future<Output = bool>,
    {
        tokio::time::timeout(timeout, async move {
            let mut i = tokio::time::interval(tick);
            loop {
                if condition().await {
                    return true;
                }
                i.tick().await;
            }
        })
        .await
    }

    // To be able to pass the tests without Redis
    cfg_if::cfg_if! {
        if #[cfg(feature = "redis_storage")] {
            use limitador::storage::redis::AsyncRedisStorage;
            use limitador::storage::redis::CachedRedisStorageBuilder;
            use limitador::storage::redis::RedisStorage;

            use limitador::AsyncRateLimiter;
            use serial_test::serial;
            use crate::test::limitador::storage::CounterStorage;
            use crate::test::limitador::storage::AsyncCounterStorage;
        }
    }

    use self::limitador::counter::Counter;
    use self::limitador::RateLimiter;
    use crate::helpers::tests_limiter::*;
    use limitador::limit::{Context, Expression, Limit};
    #[cfg(feature = "disk_storage")]
    use limitador::storage::disk::{DiskStorage, OptimizeFor};
    #[cfg(feature = "distributed_storage")]
    use limitador::storage::distributed::CrInMemoryStorage;
    use limitador::storage::in_memory::InMemoryStorage;
    use std::collections::{HashMap, HashSet};
    use std::future::Future;
    use std::thread::sleep;
    use std::time::Duration;
    #[cfg(feature = "disk_storage")]
    use tempfile::TempDir;
    use tokio::time::error::Elapsed;

    test_with_all_storage_impls!(get_namespaces);
    test_with_all_storage_impls!(get_namespaces_returns_empty_when_there_arent_any);
    test_with_all_storage_impls!(get_namespaces_doesnt_return_the_ones_that_no_longer_have_limits);
    test_with_all_storage_impls!(add_a_limit);
    test_with_all_storage_impls!(add_limit_without_vars);
    test_with_all_storage_impls!(add_several_limits_in_the_same_namespace);
    test_with_all_storage_impls!(delete_limit);
    test_with_all_storage_impls!(delete_limit_also_deletes_associated_counters);
    test_with_all_storage_impls!(get_limits_returns_empty_if_no_limits_in_namespace);
    test_with_all_storage_impls!(delete_limits_of_a_namespace);
    test_with_all_storage_impls!(delete_limits_does_not_delete_limits_from_other_namespaces);
    test_with_all_storage_impls!(delete_limits_of_a_namespace_also_deletes_counters);
    test_with_all_storage_impls!(delete_limits_of_an_empty_namespace_does_nothing);
    test_with_all_storage_impls!(rate_limited);
    test_with_all_storage_impls!(rate_limited_id_counter);
    test_with_all_storage_impls!(multiple_limits_rate_limited);
    test_with_all_storage_impls!(rate_limited_with_delta_higher_than_one);
    test_with_all_storage_impls!(rate_limited_with_delta_higher_than_max);
    test_with_all_storage_impls!(takes_into_account_only_vars_of_the_limits);
    test_with_all_storage_impls!(is_rate_limited_returns_false_when_no_limits_in_namespace);
    test_with_all_storage_impls!(is_rate_limited_returns_false_when_no_matching_limits);
    test_with_all_storage_impls!(is_rate_limited_applies_limit_if_its_unconditional);
    test_with_all_storage_impls!(check_rate_limited_and_update);
    test_with_all_storage_impls!(check_rate_limited_and_update_load_counters);
    test_with_all_storage_impls!(check_rate_limited_and_update_returns_true_if_no_limits_apply);
    test_with_all_storage_impls!(check_rate_limited_and_update_applies_limit_if_its_unconditional);
    test_with_all_storage_impls!(get_counters);
    test_with_all_storage_impls!(get_counters_returns_empty_when_no_limits_in_namespace);
    test_with_all_storage_impls!(get_counters_returns_empty_when_no_counters_in_namespace);
    test_with_all_storage_impls!(get_counters_does_not_return_expired_ones);
    test_with_all_storage_impls!(configure_with_creates_the_given_limits);
    test_with_all_storage_impls!(configure_with_keeps_the_given_limits_and_counters_if_they_exist);
    test_with_all_storage_impls!(configure_with_deletes_all_except_the_limits_given);
    test_with_all_storage_impls!(configure_with_updates_the_limits);
    test_with_all_storage_impls!(add_limit_only_adds_if_not_present);

    test_with_reservation_capable_storage_impls!(reserve_accounts_for_outstanding_reservations);
    test_with_reservation_capable_storage_impls!(
        commit_reservation_applies_actual_amount_and_releases_hold
    );
    test_with_reservation_capable_storage_impls!(reserve_without_matching_limits_is_a_noop);
    test_with_reservation_capable_storage_impls!(expired_reservations_do_not_count_as_outstanding);
    test_with_reservation_capable_storage_impls!(
        reserve_denies_when_amount_alone_exceeds_max_value
    );
    test_with_reservation_capable_storage_impls!(reserve_admits_when_amount_fits);
    test_with_reservation_capable_storage_impls!(
        reserve_with_amount_zero_creates_no_reservation_entry
    );
    test_with_reservation_capable_storage_impls!(
        reserve_then_commit_then_reserve_reflects_committed_usage
    );
    // Not run against `disk_storage`, unlike the other reservation tests above:
    // `RocksDbStorage::insert_or_update` silently no-ops a delta that alone exceeds
    // `max_value`, so `update_counters`'s forced overshoot below never actually gets
    // written - see https://github.com/Kuadrant/limitador/issues/516. Once that's fixed,
    // replace this whole manual block with a single
    // `test_with_reservation_capable_storage_impls!(reserve_denies_amount_zero_when_already_over_limit);`
    // like the other reservation tests above.
    #[tokio::test]
    async fn reserve_denies_amount_zero_when_already_over_limit_in_memory_storage() {
        let rate_limiter = RateLimiter::new_with_storage(Box::<InMemoryStorage>::default());
        reserve_denies_amount_zero_when_already_over_limit(
            &mut TestsLimiter::new_from_blocking_impl(rate_limiter),
        )
        .await;
    }

    #[cfg(feature = "redis_storage")]
    #[tokio::test]
    #[serial]
    async fn reserve_denies_amount_zero_when_already_over_limit_with_sync_redis() {
        let storage = RedisStorage::default();
        storage.clear().unwrap();
        let rate_limiter = RateLimiter::new_with_storage(Box::new(storage));
        reserve_denies_amount_zero_when_already_over_limit(
            &mut TestsLimiter::new_from_blocking_impl(rate_limiter),
        )
        .await;
    }

    #[cfg(feature = "redis_storage")]
    #[tokio::test]
    #[serial]
    async fn reserve_denies_amount_zero_when_already_over_limit_with_async_redis() {
        let storage = AsyncRedisStorage::new("redis://127.0.0.1:6379")
            .await
            .expect("We need a Redis running locally");
        storage.clear().await.unwrap();
        let rate_limiter = AsyncRateLimiter::new_with_storage(Box::new(storage));
        reserve_denies_amount_zero_when_already_over_limit(&mut TestsLimiter::new_from_async_impl(
            rate_limiter,
        ))
        .await;
    }

    #[cfg(feature = "redis_storage")]
    #[tokio::test]
    #[serial]
    async fn reserve_denies_amount_zero_when_already_over_limit_with_async_redis_and_local_cache() {
        let storage_builder = CachedRedisStorageBuilder::new("redis://127.0.0.1:6379")
            .flushing_period(Duration::from_millis(2))
            .max_cached_counters(10000);
        let storage = storage_builder
            .build()
            .await
            .expect("We need a Redis running locally");
        storage.clear().await.unwrap();
        let rate_limiter = AsyncRateLimiter::new_with_storage(Box::new(storage));
        reserve_denies_amount_zero_when_already_over_limit(&mut TestsLimiter::new_from_async_impl(
            rate_limiter,
        ))
        .await;
    }

    test_with_distributed_storage_impls!(distributed_rate_limited);

    // All these functions need to use async/await. That's needed to support
    // both the sync and the async implementations of the rate limiter.

    async fn get_namespaces(rate_limiter: &mut TestsLimiter) {
        let limits = vec![
            Limit::new(
                "first_namespace",
                10,
                60,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
            Limit::new(
                "second_namespace",
                20,
                60,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
        ];

        for limit in limits {
            rate_limiter.add_limit(&limit).await;
        }

        for &ns in ["first_namespace", "second_namespace"].iter() {
            assert!(rate_limiter.get_namespaces().await.contains(&ns.into()));
        }
    }

    async fn get_namespaces_returns_empty_when_there_arent_any(rate_limiter: &mut TestsLimiter) {
        assert!(rate_limiter.get_namespaces().await.is_empty())
    }

    async fn get_namespaces_doesnt_return_the_ones_that_no_longer_have_limits(
        rate_limiter: &mut TestsLimiter,
    ) {
        let lim1 = Limit::new(
            "first_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let lim2 = Limit::new(
            "second_namespace",
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        for limit in [&lim1, &lim2] {
            rate_limiter.add_limit(limit).await;
        }
        rate_limiter.delete_limit(&lim2).await.unwrap();

        assert!(rate_limiter
            .get_namespaces()
            .await
            .contains(&"first_namespace".into()));
        assert!(!rate_limiter
            .get_namespaces()
            .await
            .contains(&"second_namespace".into()));
    }

    async fn add_a_limit(rate_limiter: &mut TestsLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut expected_result = HashSet::new();
        expected_result.insert(limit);

        assert_eq!(
            rate_limiter.get_limits("test_namespace").await,
            expected_result
        )
    }

    async fn add_limit_without_vars(rate_limiter: &mut TestsLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            Vec::default(),
        );

        rate_limiter.add_limit(&limit).await;

        let mut expected_result = HashSet::new();
        expected_result.insert(limit);

        assert_eq!(
            rate_limiter.get_limits("test_namespace").await,
            expected_result
        )
    }

    async fn add_several_limits_in_the_same_namespace(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit_1 = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'POST'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let limit_2 = Limit::new(
            namespace,
            5,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit_1).await;
        rate_limiter.add_limit(&limit_2).await;

        let mut expected_result = HashSet::new();
        expected_result.insert(limit_1);
        expected_result.insert(limit_2);

        assert_eq!(rate_limiter.get_limits(namespace).await, expected_result)
    }

    async fn delete_limit(rate_limiter: &mut TestsLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        rate_limiter.delete_limit(&limit).await.unwrap();

        assert!(rate_limiter.get_limits("test_namespace").await.is_empty())
    }

    async fn delete_limit_also_deletes_associated_counters(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values.into(), 1)
            .await
            .unwrap();

        rate_limiter.delete_limit(&limit).await.unwrap();

        assert!(rate_limiter
            .get_counters(namespace)
            .await
            .unwrap()
            .is_empty())
    }

    async fn get_limits_returns_empty_if_no_limits_in_namespace(rate_limiter: &mut TestsLimiter) {
        assert!(rate_limiter.get_limits("test_namespace").await.is_empty())
    }

    async fn delete_limits_of_a_namespace(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limits = [
            Limit::new(
                namespace,
                10,
                60,
                vec!["req_method == 'POST'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
            Limit::new(
                namespace,
                5,
                60,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
        ];

        for limit in limits.iter() {
            rate_limiter.add_limit(limit).await;
        }

        rate_limiter.delete_limits(namespace).await.unwrap();

        assert!(rate_limiter.get_limits(namespace).await.is_empty())
    }

    async fn delete_limits_does_not_delete_limits_from_other_namespaces(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace1 = "test_namespace_1";
        let namespace2 = "test_namespace_2";

        rate_limiter
            .add_limit(&Limit::new(
                namespace1,
                10,
                60,
                vec!["x == '10'".try_into().expect("failed parsing!")],
                vec!["z".try_into().expect("failed parsing!")],
            ))
            .await;
        rate_limiter
            .add_limit(&Limit::new(
                namespace2,
                5,
                60,
                vec!["x == '10'".try_into().expect("failed parsing!")],
                vec!["z".try_into().expect("failed parsing!")],
            ))
            .await;

        rate_limiter.delete_limits(namespace1).await.unwrap();

        assert!(rate_limiter.get_limits(namespace1).await.is_empty());
        assert_eq!(rate_limiter.get_limits(namespace2).await.len(), 1);
    }

    async fn delete_limits_of_a_namespace_also_deletes_counters(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            5,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values.into(), 1)
            .await
            .unwrap();

        rate_limiter.delete_limits(namespace).await.unwrap();

        assert!(rate_limiter
            .get_counters(namespace)
            .await
            .unwrap()
            .is_empty())
    }

    async fn delete_limits_of_an_empty_namespace_does_nothing(rate_limiter: &mut TestsLimiter) {
        rate_limiter.delete_limits("test_namespace").await.unwrap()
    }

    async fn rate_limited(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        for i in 0..max_hits {
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            rate_limiter
                .update_counters(namespace, &ctx, 1)
                .await
                .unwrap();
        }
        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn rate_limited_id_counter(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::with_id(
            "test-rate_limited_id_counter",
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        for i in 0..max_hits {
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            rate_limiter
                .update_counters(namespace, &ctx, 1)
                .await
                .unwrap();
        }
        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn multiple_limits_rate_limited(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limits = vec![
            Limit::new(
                namespace,
                max_hits,
                60,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
            Limit::new(
                namespace,
                max_hits + 1,
                60,
                vec!["req_method == 'POST'".try_into().expect("failed parsing!")],
                vec!["app_id".try_into().expect("failed parsing!")],
            ),
        ];

        for limit in limits {
            rate_limiter.add_limit(&limit).await;
        }

        let mut get_values: HashMap<String, String> = HashMap::new();
        get_values.insert("req_method".to_string(), "GET".to_string());
        get_values.insert("app_id".to_string(), "test_app_id".to_string());
        let get_ctx = get_values.into();

        let mut post_values: HashMap<String, String> = HashMap::new();
        post_values.insert("req_method".to_string(), "POST".to_string());
        post_values.insert("app_id".to_string(), "test_app_id".to_string());
        let post_ctx = post_values.into();

        for i in 0..max_hits {
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &get_ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &post_ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            rate_limiter
                .check_rate_limited_and_update(namespace, &get_ctx, 1, false)
                .await
                .unwrap();
            rate_limiter
                .check_rate_limited_and_update(namespace, &post_ctx, 1, false)
                .await
                .unwrap();
        }

        // We wait for the flushing period to pass so the counters are flushed in the cached storage
        tokio::time::sleep(Duration::from_millis(40)).await;

        assert!(
            rate_limiter
                .is_rate_limited(namespace, &get_ctx, 1)
                .await
                .unwrap()
                .limited
        );
        assert!(
            !rate_limiter
                .is_rate_limited(namespace, &post_ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn rate_limited_with_delta_higher_than_one(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        // Report 5 hits twice. The limit is 10, so the first limited call should be
        // the third one.
        for _ in 0..2 {
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &ctx, 5)
                    .await
                    .unwrap()
                    .limited
            );
            rate_limiter
                .update_counters(namespace, &ctx, 5)
                .await
                .unwrap();
        }
        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn rate_limited_with_delta_higher_than_max(rate_limiter: &mut TestsLimiter) {
        let max = 10;
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            max,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, max + 1)
                .await
                .unwrap()
                .limited
        )
    }

    async fn takes_into_account_only_vars_of_the_limits(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for i in 0..max_hits {
            // Add an extra value that does not apply to the limit on each
            // iteration. It should not affect.
            let mut values = values.clone();
            values.insert("does_not_apply".to_string(), i.to_string());
            let ctx = values.into();

            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            rate_limiter
                .update_counters(namespace, &ctx, 1)
                .await
                .unwrap();
        }
        let ctx = values.into();
        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn is_rate_limited_returns_false_when_no_limits_in_namespace(
        rate_limiter: &mut TestsLimiter,
    ) {
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        let ctx = values.into();

        assert!(
            !rate_limiter
                .is_rate_limited("test_namespace", &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn is_rate_limited_returns_false_when_no_matching_limits(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        // Notice that does not match because the method is "POST".
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "POST".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        assert!(
            !rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn is_rate_limited_applies_limit_if_its_unconditional(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            Vec::default(), // unconditional
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        assert!(
            rate_limiter
                .is_rate_limited(namespace, &ctx, 1)
                .await
                .unwrap()
                .limited
        );
    }

    async fn check_rate_limited_and_update(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        for _ in 0..max_hits {
            assert!(
                !rate_limiter
                    .check_rate_limited_and_update(namespace, &ctx, 1, false)
                    .await
                    .unwrap()
                    .limited
            );
        }

        assert!(
            rate_limiter
                .check_rate_limited_and_update(namespace, &ctx, 1, false)
                .await
                .unwrap()
                .limited
        );
    }

    async fn check_rate_limited_and_update_load_counters(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        for hit in 0..max_hits {
            let result = rate_limiter
                .check_rate_limited_and_update(namespace, &ctx, 1, true)
                .await
                .unwrap();
            assert!(!result.limited);
            assert_eq!(result.counters.len(), 1);

            for counter in result.counters.iter() {
                if let Some(ttl) = counter.expires_in() {
                    assert!(ttl.as_secs() <= 60);
                }
                assert_eq!(counter.remaining().unwrap(), 3 - (hit + 1));
            }
        }

        let result = rate_limiter
            .check_rate_limited_and_update(namespace, &ctx, 1, true)
            .await
            .unwrap();
        assert!(result.limited);
        assert_eq!(result.counters.len(), 1);

        for counter in result.counters.iter() {
            if let Some(ttl) = counter.expires_in() {
                assert!(ttl.as_secs() <= 60);
            }
            assert_eq!(counter.remaining().unwrap(), 0);
        }
    }

    async fn check_rate_limited_and_update_returns_true_if_no_limits_apply(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());
        // Does not match the limit defined
        values.insert("req_method".to_string(), "POST".to_string());
        let ctx = values.into();

        assert!(
            !rate_limiter
                .check_rate_limited_and_update(namespace, &ctx, 1, false)
                .await
                .unwrap()
                .limited
        );
    }

    async fn check_rate_limited_and_update_applies_limit_if_its_unconditional(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            Vec::default(), // unconditional
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        assert!(
            rate_limiter
                .check_rate_limited_and_update(namespace, &ctx, 1, false)
                .await
                .unwrap()
                .limited
        );
    }

    async fn get_counters(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 10;
        let hits_app_1 = 1;
        let hits_app_2 = 5;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        let ctx = values.into();
        rate_limiter
            .update_counters(namespace, &ctx, hits_app_1)
            .await
            .unwrap();

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "2".to_string());
        let ctx = values.into();
        rate_limiter
            .update_counters(namespace, &ctx, hits_app_2)
            .await
            .unwrap();

        assert_eq!(rate_limiter.get_limits(namespace).await.len(), 1);

        let counters = rate_limiter.get_counters(namespace).await.unwrap();

        assert_eq!(counters.len(), 2);

        for counter in counters {
            let app_id = counter.set_variables().get("app_id").unwrap();
            let remaining = counter.remaining().unwrap();

            match app_id.as_str() {
                "1" => assert_eq!(remaining, max_hits - hits_app_1),
                "2" => assert_eq!(remaining, max_hits - hits_app_2),
                _ => panic!("Unexpected app ID"),
            }
        }
    }

    async fn get_counters_returns_empty_when_no_limits_in_namespace(
        rate_limiter: &mut TestsLimiter,
    ) {
        assert!(rate_limiter
            .get_counters("test_namespace")
            .await
            .unwrap()
            .is_empty())
    }

    async fn get_counters_returns_empty_when_no_counters_in_namespace(
        rate_limiter: &mut TestsLimiter,
    ) {
        // There's a limit, but no counters. The result should be empty.

        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        assert!(rate_limiter
            .get_counters("test_namespace")
            .await
            .unwrap()
            .is_empty())
    }

    async fn get_counters_does_not_return_expired_ones(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let limit_time = 1;

        let limit = Limit::new(
            namespace,
            10,
            limit_time,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        let ctx = values.into();
        rate_limiter
            .update_counters(namespace, &ctx, 1)
            .await
            .unwrap();

        // Give it some extra time to expire
        sleep(Duration::from_secs(limit_time + 1));

        assert_eq!(rate_limiter.get_counters(namespace).await.unwrap().len(), 0);
    }

    async fn configure_with_creates_the_given_limits(rate_limiter: &mut TestsLimiter) {
        let first_limit = Limit::new(
            "first_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let second_limit = Limit::new(
            "second_namespace",
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter
            .configure_with(vec![first_limit.clone(), second_limit.clone()])
            .await
            .unwrap();

        assert!(rate_limiter
            .get_limits("first_namespace")
            .await
            .contains(&first_limit));

        assert!(rate_limiter
            .get_limits("second_namespace")
            .await
            .contains(&second_limit));
    }

    async fn configure_with_keeps_the_given_limits_and_counters_if_they_exist(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "test_namespace";
        let max_value = 10;
        let hits_to_report = 1;

        let limit = Limit::new(
            namespace,
            max_value,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        let ctx = values.into();
        rate_limiter
            .update_counters(namespace, &ctx, hits_to_report)
            .await
            .unwrap();

        rate_limiter
            .configure_with(vec![limit.clone()])
            .await
            .unwrap();

        assert!(rate_limiter.get_limits(namespace).await.contains(&limit));

        let counters: Vec<Counter> = rate_limiter
            .get_counters(namespace)
            .await
            .unwrap()
            .drain()
            .collect();

        assert_eq!(counters.len(), 1);
        assert_eq!(counters[0].remaining().unwrap(), max_value - hits_to_report);
    }

    async fn configure_with_deletes_all_except_the_limits_given(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit_to_be_kept = Limit::new(
            namespace,
            10,
            1,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let limit_to_be_deleted = Limit::new(
            namespace,
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        for limit in [&limit_to_be_kept, &limit_to_be_deleted].iter() {
            rate_limiter.add_limit(limit).await;
        }

        rate_limiter
            .configure_with(vec![limit_to_be_kept.clone()])
            .await
            .unwrap();

        let limits = rate_limiter.get_limits(namespace).await;

        assert!(limits.contains(&limit_to_be_kept));
        assert!(!limits.contains(&limit_to_be_deleted));
    }

    async fn configure_with_updates_the_limits(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit_orig = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let limit_update = Limit::new(
            namespace,
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        rate_limiter.add_limit(&limit_orig).await;

        rate_limiter
            .configure_with(vec![limit_update.clone()])
            .await
            .unwrap();

        let limits = rate_limiter.get_limits(namespace).await;

        assert_eq!(limits.len(), 1);
        assert_eq!(limits.iter().next().unwrap().max_value(), 20);
    }

    async fn add_limit_only_adds_if_not_present(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit_1 = Limit::new(
            namespace,
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let limit_2 = Limit::new(
            namespace,
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let mut limit_3 = Limit::new(
            namespace,
            20,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );
        limit_3.set_name("Name is irrelevant too".to_owned());

        assert!(rate_limiter.add_limit(&limit_1).await);
        assert!(!rate_limiter.add_limit(&limit_2).await);
        assert!(!rate_limiter.add_limit(&limit_3).await);

        let limits = rate_limiter.get_limits(namespace).await;

        assert_eq!(limits.len(), 1);
        let known_limit = limits.iter().next().unwrap();
        assert_eq!(known_limit.max_value(), 10);
        assert_eq!(known_limit.name(), None);
    }

    async fn reserve_accounts_for_outstanding_reservations(rate_limiter: &mut TestsLimiter) {
        let namespace = "reservations";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        let first = rate_limiter
            .reserve(namespace, &ctx, 6, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!first.limited);
        assert!(first.reservation_id.is_some());

        // value(0) + outstanding(6) + 6 = 12 > 10: rejected
        let second = rate_limiter
            .reserve(namespace, &ctx, 6, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(second.limited);
        assert!(second.reservation_id.is_none());

        // value(0) + outstanding(6) + 4 = 10 <= 10: admitted
        let third = rate_limiter
            .reserve(namespace, &ctx, 4, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!third.limited);
    }

    async fn commit_reservation_applies_actual_amount_and_releases_hold(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "commit";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        let reserved = rate_limiter
            .reserve(namespace, &ctx, 6, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        let reservation_id = reserved.reservation_id.expect("should be admitted");

        // Still held: 0 + outstanding(6) + 6 = 12 > 10
        let blocked = rate_limiter
            .reserve(namespace, &ctx, 6, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(blocked.limited);

        // Real usage turned out lower than the estimate
        let commit = rate_limiter
            .commit_reservation(namespace, &ctx, &reservation_id, 2)
            .await
            .unwrap();
        assert!(commit.reservation_released);

        // Committing again is a no-op release, but still applies actual_amount unconditionally
        let second_commit = rate_limiter
            .commit_reservation(namespace, &ctx, &reservation_id, 2)
            .await
            .unwrap();
        assert!(!second_commit.reservation_released);

        // Counter is now at 4 (2 + 2) with no outstanding reservations: 4 + 6 = 10 <= 10
        let after = rate_limiter
            .reserve(namespace, &ctx, 6, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!after.limited);
    }

    // The main Reserve/Commit workflow: real usage committed after a reservation must be
    // visible to later reservations, not just to other reservations' outstanding accounting.
    // A backend that reads a stale/empty view of the counter's committed value (rather than
    // outstanding reservations) would incorrectly admit the final reserve below.
    async fn reserve_then_commit_then_reserve_reflects_committed_usage(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "reserve_commit_reserve";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        // Reserve 4, but real usage turns out to be higher than the estimate.
        let reserved = rate_limiter
            .reserve(namespace, &ctx, 4, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!reserved.limited);
        let reservation_id = reserved.reservation_id.expect("should be admitted");

        let commit = rate_limiter
            .commit_reservation(namespace, &ctx, &reservation_id, 9)
            .await
            .unwrap();
        assert!(commit.reservation_released);

        // Counter is now at 9 with no outstanding reservations: 9 + 5 = 14 > 10, denied.
        let after = rate_limiter
            .reserve(namespace, &ctx, 5, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(after.limited);
    }

    async fn reserve_without_matching_limits_is_a_noop(rate_limiter: &mut TestsLimiter) {
        let result = rate_limiter
            .reserve(
                "empty",
                &Context::default(),
                5,
                Some(Duration::from_secs(10)),
                false,
            )
            .await
            .unwrap();
        assert!(!result.limited);
        assert!(result.reservation_id.is_none());
    }

    async fn expired_reservations_do_not_count_as_outstanding(rate_limiter: &mut TestsLimiter) {
        let namespace = "expiry";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        // A zero ttl means this reservation is already expired by the time we look at it
        // again.
        let first = rate_limiter
            .reserve(namespace, &ctx, 8, Some(Duration::ZERO), false)
            .await
            .unwrap();
        assert!(!first.limited);

        // The first reservation is already expired, so another 8 fits again: 0 + 0 + 8 <= 10
        let second = rate_limiter
            .reserve(namespace, &ctx, 8, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!second.limited);
    }

    async fn reserve_denies_when_amount_alone_exceeds_max_value(rate_limiter: &mut TestsLimiter) {
        // Admission checks the raw requested amount - a request that could never fit in the
        // counter at all is denied outright, regardless of `max_fraction` (default here).
        let namespace = "over_max_value";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        let result = rate_limiter
            .reserve(namespace, &ctx, 1000, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(result.limited);
        assert!(result.reservation_id.is_none());
    }

    async fn reserve_admits_when_amount_fits(rate_limiter: &mut TestsLimiter) {
        let namespace = "amount_fits";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        let result = rate_limiter
            .reserve(namespace, &ctx, 10, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!result.limited);
        assert!(result.reservation_id.is_some());
    }

    async fn reserve_with_amount_zero_creates_no_reservation_entry(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "amount_zero";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        let result = rate_limiter
            .reserve(namespace, &ctx, 0, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(!result.limited);
        let reservation_id = result.reservation_id.expect("should be admitted");

        // Nothing was actually held, so there's nothing to release.
        let commit = rate_limiter
            .commit_reservation(namespace, &ctx, &reservation_id, 0)
            .await
            .unwrap();
        assert!(!commit.reservation_released);
    }

    async fn reserve_denies_amount_zero_when_already_over_limit(rate_limiter: &mut TestsLimiter) {
        // Checking 0 must still reflect the counter's real state, matching how
        // `is_within_limits`/`check_and_update` never special-case a zero delta: a counter
        // already over its limit (e.g. from a commit that overshot it) stays "limited" even
        // for a reservation that asks for nothing extra.
        let namespace = "amount_zero_over_limit";
        let limit = Limit::new(namespace, 10, 60, vec![], Vec::<Expression>::default());
        rate_limiter.add_limit(&limit).await;

        let ctx = Context::default();

        rate_limiter
            .update_counters(namespace, &ctx, 15)
            .await
            .unwrap();

        let result = rate_limiter
            .reserve(namespace, &ctx, 0, Some(Duration::from_secs(30)), false)
            .await
            .unwrap();
        assert!(result.limited);
    }

    // RFC 0021's motivating scenario against the shared, Redis-backed path: N concurrent
    // in-flight requests racing to reserve capacity against one limit. Unlike the local
    // (in-memory) path, this is expected to already be safe regardless of threading, since
    // the entire check-and-write happens inside one atomic Lua script executed by Redis.
    #[cfg(feature = "redis_storage")]
    #[test]
    #[serial]
    fn concurrent_reserve_calls_never_exceed_the_limit_with_sync_redis() {
        use limitador::storage::redis::RedisStorage;
        use limitador::RateLimiter;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        const MAX_VALUE: u64 = 50;
        const AMOUNT: u64 = 5;
        const CONCURRENT_REQUESTS: usize = 20;

        let storage = RedisStorage::default();
        storage.clear().unwrap();
        let rl = Arc::new(RateLimiter::new_with_storage(Box::new(storage)));
        let namespace = "sync_redis_race";
        let limit = Limit::new(
            namespace,
            MAX_VALUE,
            60,
            vec![],
            Vec::<Expression>::default(),
        );
        rl.add_limit(limit);
        let ns: limitador::limit::Namespace = namespace.into();

        let admitted_count = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..CONCURRENT_REQUESTS {
                let rl = rl.clone();
                let ns = ns.clone();
                let admitted_count = admitted_count.clone();
                s.spawn(move || {
                    let ctx = Context::default();
                    let res = rl
                        .reserve(&ns, &ctx, AMOUNT, Some(Duration::from_secs(30)), false)
                        .unwrap();
                    if !res.limited {
                        admitted_count.fetch_add(1, Ordering::SeqCst);
                    }
                });
            }
        });

        let expected = (MAX_VALUE / AMOUNT) as usize;
        assert_eq!(admitted_count.load(Ordering::SeqCst), expected);
    }

    #[cfg(feature = "redis_storage")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[serial]
    async fn concurrent_reserve_calls_never_exceed_the_limit_with_async_redis() {
        use limitador::storage::redis::AsyncRedisStorage;
        use limitador::AsyncRateLimiter;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        const MAX_VALUE: u64 = 50;
        const AMOUNT: u64 = 5;
        const CONCURRENT_REQUESTS: usize = 20;

        let storage = AsyncRedisStorage::new("redis://127.0.0.1:6379")
            .await
            .expect("We need a Redis running locally");
        storage.clear().await.unwrap();
        let rl = Arc::new(AsyncRateLimiter::new_with_storage(Box::new(storage)));
        let namespace = "async_redis_race";
        let limit = Limit::new(
            namespace,
            MAX_VALUE,
            60,
            vec![],
            Vec::<Expression>::default(),
        );
        rl.add_limit(limit);
        let ns: limitador::limit::Namespace = namespace.into();

        let admitted_count = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(CONCURRENT_REQUESTS);
        for _ in 0..CONCURRENT_REQUESTS {
            let rl = rl.clone();
            let ns = ns.clone();
            let admitted_count = admitted_count.clone();
            handles.push(tokio::spawn(async move {
                let ctx = Context::default();
                let res = rl
                    .reserve(&ns, &ctx, AMOUNT, Some(Duration::from_secs(30)), false)
                    .await
                    .unwrap();
                if !res.limited {
                    admitted_count.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }

        let expected = (MAX_VALUE / AMOUNT) as usize;
        assert_eq!(admitted_count.load(Ordering::SeqCst), expected);
    }

    #[allow(dead_code)]
    async fn distributed_rate_limited<Fut>(create_distributed_limiters: fn(count: usize) -> Fut)
    where
        Fut: Future<Output = Vec<TestsLimiter>>,
    {
        let rate_limiters = create_distributed_limiters(2).await;
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        for rate_limiter in rate_limiters.iter() {
            rate_limiter.add_limit(&limit).await;
        }

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req_method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        let ctx = values.into();

        for i in 0..max_hits {
            // Alternate between the two rate limiters
            let rate_limiter = rate_limiters.get((i % 2) as usize).unwrap();
            assert!(
                !rate_limiter
                    .is_rate_limited(namespace, &ctx, 1)
                    .await
                    .unwrap()
                    .limited,
                "Must not be limited after {i}"
            );
            rate_limiter
                .update_counters(namespace, &ctx, 1)
                .await
                .unwrap();
        }

        // eventually it should get rate limited...
        assert!(eventually(
            Duration::from_secs(5),
            Duration::from_millis(100),
            || async {
                let rate_limiter = rate_limiters.first().unwrap();
                rate_limiter
                    .is_rate_limited(namespace, &ctx, 1)
                    .await
                    .unwrap()
                    .limited
            }
        )
        .await
        .unwrap());
    }
}
