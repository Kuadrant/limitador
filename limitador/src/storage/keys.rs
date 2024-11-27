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
use std::sync::Arc;

pub fn key_for_counter(counter: &Counter) -> Vec<u8> {
    if counter.id().is_none() {
        // continue to use the legacy text encoding...
        let namespace = counter.namespace().as_ref();
        let key = if counter.remaining().is_some() || counter.expires_in().is_some() {
            format!(
                "namespace:{{{namespace}}},counter:{}",
                serde_json::to_string(&counter.key()).unwrap()
            )
        } else {
            format!(
                "namespace:{{{namespace}}},counter:{}",
                serde_json::to_string(counter).unwrap()
            )
        };
        key.into_bytes()
    } else {
        // if the id is set, use the new binary encoding...
        bin::key_for_counter_v2(counter)
    }
}

pub fn key_for_counters_of_limit(limit: &Limit) -> Vec<u8> {
    if let Some(id) = limit.id() {
        #[derive(PartialEq, Debug, Serialize, Deserialize)]
        struct IdLimitKey<'a> {
            id: &'a str,
        }

        let key = IdLimitKey { id };

        let mut encoded_key = Vec::new();
        encoded_key = postcard::to_extend(&2u8, encoded_key).unwrap();
        encoded_key = postcard::to_extend(&key, encoded_key).unwrap();
        encoded_key
    } else {
        let namespace = limit.namespace().as_ref();
        format!(
            "namespace:{{{namespace}}},counters_of_limit:{}",
            serde_json::to_string(limit).unwrap()
        )
        .into_bytes()
    }
}

