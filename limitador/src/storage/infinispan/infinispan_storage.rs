use crate::counter::Counter;
use crate::limit::{Limit, Namespace};
use crate::storage::infinispan::counters::CounterOpts;
use crate::storage::infinispan::response::response_to_string;
use crate::storage::infinispan::{counters, sets};
use crate::storage::keys::*;
use crate::storage::{AsyncStorage, Authorization, StorageErr};
use async_trait::async_trait;
use infinispan::errors::InfinispanError;
use infinispan::request;
use infinispan::Infinispan;
use std::collections::HashSet;
use std::time::Duration;

const INFINISPAN_LIMITS_CACHE_NAME: &str = "limits";

pub struct InfinispanStorage {
    infinispan: Infinispan,
}

#[async_trait]
impl AsyncStorage for InfinispanStorage {
    async fn get_namespaces(&self) -> Result<HashSet<Namespace>, StorageErr> {
        Ok(self
            .get_set(key_for_namespaces_set())
            .await?
            .iter()
            .map(|ns| Namespace::from(ns.as_ref()))
            .collect())
    }

    async fn add_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        let serialized_limit = serde_json::to_string(limit).unwrap();

        self.add_to_set(
            key_for_limits_of_namespace(limit.namespace()),
            serialized_limit,
        )
        .await?;

        self.add_to_set(
            key_for_namespaces_set(),
            String::from(limit.namespace().clone().as_ref()),
        )
        .await?;

