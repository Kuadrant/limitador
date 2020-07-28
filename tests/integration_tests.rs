macro_rules! test_with_all_storage_impls {
    // This macro uses the "paste" crate to define the names of the functions.
    // Also, the Redis tests cannot be run in parallel. The "serial" tag from
    // the "serial-test" crate takes care of that.
    ($function:ident) => {
        paste::item! {
            #[test]
            fn [<$function _in_memory_storage>]() {
                let mut rate_limiter =
                    RateLimiter::new_with_storage(Box::new(InMemoryStorage::default()));
                $function(&mut rate_limiter);
            }

            #[test]
            #[serial]
            fn [<$function _with_redis>]() {
                let mut rate_limiter = RateLimiter::new_with_storage(
                    Box::new(RedisStorage::default())
                );
                clean_redis_test_db();
                $function(&mut rate_limiter);
            }

            #[test]
            fn [<$function _with_wasm_storage>]() {
                let mut rate_limiter = RateLimiter::new_with_storage(
                    Box::new(WasmStorage::new(Box::new(TestClock {})))
                );
                $function(&mut rate_limiter);
            }
        }
    };
}

#[cfg(test)]
mod test {
    extern crate limitador;
    use self::limitador::storage::wasm::Clock;
    use limitador::limit::Limit;
    use limitador::storage::in_memory::InMemoryStorage;
    use limitador::storage::redis::RedisStorage;
    use limitador::storage::wasm::WasmStorage;
    use limitador::RateLimiter;
    use serial_test::serial;
    use std::collections::{HashMap, HashSet};
    use std::thread::sleep;
    use std::time::Duration;
    use std::time::SystemTime;

    // This is only needed for the WASM-compatible storage.
    pub struct TestClock {}
    impl Clock for TestClock {
        fn get_current_time(&self) -> SystemTime {
            SystemTime::now()
        }
    }

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
    test_with_all_storage_impls!(takes_into_account_only_vars_of_the_limits);
    test_with_all_storage_impls!(is_rate_limited_returns_false_when_no_limits_in_namespace);
    test_with_all_storage_impls!(is_rate_limited_returns_false_when_no_matching_limits);
    test_with_all_storage_impls!(check_rate_limited_and_update);
    test_with_all_storage_impls!(get_counters);
    test_with_all_storage_impls!(get_counters_returns_empty_when_no_limits_in_namespace);
    test_with_all_storage_impls!(get_counters_returns_empty_when_no_counters_in_namespace);
    test_with_all_storage_impls!(get_counters_does_not_return_expired_ones);

    fn add_a_limit(rate_limiter: &mut RateLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut expected_result = HashSet::new();
        expected_result.insert(limit);

        assert_eq!(
            rate_limiter.get_limits("test_namespace").unwrap(),
            expected_result
        )
    }

    fn add_limit_without_vars(rate_limiter: &mut RateLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req.method == GET"],
            Vec::<String>::new(),
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut expected_result = HashSet::new();
        expected_result.insert(limit);

        assert_eq!(
            rate_limiter.get_limits("test_namespace").unwrap(),
            expected_result
        )
    }

    fn add_several_limits_in_the_same_namespace(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";

        let limit_1 = Limit::new(
            namespace,
            10,
            60,
            vec!["req.method == POST"],
            vec!["app_id"],
        );

        let limit_2 = Limit::new(namespace, 5, 60, vec!["req.method == GET"], vec!["app_id"]);

        rate_limiter.add_limit(&limit_1).unwrap();
        rate_limiter.add_limit(&limit_2).unwrap();

        let mut expected_result = HashSet::new();
        expected_result.insert(limit_1);
        expected_result.insert(limit_2);

        assert_eq!(rate_limiter.get_limits(namespace).unwrap(), expected_result)
    }

    fn delete_limit(rate_limiter: &mut RateLimiter) {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        rate_limiter.delete_limit(&limit).unwrap();

        assert!(rate_limiter
            .get_limits("test_namespace")
            .unwrap()
            .is_empty())
    }

    fn delete_limit_also_deletes_associated_counters(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 10, 60, vec!["req.method == GET"], vec!["app_id"]);

        rate_limiter.add_limit(&limit).unwrap();

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter.update_counters(namespace, &values, 1).unwrap();

        rate_limiter.delete_limit(&limit).unwrap();

