extern crate limitador;

use limitador::limit::Limit;
use limitador::RateLimiter;
use std::collections::{HashMap, HashSet};

#[test]
fn add_a_limit() {
    let limit = Limit::new(
        "test_namespace",
        10,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

    let mut expected_result = HashSet::new();
    expected_result.insert(limit);

    assert_eq!(
        rate_limiter.get_limits("test_namespace").unwrap(),
        expected_result
    )
}

#[test]
fn add_limit_without_vars() {
    let limit = Limit::new(
        "test_namespace",
        10,
        60,
        vec!["req.method == GET"],
        Vec::<String>::new(),
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

    let mut expected_result = HashSet::new();
    expected_result.insert(limit);

    assert_eq!(
        rate_limiter.get_limits("test_namespace").unwrap(),
        expected_result
    )
}

#[test]
fn delete_limit() {
    let limit = Limit::new(
        "test_namespace",
        10,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

    rate_limiter.delete_limit(&limit).unwrap();

    assert!(rate_limiter
        .get_limits("test_namespace")
        .unwrap()
        .is_empty())
}

#[test]
fn delete_limit_also_deletes_associated_counters() {
    let namespace = "test_namespace";
    let limit = Limit::new(namespace, 10, 60, vec!["req.method == GET"], vec!["app_id"]);

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

    let mut values = HashMap::new();
    values.insert("req.method".to_string(), "GET".to_string());
    values.insert("app_id".to_string(), "1".to_string());
    rate_limiter.update_counters(namespace, &values, 1).unwrap();

    rate_limiter.delete_limit(&limit).unwrap();

    assert!(rate_limiter.get_counters(namespace).unwrap().is_empty())
}

#[test]
fn add_several_limits_in_the_same_namespace() {
    let namespace = "test_namespace";

    let limit_1 = Limit::new(
        namespace,
        10,
        60,
        vec!["req.method == POST"],
        vec!["app_id"],
    );

    let limit_2 = Limit::new(namespace, 5, 60, vec!["req.method == GET"], vec!["app_id"]);

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit_1.clone()).unwrap();
    rate_limiter.add_limit(limit_2.clone()).unwrap();

    let mut expected_result = HashSet::new();
    expected_result.insert(limit_1);
    expected_result.insert(limit_2);

    assert_eq!(rate_limiter.get_limits(namespace).unwrap(), expected_result)
}

#[test]
fn delete_limits_of_a_namespace() {
    let namespace = "test_namespace";
    let mut rate_limiter = RateLimiter::new();

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
    .for_each(|limit| rate_limiter.add_limit(limit.clone()).unwrap());

    rate_limiter.delete_limits(namespace).unwrap();

    assert!(rate_limiter.get_limits(namespace).unwrap().is_empty())
}

#[test]
fn delete_limits_of_a_namespace_also_deletes_counters() {
    let namespace = "test_namespace";
    let mut rate_limiter = RateLimiter::new();
    let limit = Limit::new(namespace, 5, 60, vec!["req.method == GET"], vec!["app_id"]);

    rate_limiter.add_limit(limit).unwrap();

    let mut values = HashMap::new();
    values.insert("req.method".to_string(), "GET".to_string());
    values.insert("app_id".to_string(), "1".to_string());
    rate_limiter.update_counters(namespace, &values, 1).unwrap();

    rate_limiter.delete_limits(namespace).unwrap();

    assert!(rate_limiter.get_counters(namespace).unwrap().is_empty())
}

#[test]
fn rate_limited() {
    let namespace = "test_namespace";
    let max_hits = 3;
    let limit = Limit::new(
        namespace,
        max_hits,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

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

#[test]
fn rate_limited_with_delta_higher_than_one() {
    let namespace = "test_namespace";
    let limit = Limit::new(namespace, 10, 60, vec!["req.method == GET"], vec!["app_id"]);

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

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

#[test]
fn takes_into_account_only_vars_of_the_limits() {
    let namespace = "test_namespace";
    let max_hits = 3;
    let limit = Limit::new(
        namespace,
        max_hits,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

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

#[test]
fn check_rate_limited_and_update() {
    let namespace = "test_namespace";
    let max_hits = 3;

    let limit = Limit::new(
        namespace,
        max_hits,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

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

#[test]
fn get_counters() {
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

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

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

    for counter_data in counters {
        let app_id = counter_data.0.set_variables().get("app_id").unwrap();

        match app_id.as_str() {
            "1" => assert_eq!(counter_data.1, max_hits - hits_app_1),
            "2" => assert_eq!(counter_data.1, max_hits - hits_app_2),
            _ => panic!("Unexpected app ID"),
        }
    }
}

#[test]
fn get_counters_can_return_empty_list() {
    // There's a limit, but no counters. The result should be empty.

    let limit = Limit::new(
        "test_namespace",
        10,
        60,
        vec!["req.method == GET"],
        vec!["app_id"],
    );

    let mut rate_limiter = RateLimiter::new();
    rate_limiter.add_limit(limit.clone()).unwrap();

    assert!(rate_limiter
        .get_counters("test_namespace")
        .unwrap()
        .is_empty())
}