        Ok(())
    }

    async fn get_limits(&self, namespace: &Namespace) -> Result<HashSet<Limit>, StorageErr> {
        Ok(self
            .get_set(&key_for_limits_of_namespace(namespace))
            .await?
            .iter()
            .map(|limit_json| serde_json::from_str(limit_json).unwrap())
            .collect())
    }

    async fn delete_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        self.delete_counters_associated_with_limit(limit).await?;

        let _ = self
            .infinispan
            .run(&request::entries::delete(
                INFINISPAN_LIMITS_CACHE_NAME,
                key_for_counters_of_limit(&limit),
            ))
            .await?;

        let serialized_limit = serde_json::to_string(limit).unwrap();

        let limits_in_namespace = self
            .delete_from_set(
                key_for_limits_of_namespace(limit.namespace()),
                serialized_limit,
            )
            .await?;

        if limits_in_namespace.is_empty() {
            self.delete_from_set(
                key_for_namespaces_set(),
                String::from(limit.namespace().clone().as_ref()),
            )
            .await?;
        }

        Ok(())
    }

    async fn delete_limits(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        self.delete_counters_of_namespace(namespace).await?;

        for limit in self.get_limits(namespace).await? {
            let _ = self
                .infinispan
                .run(&request::entries::delete(
                    INFINISPAN_LIMITS_CACHE_NAME,
                    key_for_counters_of_limit(&limit),
                ))
                .await?;
        }

        let _ = self
            .infinispan
            .run(&request::entries::delete(
                INFINISPAN_LIMITS_CACHE_NAME,
                key_for_limits_of_namespace(namespace),
            ))
            .await?;

        Ok(())
    }

    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let counter_key = key_for_counter(counter);
        let counter_val =
            counters::get_value(&self.infinispan, INFINISPAN_LIMITS_CACHE_NAME, &counter_key)
                .await?;

        match counter_val {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let counter_key = key_for_counter(counter);

        let counter_created = counters::decrement_by(
            &self.infinispan,
            INFINISPAN_LIMITS_CACHE_NAME,
            &counter_key,
            delta,
            &CounterOpts::new(counter.max_value(), Duration::from_secs(counter.seconds())),
        )
        .await?;

        if counter_created {
            self.add_to_set(key_for_counters_of_limit(counter.limit()), counter_key)
                .await?;
        }

        Ok(())
    }

    async fn check_and_update<'c>(
        &self,
        counters: &HashSet<&'c Counter>,
        delta: i64,
    ) -> Result<Authorization<'c>, StorageErr> {
        for counter in counters {
            if !self.is_within_limits(counter, delta).await? {
                return Ok(Authorization::Limited(counter));
            }
        }

        // Update only if all are withing limits
        for counter in counters {
            self.update_counter(counter, delta).await?
        }

        Ok(Authorization::Ok)
    }

    async fn get_counters(&self, namespace: &Namespace) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        for limit in self.get_limits(namespace).await? {
            for counter_key in self.counter_keys_of_limit(&limit).await? {
                let counter_val = counters::get_value(
                    &self.infinispan,
                    INFINISPAN_LIMITS_CACHE_NAME,
                    &counter_key,
                )
                .await?;

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                //
                // TODO: we should delete the counter from the set of counters
                // This does not cause any bugs, but consumes memory
                // unnecessarily.

                if let Some(val) = counter_val {
                    let mut counter: Counter = counter_from_counter_key(&counter_key);
                    let ttl = 0; // TODO: calculate TTL from response headers.
                    counter.set_remaining(val);
                    counter.set_expires_in(Duration::from_secs(ttl));
                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        let _ = self
            .infinispan
            .run(&request::caches::clear(INFINISPAN_LIMITS_CACHE_NAME))
            .await?;

        let _ = self.delete_all_counters().await?;

        Ok(())
    }
}

impl InfinispanStorage {
    pub async fn new(url: &str, username: &str, password: &str) -> InfinispanStorage {
        let infinispan = Infinispan::new(url, username, password);

        // TODO: the cache type and its attributes should be configurable. For
        // now, we use the "local" type with the default attributes.
        //
        // TODO: check if this recreates everything or does not do anything when
        // it already exists.
        let _ = infinispan
            .run(&request::caches::create_local(INFINISPAN_LIMITS_CACHE_NAME))
            .await
            .unwrap();

        InfinispanStorage { infinispan }
    }

    async fn delete_counters_of_namespace(&self, namespace: &Namespace) -> Result<(), StorageErr> {
        for limit in self.get_limits(namespace).await? {
            self.delete_counters_associated_with_limit(&limit).await?
        }

        Ok(())
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        for counter_key in self.counter_keys_of_limit(&limit).await? {
            counters::delete(&self.infinispan, INFINISPAN_LIMITS_CACHE_NAME, &counter_key).await?
        }

        Ok(())
    }

    async fn delete_all_counters(&self) -> Result<(), StorageErr> {
        let resp = self.infinispan.run(&request::counters::list()).await?;

        let counter_names: HashSet<String> =
            serde_json::from_str(&response_to_string(resp).await).unwrap();

        for counter_name in counter_names {
            let _ = self
                .infinispan
                .run(&request::counters::delete(counter_name))
                .await?;
        }

        Ok(())
    }

    async fn counter_keys_of_limit(
        &self,
        limit: &Limit,
    ) -> Result<HashSet<String>, InfinispanError> {
        self.get_set(key_for_counters_of_limit(&limit)).await
    }

    async fn get_set(&self, set_key: impl AsRef<str>) -> Result<HashSet<String>, InfinispanError> {
        sets::get(&self.infinispan, INFINISPAN_LIMITS_CACHE_NAME, set_key).await
    }

    async fn add_to_set(
        &self,
        set_key: impl Into<String>,
        element: impl Into<String>,
    ) -> Result<(), StorageErr> {
        sets::add(
            &self.infinispan,
            INFINISPAN_LIMITS_CACHE_NAME,
            set_key,
            element,
        )
        .await
    }

    async fn delete_from_set(
        &self,
        set_key: impl Into<String>,
        element: impl Into<String>,
    ) -> Result<HashSet<String>, StorageErr> {
        sets::delete(
            &self.infinispan,
            INFINISPAN_LIMITS_CACHE_NAME,
            set_key,
            element,
        )
        .await
    }
}
