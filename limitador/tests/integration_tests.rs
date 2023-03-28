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
                    RateLimiter::new_with_storage(Box::new(InMemoryStorage::default()));
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[cfg(feature = "redis_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_redis>]() {
                let storage = RedisStorage::default();
                storage.clear().unwrap();
                let rate_limiter = RateLimiter::new_with_storage(
                    Box::new(storage)
                );
                RedisStorage::default().clear().unwrap();
                $function(&mut TestsLimiter::new_from_blocking_impl(rate_limiter)).await;
            }

            #[tokio::test]
            async fn [<$function _with_wasm_storage>]() {
                let rate_limiter = RateLimiter::new_with_storage(
                    Box::new(WasmStorage::new(Box::new(TestClock {})))
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
                AsyncRedisStorage::new("redis://127.0.0.1:6379").await.expect("We need a Redis running locally").clear().await.unwrap();
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }

            #[cfg(feature = "infinispan_storage")]
            #[tokio::test]
            #[serial]
            async fn [<$function _with_infinispan>]() {
                let storage = InfinispanStorageBuilder::new(
                    "http://127.0.0.1:11222", "username", "password"
                ).build().await;
                storage.clear().await.unwrap();
                let rate_limiter = AsyncRateLimiter::new_with_storage(
                    Box::new(storage)
                );
                $function(&mut TestsLimiter::new_from_async_impl(rate_limiter)).await;
            }
        }
    };
}

mod helpers;

#[cfg(test)]
mod test {
    extern crate limitador;

