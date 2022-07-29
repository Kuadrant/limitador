use crate::counter::Counter;
use crate::limit::Limit;
use crate::storage::infinispan::counters::{Consistency, CounterOpts};
use crate::storage::infinispan::response::response_to_string;
use crate::storage::infinispan::{
    counters, sets, DEFAULT_INFINISPAN_CONSISTENCY, DEFAULT_INFINISPAN_LIMITS_CACHE_NAME,
};
use crate::storage::keys::*;
use crate::storage::{AsyncCounterStorage, Authorization, StorageErr};
use async_trait::async_trait;
use infinispan::errors::InfinispanError;
use infinispan::request;
use infinispan::Infinispan;
use std::collections::HashSet;
use std::time::Duration;

pub struct InfinispanStorage {
    infinispan: Infinispan,
    cache_name: String,
    counters_consistency: Consistency,
}

pub struct InfinispanStorageBuilder {
    url: String,
    username: String,
    password: String,
    cache_name: Option<String>,
    counters_consistency: Option<Consistency>,
}

#[async_trait]
impl AsyncCounterStorage for InfinispanStorage {
    async fn is_within_limits(&self, counter: &Counter, delta: i64) -> Result<bool, StorageErr> {
        let counter_key = key_for_counter(counter);
        let counter_val =
            counters::get_value(&self.infinispan, &self.cache_name, &counter_key).await?;

        match counter_val {
            Some(val) => Ok(val - delta >= 0),
            None => Ok(counter.max_value() - delta >= 0),
        }
    }

    async fn update_counter(&self, counter: &Counter, delta: i64) -> Result<(), StorageErr> {
        let counter_key = key_for_counter(counter);

        let counter_created = counters::decrement_by(
            &self.infinispan,
            &self.cache_name,
            &counter_key,
            delta,
            &CounterOpts::new(
                counter.max_value(),
                Duration::from_secs(counter.seconds()),
                self.counters_consistency,
            ),
        )
        .await?;

        if counter_created {
            self.add_to_set(key_for_counters_of_limit(counter.limit()), counter_key)
                .await?;
        }

        Ok(())
    }

    async fn check_and_update(
        &self,
        counters: HashSet<Counter>,
        delta: i64,
    ) -> Result<Authorization, StorageErr> {
        for counter in counters.iter() {
            if !self.is_within_limits(counter, delta).await? {
                return Ok(Authorization::Limited(
                    counter.limit().name().map(|n| n.to_owned()),
                ));
            }
        }

        // Update only if all are withing limits
        for counter in counters {
            self.update_counter(&counter, delta).await?
        }

        Ok(Authorization::Ok)
    }

    async fn get_counters(&self, limits: HashSet<Limit>) -> Result<HashSet<Counter>, StorageErr> {
        let mut res = HashSet::new();

        for limit in limits {
            for counter_key in self.counter_keys_of_limit(&limit).await? {
                let counter_val =
                    counters::get_value(&self.infinispan, &self.cache_name, &counter_key).await?;

                // If the key does not exist, it means that the counter expired,
                // so we don't have to return it.
                //
                // TODO: we should delete the counter from the set of counters
                // This does not cause any bugs, but consumes memory
                // unnecessarily.

                if let Some(val) = counter_val {
                    let mut counter: Counter = counter_from_counter_key(&counter_key, &limit);
                    let ttl = 0; // TODO: calculate TTL from response headers.
                    counter.set_remaining(val);
                    counter.set_expires_in(Duration::from_secs(ttl));
                    res.insert(counter);
                }
            }
        }

        Ok(res)
    }

    async fn delete_counters(&self, limits: HashSet<Limit>) -> Result<(), StorageErr> {
        for limit in limits {
            self.delete_counters_associated_with_limit(&limit).await?;
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), StorageErr> {
        let _ = self
            .infinispan
            .run(&request::caches::clear(&self.cache_name))
            .await?;

        let _ = self.delete_all_counters().await?;

        Ok(())
    }
}

impl InfinispanStorage {
    pub async fn new(
        url: &str,
        username: &str,
        password: &str,
        cache_name: Option<String>,
        counters_consistency: Consistency,
    ) -> Self {
        let infinispan = Infinispan::new(url, username, password);

        match cache_name {
            Some(cache_name) => Self {
                infinispan,
                cache_name,
                counters_consistency,
            },
            None => {
                let cache_name = DEFAULT_INFINISPAN_LIMITS_CACHE_NAME;

                let _ = infinispan
                    .run(&request::caches::create_local(&cache_name))
                    .await
                    .unwrap();

                Self {
                    infinispan,
                    cache_name: cache_name.into(),
                    counters_consistency,
                }
            }
        }
    }

    async fn delete_counters_associated_with_limit(&self, limit: &Limit) -> Result<(), StorageErr> {
        for counter_key in self.counter_keys_of_limit(limit).await? {
            counters::delete(&self.infinispan, &self.cache_name, &counter_key).await?
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
        self.get_set(key_for_counters_of_limit(limit)).await
    }

    async fn get_set(&self, set_key: impl AsRef<str>) -> Result<HashSet<String>, InfinispanError> {
        sets::get(&self.infinispan, &self.cache_name, set_key).await
    }

    async fn add_to_set(
        &self,
        set_key: impl Into<String>,
        element: impl Into<String>,
    ) -> Result<(), StorageErr> {
        sets::add(&self.infinispan, &self.cache_name, set_key, element).await
    }
}

impl InfinispanStorageBuilder {
    pub fn new(
        url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            username: username.into(),
            password: password.into(),
            cache_name: None,
            counters_consistency: None,
        }
    }

    pub fn cache_name(mut self, cache_name: impl Into<String>) -> Self {
        self.cache_name = Some(cache_name.into());
        self
    }

    pub fn counters_consistency(mut self, counters_consistency: Consistency) -> Self {
        self.counters_consistency = Some(counters_consistency);
        self
    }

    pub async fn build(self) -> InfinispanStorage {
        InfinispanStorage::new(
            &self.url,
            &self.username,
            &self.password,
            self.cache_name,
            self.counters_consistency
                .unwrap_or(DEFAULT_INFINISPAN_CONSISTENCY),
        )
        .await
    }
}