        assert!(rate_limiter.get_counters(namespace).unwrap().is_empty())
    }

    fn get_limits_returns_empty_if_no_limits_in_namespace(rate_limiter: &mut RateLimiter) {
        assert!(rate_limiter
            .get_counters("test_namespace")
            .unwrap()
            .is_empty())
    }

    fn delete_limits_of_a_namespace(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";

        [
            Limit::new(
                namespace,
                10,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            ),
            Limit::new(namespace, 5, 60, vec!["req.method == GET"], vec!["app_id"]),
        ]
        .iter()
        .for_each(|limit| rate_limiter.add_limit(&limit).unwrap());

        rate_limiter.delete_limits(namespace).unwrap();

        assert!(rate_limiter.get_limits(namespace).unwrap().is_empty())
    }

    fn delete_limits_does_not_delete_limits_from_other_namespaces(rate_limiter: &mut RateLimiter) {
        let namespace1 = "test_namespace_1";
        let namespace2 = "test_namespace_2";

        rate_limiter
            .add_limit(&Limit::new(namespace1, 10, 60, vec!["x == 10"], vec!["z"]))
            .unwrap();
        rate_limiter
            .add_limit(&Limit::new(namespace2, 5, 60, vec!["x == 10"], vec!["z"]))
            .unwrap();

        rate_limiter.delete_limits(namespace1).unwrap();

        assert!(rate_limiter.get_limits(namespace1).unwrap().is_empty());
        assert_eq!(rate_limiter.get_limits(namespace2).unwrap().len(), 1);
    }

    fn delete_limits_of_a_namespace_also_deletes_counters(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 5, 60, vec!["req.method == GET"], vec!["app_id"]);

        rate_limiter.add_limit(&limit).unwrap();

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter.update_counters(namespace, &values, 1).unwrap();

        rate_limiter.delete_limits(namespace).unwrap();

        assert!(rate_limiter.get_counters(namespace).unwrap().is_empty())
    }

    fn delete_limits_of_an_empty_namespace_does_nothing(rate_limiter: &mut RateLimiter) {
        rate_limiter.delete_limits("test_namespace").unwrap()
    }

    fn rate_limited(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for _ in 0..max_hits {
            assert_eq!(
                false,
                rate_limiter.is_rate_limited(namespace, &values, 1).unwrap()
            );
            rate_limiter.update_counters(namespace, &values, 1).unwrap();
        }
        assert_eq!(
            true,
            rate_limiter.is_rate_limited(namespace, &values, 1).unwrap()
        );
    }

    fn rate_limited_with_delta_higher_than_one(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 10, 60, vec!["req.method == GET"], vec!["app_id"]);

        rate_limiter.add_limit(&limit).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        // Report 5 hits twice. The limit is 10, so the first limited call should be
        // the third one.
        for _ in 0..2 {
            assert_eq!(
                false,
                rate_limiter.is_rate_limited(namespace, &values, 5).unwrap()
            );
            rate_limiter.update_counters(namespace, &values, 5).unwrap();
        }
        assert_eq!(
            true,
            rate_limiter.is_rate_limited(namespace, &values, 1).unwrap()
        );
    }

    fn takes_into_account_only_vars_of_the_limits(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;
        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for i in 0..max_hits {
            // Add an extra value that does not apply to the limit on each
            // iteration. It should not affect.
            values.insert("does_not_apply".to_string(), i.to_string());

            assert_eq!(
                false,
                rate_limiter.is_rate_limited(namespace, &values, 1).unwrap()
            );
            rate_limiter.update_counters(namespace, &values, 1).unwrap();
        }
        assert_eq!(
            true,
            rate_limiter.is_rate_limited(namespace, &values, 1).unwrap()
        );
    }

    fn is_rate_limited_returns_false_when_no_limits_in_namespace(rate_limiter: &mut RateLimiter) {
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());

        assert_eq!(
            rate_limiter
                .is_rate_limited("test_namespace", &values, 1)
                .unwrap(),
            false
        );
    }

    fn is_rate_limited_returns_false_when_no_matching_limits(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";

        let limit = Limit::new(
            namespace,
            0, // So reporting 1 more would not be allowed
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        // Notice that does not match because the method is "POST".
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "POST".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        assert_eq!(
            rate_limiter.is_rate_limited(namespace, &values, 1).unwrap(),
            false
        );
    }

    fn check_rate_limited_and_update(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let max_hits = 3;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());

        for _ in 0..max_hits {
            assert_eq!(
                false,
                rate_limiter
                    .check_rate_limited_and_update(namespace, &values, 1)
                    .unwrap()
            );
        }

        assert_eq!(
            true,
            rate_limiter
                .check_rate_limited_and_update(namespace, &values, 1)
                .unwrap()
        );
    }

    fn get_counters(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let max_hits = 10;
        let hits_app_1 = 1;
        let hits_app_2 = 5;

        let limit = Limit::new(
            namespace,
            max_hits,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter
            .update_counters(namespace, &values, hits_app_1)
            .unwrap();

        values.insert("app_id".to_string(), "2".to_string());
        rate_limiter
            .update_counters(namespace, &values, hits_app_2)
            .unwrap();

        let counters = rate_limiter.get_counters(namespace).unwrap();

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

    fn get_counters_returns_empty_when_no_limits_in_namespace(rate_limiter: &mut RateLimiter) {
        assert!(rate_limiter
            .get_counters("test_namespace")
            .unwrap()
            .is_empty())
    }

    fn get_counters_returns_empty_when_no_counters_in_namespace(rate_limiter: &mut RateLimiter) {
        // There's a limit, but no counters. The result should be empty.

        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        assert!(rate_limiter
            .get_counters("test_namespace")
            .unwrap()
            .is_empty())
    }

    fn get_counters_does_not_return_expired_ones(rate_limiter: &mut RateLimiter) {
        let namespace = "test_namespace";
        let limit_time = 1;

        let limit = Limit::new(
            "test_namespace",
            10,
            limit_time,
            vec!["req.method == GET"],
            vec!["app_id"],
        );

        rate_limiter.add_limit(&limit).unwrap();

        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());
        rate_limiter.update_counters(namespace, &values, 1).unwrap();

        // Give it some extra time to expire
        sleep(Duration::from_secs(limit_time + 1));

        assert!(rate_limiter.get_counters(namespace).unwrap().is_empty());
    }

    fn clean_redis_test_db() {
        let redis_client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let mut con = redis_client.get_connection().unwrap();
        redis::cmd("FLUSHDB").execute(&mut con);
    }
}
