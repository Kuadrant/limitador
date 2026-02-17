use limitador::counter::Counter;
use limitador::errors::LimitadorError;
use limitador::limit::{Context, Limit, Namespace};
use limitador::{AsyncRateLimiter, CheckResult, RateLimiter};
use std::collections::HashSet;

// This exposes a struct that wraps both implementations of the rate limiter,
// the blocking and the async one. This allows us to avoid duplications in the
// tests.

enum LimiterImpl {
    Blocking(RateLimiter),
    #[cfg(feature = "redis_storage")]
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

    #[cfg(feature = "redis_storage")]
    pub fn new_from_async_impl(limiter: AsyncRateLimiter) -> Self {
        Self {
            limiter_impl: LimiterImpl::Async(limiter),
        }
    }

    pub async fn get_namespaces(&self) -> HashSet<Namespace> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_namespaces(),
            LimiterImpl::Async(limiter) => limiter.get_namespaces(),
        }
    }

    pub async fn add_limit(&self, limit: &Limit) -> bool {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.add_limit(limit.clone()),
            LimiterImpl::Async(limiter) => limiter.add_limit(limit.clone()),
        }
    }

    pub async fn delete_limit(&self, limit: &Limit) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.delete_limit(limit),
            LimiterImpl::Async(limiter) => limiter.delete_limit(limit).await,
        }
    }

    pub async fn get_limits(&self, namespace: &str) -> HashSet<Limit> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_limits(&namespace.into()),
            LimiterImpl::Async(limiter) => limiter.get_limits(&namespace.into()),
        }
    }

    pub async fn delete_limits(&self, namespace: &str) -> Result<(), LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.delete_limits(&namespace.into()),
            LimiterImpl::Async(limiter) => limiter.delete_limits(&namespace.into()).await,
        }
    }

    pub async fn is_rate_limited(
        &self,
        namespace: &str,
        ctx: &Context<'_>,
        delta: u64,
    ) -> Result<CheckResult, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => {
                limiter.is_rate_limited(&namespace.into(), ctx, delta, false) // TODO: test w/ true
            }
            LimiterImpl::Async(limiter) => {
                limiter.is_rate_limited(&namespace.into(), ctx, delta, false).await
            }
        }
    }

    pub async fn update_counters(
        &self,
        namespace: &str,
        ctx: &Context<'_>,
        delta: u64,
    ) -> Result<CheckResult, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => {
                limiter.update_counters(&namespace.into(), ctx, delta, false) // TODO: test w/true
            }
            LimiterImpl::Async(limiter) => {
                limiter.update_counters(&namespace.into(), ctx, delta, false).await
            }
        }
    }

    pub async fn check_rate_limited_and_update(
        &self,
        namespace: &str,
        ctx: &Context<'_>,
        delta: u64,
        load_counters: bool,
    ) -> Result<CheckResult, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => {
                limiter.check_rate_limited_and_update(&namespace.into(), ctx, delta, load_counters)
            }
            LimiterImpl::Async(limiter) => {
                limiter
                    .check_rate_limited_and_update(&namespace.into(), ctx, delta, load_counters)
                    .await
            }
        }
    }

    pub async fn get_counters(&self, namespace: &str) -> Result<HashSet<Counter>, LimitadorError> {
        match &self.limiter_impl {
            LimiterImpl::Blocking(limiter) => limiter.get_counters(&namespace.into()),
            LimiterImpl::Async(limiter) => limiter.get_counters(&namespace.into()).await,
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
