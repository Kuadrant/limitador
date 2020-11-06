#[macro_use]
extern crate log;

use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use futures::join;
use limitador::limit::Limit;
use limitador::storage::redis::{AsyncRedisStorage, CachedRedisStorageBuilder};
use limitador::{AsyncRateLimiter, RateLimiter};
use std::env;
use std::sync::Arc;
use std::time::Duration;

mod envoy_rls;
mod http_api;

const LIMITS_FILE_ENV: &str = "LIMITS_FILE";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_HTTP_API_PORT: u32 = 8080;
const DEFAULT_ENVOY_RLS_PORT: u32 = 8081;
const ENV_OPTION_ENABLED: &str = "1";

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

impl Limiter {
    pub async fn new() -> Limiter {
        let rate_limiter = match env::var("REDIS_URL") {
            Ok(redis_url) => {
                if Self::env_option_is_enabled("REDIS_LOCAL_CACHE_ENABLED") {
                    Self::limiter_with_redis_and_local_cache(&redis_url).await
                } else {
                    // Let's use the async impl. This could be configurable if needed.
                    Self::limiter_with_async_redis(&redis_url).await
                }
            }
            Err(_) => Self::default_blocking_limiter(),
        };

        rate_limiter.load_limits_from_file().await;

        rate_limiter
    }

    async fn limiter_with_async_redis(redis_url: &str) -> Limiter {
        let async_limiter =
            AsyncRateLimiter::new_with_storage(Box::new(AsyncRedisStorage::new(&redis_url).await));
        Limiter::Async(async_limiter)
    }

    async fn limiter_with_redis_and_local_cache(redis_url: &str) -> Limiter {
        // TODO: Not all the options are configurable via ENV. Add them as needed.

        let mut cached_redis_storage = CachedRedisStorageBuilder::new(&redis_url);

        if let Ok(flushing_period_secs) = env::var("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS") {
            let parsed_flushing_period: i64 = flushing_period_secs.parse().unwrap();

            if parsed_flushing_period < 0 {
                cached_redis_storage = cached_redis_storage.flushing_period(None)
            } else {
                cached_redis_storage = cached_redis_storage
                    .flushing_period(Some(Duration::from_millis(parsed_flushing_period as u64)))
            }
        };

        if let Ok(max_ttl_cached_counters) =
            env::var("REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS")
        {
            cached_redis_storage = cached_redis_storage.max_ttl_cached_counters(
                Duration::from_millis(max_ttl_cached_counters.parse().unwrap()),
            )
        }

        if let Ok(ttl_ratio_cached_counters) =
            env::var("REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS")
        {
            cached_redis_storage = cached_redis_storage
                .ttl_ratio_cached_counters(ttl_ratio_cached_counters.parse().unwrap());
        }

        let async_limiter =
            AsyncRateLimiter::new_with_storage(Box::new(cached_redis_storage.build().await));
        Limiter::Async(async_limiter)
    }

    fn default_blocking_limiter() -> Limiter {
        Limiter::Blocking(RateLimiter::default())
    }

    fn env_option_is_enabled(env_name: &str) -> bool {
        match env::var(env_name) {
            Ok(value) => value == ENV_OPTION_ENABLED,
            Err(_) => false,
        }
    }

    async fn load_limits_from_file(&self) {
        if let Ok(limits_file_path) = env::var(LIMITS_FILE_ENV) {
            let f = std::fs::File::open(limits_file_path).unwrap();
            let limits: Vec<Limit> = serde_yaml::from_reader(f).unwrap();

            match &self {
                Limiter::Blocking(limiter) => limiter.configure_with(limits).unwrap(),
                Limiter::Async(limiter) => limiter.configure_with(limits).await.unwrap(),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Both actix and tonic create N threads (N = number of cpus). Tonic
    // seems to use only the threads it creates, but Actix uses also the ones
    // created by Tonic. We'd need to either create just N threads in total and
    // make both frameworks use them or make Actix use only the threads it
    // creates. This only happens when using the async redis storage
    // implementation.

    env_logger::init();

    let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await);

    let tokio_local = tokio::task::LocalSet::new();
    let actix_system_handler = actix_rt::System::run_in_tokio("http-server", &tokio_local);
    let http_api_host = env::var("HTTP_API_HOST").unwrap_or_else(|_| String::from(DEFAULT_HOST));
    let http_api_port =
        env::var("HTTP_API_PORT").unwrap_or_else(|_| DEFAULT_HTTP_API_PORT.to_string());
    let http_api_address = format!("{}:{}", http_api_host, http_api_port);
    let http_server_handler = run_http_server(&http_api_address, rate_limiter.clone());

    let envoy_rls_host = env::var("ENVOY_RLS_HOST").unwrap_or_else(|_| String::from(DEFAULT_HOST));
    let envoy_rls_port =
        env::var("ENVOY_RLS_PORT").unwrap_or_else(|_| DEFAULT_ENVOY_RLS_PORT.to_string());
    let envoy_rls_address = format!("{}:{}", envoy_rls_host, envoy_rls_port);
    info!("Envoy RLS server starting on {}", envoy_rls_address);
    let envoy_rls_handler =
        run_envoy_rls_server(envoy_rls_address.to_string(), rate_limiter.clone());

    join!(http_server_handler, envoy_rls_handler);

    actix_system_handler.await?;

    Ok(())
}