    // To be able to pass the tests without Redis
    cfg_if::cfg_if! {
        if #[cfg(feature = "redis_storage")] {
            use limitador::storage::redis::AsyncRedisStorage;
            use limitador::storage::redis::RedisStorage;

            use limitador::AsyncRateLimiter;
            use serial_test::serial;
            use crate::test::limitador::storage::CounterStorage;
            use crate::test::limitador::storage::AsyncCounterStorage;
        }
    }

    cfg_if::cfg_if! {
       if #[cfg(feature = "infinispan_storage")] {
           use limitador::storage::infinispan::InfinispanStorageBuilder;
       }
    }

    use self::limitador::counter::Counter;
    use self::limitador::storage::wasm::Clock;
    use self::limitador::RateLimiter;
    use crate::helpers::tests_limiter::*;
    use limitador::limit::Limit;
    use limitador::storage::in_memory::InMemoryStorage;
    use limitador::storage::wasm::WasmStorage;
    use std::collections::{HashMap, HashSet};
    use std::thread::sleep;
    use std::time::{Duration, SystemTime};

    // This is only needed for the WASM-compatible storage.
    pub struct TestClock {}
    impl Clock for TestClock {
        fn get_current_time(&self) -> SystemTime {
            SystemTime::now()
        }
    }

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

    // All these functions need to use async/await. That's needed to support
    // both the sync and the async implementations of the rate limiter.

    async fn get_namespaces(rate_limiter: &mut TestsLimiter) {
        let limits = vec![
            Limit::new(
                "first_namespace",
                10,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
            ),
            Limit::new(
                "second_namespace",
                20,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let lim2 = Limit::new(
            "second_namespace",
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        for limit in vec![&lim1, &lim2].iter() {
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            Vec::<String>::new(),
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
            vec!["req.method == 'POST'"],
            vec!["app_id"],
        );

        let limit_2 = Limit::new(
            namespace,
            5,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, 1)
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
                vec!["req.method == 'POST'"],
                vec!["app_id"],
            ),
            Limit::new(
                namespace,
                5,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
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
                vec!["x == '10'"],
                vec!["z"],
            ))
            .await;
        rate_limiter
            .add_limit(&Limit::new(namespace2, 5, 60, vec!["x == '10'"], vec!["z"]))
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, 1)
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for _ in 0..max_hits {
            assert!(!rate_limiter
                .is_rate_limited(namespace, &values, 1)
                .await
                .unwrap());
            rate_limiter
                .update_counters(namespace, &values, 1)
                .await
                .unwrap();
        }
        assert!(rate_limiter
            .is_rate_limited(namespace, &values, 1)
            .await
            .unwrap());
    }

    async fn rate_limited_with_delta_higher_than_one(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            10,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        // Report 5 hits twice. The limit is 10, so the first limited call should be
        // the third one.
        for _ in 0..2 {
            assert!(!rate_limiter
                .is_rate_limited(namespace, &values, 5)
                .await
                .unwrap());
            rate_limiter
                .update_counters(namespace, &values, 5)
                .await
                .unwrap();
        }
        assert!(rate_limiter
            .is_rate_limited(namespace, &values, 1)
            .await
            .unwrap());
    }

    async fn rate_limited_with_delta_higher_than_max(rate_limiter: &mut TestsLimiter) {
        let max = 10;
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            max,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        assert!(rate_limiter
            .is_rate_limited(namespace, &values, max + 1)
            .await
            .unwrap())
    }

    async fn takes_into_account_only_vars_of_the_limits(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for i in 0..max_hits {
            // Add an extra value that does not apply to the limit on each
            // iteration. It should not affect.
            values.insert("does_not_apply".to_string(), i.to_string());

            assert!(!rate_limiter
                .is_rate_limited(namespace, &values, 1)
                .await
                .unwrap());
            rate_limiter
                .update_counters(namespace, &values, 1)
                .await
                .unwrap();
        }
        assert!(rate_limiter
            .is_rate_limited(namespace, &values, 1)
            .await
            .unwrap());
    }

    async fn is_rate_limited_returns_false_when_no_limits_in_namespace(
        rate_limiter: &mut TestsLimiter,
    ) {
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());

        assert!(!rate_limiter
            .is_rate_limited("test_namespace", &values, 1)
            .await
            .unwrap());
    }

    async fn is_rate_limited_returns_false_when_no_matching_limits(
        rate_limiter: &mut TestsLimiter,
    ) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        // Notice that does not match because the method is "POST".
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "POST".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        assert!(!rate_limiter
            .is_rate_limited(namespace, &values, 1)
            .await
            .unwrap());
    }

    async fn is_rate_limited_applies_limit_if_its_unconditional(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            Vec::<String>::new(), // unconditional
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());

        assert!(rate_limiter
            .is_rate_limited(namespace, &values, 1)
            .await
            .unwrap());
    }

    async fn check_rate_limited_and_update(rate_limiter: &mut TestsLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for _ in 0..max_hits {
            assert!(
                !rate_limiter
                    .check_rate_limited_and_update(namespace, &values, 1, false)
                    .await
                    .unwrap()
                    .limited
            );
        }

        assert!(
            rate_limiter
                .check_rate_limited_and_update(namespace, &values, 1, false)
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for hit in 0..max_hits {
            let result = rate_limiter
                .check_rate_limited_and_update(namespace, &values, 1, true)
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
            .check_rate_limited_and_update(namespace, &values, 1, true)
            .await
            .unwrap();
        assert!(result.limited);
        assert_eq!(result.counters.len(), 1);

        for counter in result.counters.iter() {
            if let Some(ttl) = counter.expires_in() {
                assert!(ttl.as_secs() <= 60);
            }
            assert_eq!(counter.remaining().unwrap(), -1);
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());
        // Does not match the limit defined
        values.insert("req.method".to_string(), "POST".to_string());

        assert!(
            !rate_limiter
                .check_rate_limited_and_update(namespace, &values, 1, false)
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
            Vec::<String>::new(), // unconditional
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("app_id".to_string(), "test_app_id".to_string());

        assert!(
            rate_limiter
                .check_rate_limited_and_update(namespace, &values, 1, false)
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, hits_app_1)
            .await
            .unwrap();

        values.insert("app_id".to_string(), "2".to_string());
        rate_limiter
            .update_counters(namespace, &values, hits_app_2)
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            "test_namespace",
            10,
            limit_time,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, 1)
            .await
            .unwrap();

        // Give it some extra time to expire
        sleep(Duration::from_secs(limit_time + 1));

        assert!(rate_limiter
            .get_counters(namespace)
            .await
            .unwrap()
            .is_empty());
    }

    async fn configure_with_creates_the_given_limits(rate_limiter: &mut TestsLimiter) {
        let first_limit = Limit::new(
            "first_namespace",
            10,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let second_limit = Limit::new(
            "second_namespace",
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).await;

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, hits_to_report)
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let limit_to_be_deleted = Limit::new(
            namespace,
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let limit_update = Limit::new(
            namespace,
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let limit_2 = Limit::new(
            namespace,
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let mut limit_3 = Limit::new(
            namespace,
            20,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
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
}
