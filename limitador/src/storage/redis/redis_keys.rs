// To use Redis cluster and some Redis proxies, all the keys used in a
// command or in a pipeline need to be sharded to the same server.
// To help with that, Redis uses "hash tags". When a key contains "{" and
// "}" only what's inside them is hashed.
// To ensure that all the keys involved in a given operation belong to the
// same shard, we can shard by namespace.
// When there are multiple pairs of "{" and "}" only the first one is taken
// into account. Ref: https://redis.io/topics/cluster-spec (key hash tags).
// Reminder: in format!(), "{" is escaped with "{{".

use crate::counter::Counter;
use crate::limit::{Limit, Namespace};

pub fn key_for_namespaces_set() -> String {
    "namespaces_set".to_string()
}

pub fn key_for_limits_of_namespace(namespace: &Namespace) -> String {
    format!("limits_of_namespace:{{{}}}", namespace.as_ref())
}

pub fn key_for_counter(counter: &Counter) -> String {
    format!(
        "namespace:{{{}}},counter:{}",
        counter.namespace().as_ref(),
        serde_json::to_string(counter).unwrap()
    )
}

pub fn key_for_counters_of_limit(limit: &Limit) -> String {
    format!(
        "namespace:{{{}}},counters_of_limit:{}",
        limit.namespace().as_ref(),
        serde_json::to_string(limit).unwrap()
    )
}

pub fn counter_from_counter_key(key: &str) -> Counter {
    let counter_prefix = "counter:";
    let start_pos_counter = key.find(counter_prefix).unwrap() + counter_prefix.len();

    serde_json::from_str(&key[start_pos_counter..]).unwrap()
}
