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
    use crate::storage::keys::key_for_counters_of_limit;
    use crate::Limit;

    #[test]
    fn key_for_limit_format() {
        let limit = Limit::new(
            "example.com",
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        );
        assert_eq!(
            "namespace:{example.com},counters_of_limit:{\"namespace\":\"example.com\",\"seconds\":60,\"conditions\":[\"req.method == GET\"],\"variables\":[\"app_id\"]}",
            key_for_counters_of_limit(&limit))
    }
}
