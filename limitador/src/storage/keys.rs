// To use Redis cluster and some Redis proxies, all the keys used in a
// command or in a pipeline need to be sharded to the same server.
// To help with that, Redis uses "hash tags". When a key contains "{" and
// "}" only what's inside them is hashed.
// To ensure that all the keys involved in a given operation belong to the
// same shard, we can shard by namespace.
// When there are multiple pairs of "{" and "}" only the first one is taken
// into account. Ref: https://redis.io/topics/cluster-spec (key hash tags).
// Reminder: in format!(), "{" is escaped with "{{".

// Note: keep in mind that what's described above is the default in Redis, when
// reusing this module for other storage implementations make sure that using
// "{}" for sharding applies.

use crate::counter::Counter;
use crate::limit::Limit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CounterKey {
    limit: Limit,
    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    set_variables: BTreeMap<String, String>,
}

pub fn key_for_counter(counter: &Counter) -> String {
    let l = counter.limit();

    let mut set_variables: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in counter.set_variables() {
        set_variables.insert(k.clone(), v.clone());
    }

    let counter_key = CounterKey {
        limit: l.clone(),
        set_variables,
    };
    format!(
        "namespace:{{{}}},counter:{}",
        counter.namespace().as_ref(),
        serde_json::to_string(&counter_key).unwrap()
    )
}

pub fn key_for_counters_of_limit(limit: &Limit) -> String {
    format!(
        "namespace:{{{}}},counters_of_limit:{}",
        limit.namespace().as_ref(),
        serde_json::to_string(limit).unwrap()
    )
}

pub fn counter_from_counter_key(key: &str, limit: &Limit) -> Counter {
    let counter_prefix = "counter:";
    let start_pos_counter = key.find(counter_prefix).unwrap() + counter_prefix.len();

    let mut counter: Counter = serde_json::from_str(&key[start_pos_counter..]).unwrap();

    if !counter.update_to_limit(limit) {
        // this means some kind of data corruption _or_ most probably
        // an out of sync `impl PartialEq for Limit` vs `pub fn key_for_counter(counter: &Counter) -> String`
        panic!(
            "Failed to rebuild Counter's Limit from the provided Limit: {:?} vs {:?}",
            counter.limit(),
            limit
        )
    }
    counter
}

#[cfg(test)]
mod tests {
    use crate::counter::Counter;
    use crate::storage::keys::{key_for_counter, key_for_counters_of_limit};
    use crate::Limit;
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn test_key_for_counter() {
        let mut values = HashMap::new();
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "1".to_string());

        let mut counter = Counter::new(
            Limit::new(
                "example.com",
                10,
                60,
                vec!["req.method == 'GET'"],
                vec!["app_id"],
            ),
            values,
        );

        assert_eq!(
            "namespace:{example.com},counter:{\"limit\":{\"namespace\":\"example.com\",\"seconds\":60,\"conditions\":[\"req.method == \\\"GET\\\"\"],\"variables\":[\"app_id\"]},\"set_variables\":{\"app_id\":\"1\"}}",
            key_for_counter(&counter));

        // Even if we the the remaining and ttl, the counter key should stay the same.
        counter.set_remaining(9);
        counter.set_expires_in(Duration::from_secs(59));
        assert_eq!(
            "namespace:{example.com},counter:{\"limit\":{\"namespace\":\"example.com\",\"seconds\":60,\"conditions\":[\"req.method == \\\"GET\\\"\"],\"variables\":[\"app_id\"]},\"set_variables\":{\"app_id\":\"1\"}}",
            key_for_counter(&counter));
    }

    #[test]
    fn key_for_limit_format() {
        let limit = Limit::new(
            "example.com",
            10,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );
        assert_eq!(
            "namespace:{example.com},counters_of_limit:{\"namespace\":\"example.com\",\"seconds\":60,\"conditions\":[\"req.method == \\\"GET\\\"\"],\"variables\":[\"app_id\"]}",
            key_for_counters_of_limit(&limit))
    }
}
