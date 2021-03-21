use limitador::counter::Counter;
use limitador::errors::LimitadorError;
use limitador::limit::{Limit, Namespace};
use limitador::{AsyncRateLimiter, RateLimiter};
use std::collections::{HashMap, HashSet};

// This exposes a struct that wraps both implementations of the rate limiter,
// the blocking and the async one. This allows us to avoid duplications in the
// tests.

enum LimiterImpl {
    Blocking(RateLimiter),
    #[allow(dead_code)] // dead when no "redis_storage"
    Async(AsyncRateLimiter),
}

pub struct TestsLimiter {
    limiter_impl: LimiterImpl,
}

impl TestsLimiter {
    pub fn new_from_blocking_impl(limiter: RateLimiter) -> Self {
        Self {
            limiter_impl: LimiterImpl::Blocking(limiter),
        }
    }

    #[allow(dead_code)] // dead when no "redis_storage"
    pub fn new_from_async_impl(limiter: AsyncRateLimiter) -> Self {
        Self {
            limiter_impl: LimiterImpl::Async(limiter),
        }
    }

    pub async fn get_namespaces(&self) -> Result<HashSet<Namespace>, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_namespaces(),
            LimiterImpl::Async(limiter) => limiter.get_namespaces().await,
        }
    }

    pub async fn add_limit(&self, limit: &Limit) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.add_limit(limit),
            LimiterImpl::Async(limiter) => limiter.add_limit(limit).await,
        }
    }

    pub async fn delete_limit(&self, limit: &Limit) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.delete_limit(limit),
            LimiterImpl::Async(limiter) => limiter.delete_limit(limit).await,
        }
    }

    pub async fn get_limits(&self, namespace: &str) -> Result<HashSet<Limit>, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_limits(namespace),
            LimiterImpl::Async(limiter) => limiter.get_limits(namespace).await,
        }
    }

    pub async fn delete_limits(&self, namespace: &str) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.delete_limits(namespace),
            LimiterImpl::Async(limiter) => limiter.delete_limits(namespace).await,
        }
    }

    pub async fn is_rate_limited(
        &self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
    ) -> Result<bool, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.is_rate_limited(namespace, values, delta),
            LimiterImpl::Async(limiter) => limiter.is_rate_limited(namespace, values, delta).await,
        }
    }

    pub async fn update_counters(
        &self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
    ) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.update_counters(namespace, values, delta),
            LimiterImpl::Async(limiter) => limiter.update_counters(namespace, values, delta).await,
        }
    }

    pub async fn check_rate_limited_and_update(
        &self,
        namespace: &str,
        values: &HashMap<String, String>,
        delta: i64,
    ) -> Result<bool, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => {
                limiter.check_rate_limited_and_update(namespace, values, delta)
            }
            LimiterImpl::Async(limiter) => {
                limiter
                    .check_rate_limited_and_update(namespace, values, delta)
                    .await
            }
        }
    }

    pub async fn get_counters(&self, namespace: &str) -> Result<HashSet<Counter>, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_counters(namespace),
            LimiterImpl::Async(limiter) => limiter.get_counters(namespace).await,
        }
    }

    pub async fn configure_with(
        &self,
        limits: impl IntoIterator<Item = Limit>,
    ) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.configure_with(limits),
            LimiterImpl::Async(limiter) => limiter.configure_with(limits).await,
        }
    }
}
