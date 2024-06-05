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

pub fn key_for_counter(counter: &Counter) -> String {
    if counter.remaining().is_some() || counter.expires_in().is_some() {
        format!(
            "{},counter:{}",
            prefix_for_namespace(counter.namespace().as_ref()),
            serde_json::to_string(&counter.key()).unwrap()
        )
    } else {
        format!(
            "{},counter:{}",
            prefix_for_namespace(counter.namespace().as_ref()),
            serde_json::to_string(counter).unwrap()
        )
    }
}

pub fn key_for_counters_of_limit(limit: &Limit) -> String {
    format!(
        "namespace:{{{}}},counters_of_limit:{}",
        limit.namespace().as_ref(),
        serde_json::to_string(limit).unwrap()
    )
}

pub fn prefix_for_namespace(namespace: &str) -> String {
    format!("namespace:{{{namespace}}},")
}

pub fn counter_from_counter_key(key: &str, limit: &Limit) -> Counter {
    let mut counter = partial_counter_from_counter_key(key);
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

pub fn partial_counter_from_counter_key(key: &str) -> Counter {
    let namespace_prefix = "namespace:";
    let counter_prefix = ",counter:";

    // Find the start position of the counter portion
    let start_pos_namespace = key
        .find(namespace_prefix)
        .expect("Namespace not found in the key");
    let start_pos_counter = key[start_pos_namespace..]
        .find(counter_prefix)
        .expect("Counter not found in the key")
        + start_pos_namespace
        + counter_prefix.len();

    // Extract counter JSON substring and deserialize it
    let counter_str = &key[start_pos_counter..];
    let counter: Counter =
        serde_json::from_str(counter_str).expect("Failed to deserialize counter JSON");
    counter
}

#[cfg(test)]
mod tests {
    use super::{
        key_for_counter, key_for_counters_of_limit, partial_counter_from_counter_key,
        prefix_for_namespace,
    };
    use crate::counter::Counter;
    use crate::Limit;
    use std::collections::HashMap;
    use std::time::Duration;

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

    #[test]
    fn counter_key_and_counter_are_symmetric() {
        let namespace = "ns_counter:";
        let limit = Limit::new(namespace, 1, 1, vec!["req.method == 'GET'"], vec!["app_id"]);
        let counter = Counter::new(limit.clone(), HashMap::default());
        let raw = key_for_counter(&counter);
        assert_eq!(counter, partial_counter_from_counter_key(&raw));
        let prefix = prefix_for_namespace(namespace);
        assert_eq!(&raw[0..prefix.len()], &prefix);
    }

    #[test]
    fn counter_key_does_not_include_transient_state() {
        let namespace = "ns_counter:";
        let limit = Limit::new(namespace, 1, 1, vec!["req.method == 'GET'"], vec!["app_id"]);
        let counter = Counter::new(limit.clone(), HashMap::default());
        let mut other = counter.clone();
        other.set_remaining(123);
        other.set_expires_in(Duration::from_millis(456));
        assert_eq!(key_for_counter(&counter), key_for_counter(&other));
    }
}

#[cfg(feature = "disk_storage")]
pub mod bin {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    use crate::counter::Counter;
    use crate::limit::Limit;

    #[derive(PartialEq, Debug, Serialize, Deserialize)]
    struct CounterKey<'a> {
        ns: &'a str,
        seconds: u64,
        conditions: Vec<String>,
        variables: Vec<(&'a str, &'a str)>,
    }

    impl<'a> From<&'a Counter> for CounterKey<'a> {
        fn from(counter: &'a Counter) -> Self {
            let set = counter.limit().conditions();
            let mut conditions = Vec::with_capacity(set.len());
            for cond in &set {
                conditions.push(cond.clone());
            }
            conditions.sort();

            CounterKey {
                ns: counter.namespace().as_ref(),
                seconds: counter.window().as_secs(),
                conditions,
                variables: counter.variables_for_key(),
            }
        }
    }

    #[derive(PartialEq, Debug, Serialize, Deserialize)]
    struct LimitCountersKey<'a> {
        ns: &'a str,
        seconds: u64,
        conditions: Vec<String>,
        variables: Vec<&'a str>,
    }

    impl<'a> From<&'a Limit> for LimitCountersKey<'a> {
        fn from(limit: &'a Limit) -> Self {
            let set = limit.conditions();
            let mut conditions = Vec::with_capacity(set.len());
            for cond in &set {
                conditions.push(cond.clone());
            }
            conditions.sort();

            LimitCountersKey {
                ns: limit.namespace().as_ref(),
                seconds: limit.seconds(),
                conditions,
                variables: limit.variables_for_key(),
            }
        }
    }

    pub fn key_for_counter(counter: &Counter) -> Vec<u8> {
        let key: CounterKey = counter.into();
        postcard::to_stdvec(&key).unwrap()
    }

    pub fn prefix_for_namespace(namespace: &str) -> Vec<u8> {
        postcard::to_stdvec(namespace).unwrap()
    }

    pub fn partial_counter_from_counter_key(key: &[u8]) -> Counter {
        let key: CounterKey = postcard::from_bytes(key).unwrap();
        let CounterKey {
            ns,
            seconds,
            conditions,
            variables,
        } = key;

        let map: HashMap<String, String> = variables
            .into_iter()
            .map(|(var, value)| (var.to_string(), value.to_string()))
            .collect();
        let limit = Limit::new(ns, u64::default(), seconds, conditions, map.keys());
        Counter::new(limit, map)
    }

    #[cfg(test)]
    mod tests {
        use super::{
            key_for_counter, partial_counter_from_counter_key, prefix_for_namespace, CounterKey,
        };
        use crate::counter::Counter;
        use crate::Limit;
        use std::collections::HashMap;

        #[test]
        fn counter_key_serializes_and_back() {
            let namespace = "ns_counter:";
            let limit = Limit::new(
                namespace,
                1,
                2,
                vec!["foo == 'bar'"],
                vec!["app_id", "role", "wat"],
            );
            let mut vars = HashMap::default();
            vars.insert("role".to_string(), "admin".to_string());
            vars.insert("app_id".to_string(), "123".to_string());
            vars.insert("wat".to_string(), "dunno".to_string());
            let counter = Counter::new(limit.clone(), vars);

            let raw = key_for_counter(&counter);
            let key_back: CounterKey =
                postcard::from_bytes(&raw).expect("This should deserialize back!");
            let key: CounterKey = (&counter).into();
            assert_eq!(key_back, key);
        }

        #[test]
        fn counter_key_and_counter_are_symmetric() {
            let namespace = "ns_counter:";
            let limit = Limit::new(namespace, 1, 1, vec!["req.method == 'GET'"], vec!["app_id"]);
            let mut variables = HashMap::default();
            variables.insert("app_id".to_string(), "123".to_string());
            let counter = Counter::new(limit.clone(), variables);
            let raw = key_for_counter(&counter);
            assert_eq!(counter, partial_counter_from_counter_key(&raw));
        }

        #[test]
        fn counter_key_starts_with_namespace_prefix() {
            let namespace = "ns_counter:";
            let limit = Limit::new(namespace, 1, 1, vec!["req.method == 'GET'"], vec!["app_id"]);
            let counter = Counter::new(limit, HashMap::default());
            let serialized_counter = key_for_counter(&counter);

            let prefix = prefix_for_namespace(namespace);
            assert_eq!(&serialized_counter[..prefix.len()], &prefix);
        }
    }
}