pub fn counter_from_counter_key(key: &Vec<u8>, limit: Arc<Limit>) -> Counter {
    let mut counter = partial_counter_from_counter_key(key);
    if !counter.update_to_limit(Arc::clone(&limit)) {
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

pub fn partial_counter_from_counter_key(key: &Vec<u8>) -> Counter {
    if key.starts_with(b"namespace:") {
        let key = String::from_utf8_lossy(key.as_ref());

        // It's using to the legacy text encoding...
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
    } else {
        // It's using to the new binary encoding...
        bin::partial_counter_from_counter_key_v2(key)
    }
}

#[cfg(test)]
mod tests {
    use super::{key_for_counter, key_for_counters_of_limit, partial_counter_from_counter_key};
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
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id"],
        );
        assert_eq!(
            "namespace:{example.com},counters_of_limit:{\"namespace\":\"example.com\",\"seconds\":60,\"conditions\":[\"req_method == 'GET'\"],\"variables\":[\"app_id\"]}".as_bytes(),
            key_for_counters_of_limit(&limit))
    }

    #[test]
    fn key_for_limit_with_id_format() {
        let limit = Limit::with_id(
            "test_id",
            "example.com",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id"],
        );
        assert_eq!(
            "\u{2}\u{7}test_id".as_bytes(),
            key_for_counters_of_limit(&limit)
        )
    }

    #[test]
    fn counter_key_and_counter_are_symmetric() {
        let namespace = "ns_counter:";
        let limit = Limit::new(
            namespace,
            1,
            1,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id"],
        );
        let counter = Counter::new(limit.clone(), HashMap::default());
        let raw = key_for_counter(&counter);
        assert_eq!(counter, partial_counter_from_counter_key(&raw));
    }

    #[test]
    fn counter_key_does_not_include_transient_state() {
        let namespace = "ns_counter:";
        let limit = Limit::new(
            namespace,
            1,
            1,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id"],
        );
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
    use crate::limit::{Limit, Predicate};

    #[derive(PartialEq, Debug, Serialize, Deserialize)]
    struct IdCounterKey<'a> {
        id: &'a str,
        variables: Vec<(&'a str, &'a str)>,
    }

    impl<'a> From<&'a Counter> for IdCounterKey<'a> {
        fn from(counter: &'a Counter) -> Self {
            IdCounterKey {
                id: counter.id().unwrap(),
                variables: counter.variables_for_key(),
            }
        }
    }

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

    pub fn key_for_counter_v2(counter: &Counter) -> Vec<u8> {
        let mut encoded_key = Vec::new();
        if counter.id().is_none() {
            let key: CounterKey = counter.into();
            encoded_key = postcard::to_extend(&1u8, encoded_key).unwrap();
            encoded_key = postcard::to_extend(&key, encoded_key).unwrap()
        } else {
            let key: IdCounterKey = counter.into();
            encoded_key = postcard::to_extend(&2u8, encoded_key).unwrap();
            encoded_key = postcard::to_extend(&key, encoded_key).unwrap();
        }
        encoded_key
    }

    pub fn partial_counter_from_counter_key_v2(key: &[u8]) -> Counter {
        let (version, key) = postcard::take_from_bytes::<u8>(key).unwrap();
        match version {
            1u8 => {
                let CounterKey {
                    ns,
                    seconds,
                    conditions,
                    variables,
                } = postcard::from_bytes(key).unwrap();

                let map: HashMap<String, String> = variables
                    .into_iter()
                    .map(|(var, value)| (var.to_string(), value.to_string()))
                    .collect();
                let limit = Limit::new(
                    ns,
                    u64::default(),
                    seconds,
                    conditions
                        .into_iter()
                        .map(|p| p.try_into().expect("condition corrupted!"))
                        .collect::<Vec<Predicate>>(),
                    map.keys(),
                );
                Counter::new(limit, map)
            }
            2u8 => {
                let IdCounterKey { id, variables } = postcard::from_bytes(key).unwrap();
                let map: HashMap<String, String> = variables
                    .into_iter()
                    .map(|(var, value)| (var.to_string(), value.to_string()))
                    .collect();

                // we are not able to rebuild the full limit since we only have the id and variables.
                let limit =
                    Limit::with_id::<&str, &str>(id, "", u64::default(), 0, vec![], map.keys());
                Counter::new(limit, map)
            }
            _ => panic!("Unknown version: {}", version),
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
        let limit = Limit::new(
            ns,
            u64::default(),
            seconds,
            conditions
                .into_iter()
                .map(|p| p.try_into().expect("condition corrupted!"))
                .collect::<Vec<Predicate>>(),
            map.keys(),
        );
        Counter::new(limit, map)
    }

    #[cfg(test)]
    mod tests {
        use super::{
            key_for_counter, key_for_counter_v2, partial_counter_from_counter_key,
            prefix_for_namespace, CounterKey,
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
                vec!["foo == 'bar'".try_into().expect("failed parsing!")],
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
            let limit = Limit::new(
                namespace,
                1,
                1,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id"],
            );
            let mut variables = HashMap::default();
            variables.insert("app_id".to_string(), "123".to_string());
            let counter = Counter::new(limit.clone(), variables);
            let raw = key_for_counter(&counter);
            assert_eq!(counter, partial_counter_from_counter_key(&raw));
        }

        #[test]
        fn counter_key_starts_with_namespace_prefix() {
            let namespace = "ns_counter:";
            let limit = Limit::new(
                namespace,
                1,
                1,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id"],
            );
            let counter = Counter::new(limit, HashMap::default());
            let serialized_counter = key_for_counter(&counter);

            let prefix = prefix_for_namespace(namespace);
            assert_eq!(&serialized_counter[..prefix.len()], &prefix);
        }

        #[test]
        fn counters_with_id() {
            let namespace = "ns_counter:";
            let limit_without_id = Limit::new(
                namespace,
                1,
                1,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id"],
            );
            let limit_with_id = Limit::with_id(
                "id200",
                namespace,
                1,
                1,
                vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
                vec!["app_id"],
            );

            let counter_with_id = Counter::new(limit_with_id, HashMap::default());
            let serialized_with_id_counter = key_for_counter(&counter_with_id);

            let counter_without_id = Counter::new(limit_without_id, HashMap::default());
            let serialized_without_id_counter = key_for_counter(&counter_without_id);

            // the original key_for_counter continues to encode kinda big
            assert_eq!(serialized_without_id_counter.len(), 35);
            assert_eq!(serialized_with_id_counter.len(), 35);

            // serialized_counter_v2 will only encode the id.... so it will be smaller for
            // counters with an id.
            let serialized_counter_with_id_v2 = key_for_counter_v2(&counter_with_id);
            assert_eq!(serialized_counter_with_id_v2.clone().len(), 8);

            // but continues to be large for counters without an id.
            let serialized_counter_without_id_v2 = key_for_counter_v2(&counter_without_id);
            assert_eq!(serialized_counter_without_id_v2.clone().len(), 36);
        }
    }
}
