#![deny(clippy::all, clippy::cargo)]

#[macro_use]
extern crate log;

use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use limitador::limit::Limit;
use limitador::storage::infinispan::{Consistency, InfinispanStorageBuilder};
use limitador::storage::redis::{AsyncRedisStorage, CachedRedisStorage, CachedRedisStorageBuilder};
use limitador::storage::AsyncStorage;
use limitador::{AsyncRateLimiter, AsyncRateLimiterBuilder, RateLimiter, RateLimiterBuilder};
use std::convert::TryInto;
use std::sync::Arc;
use std::time::Duration;
use std::{env, process};
use thiserror::Error;
use url::Url;

mod envoy_rls;
mod http_api;

const LIMITS_FILE_ENV: &str = "LIMITS_FILE";
const LIMIT_NAME_IN_PROMETHEUS_LABELS_ENV: &str = "LIMIT_NAME_IN_PROMETHEUS_LABELS";
const REDIS_URL_ENV: &str = "REDIS_URL";
const INFINISPAN_URL_ENV: &str = "INFINISPAN_URL";
const INFINISPAN_CACHE_NAME_ENV: &str = "INFINISPAN_CACHE_NAME";
const INFINISPAN_COUNTERS_CONSISTENCY_ENV: &str = "INFINISPAN_COUNTERS_CONSISTENCY";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_HTTP_API_PORT: u32 = 8080;
const DEFAULT_ENVOY_RLS_PORT: u32 = 8081;
const ENV_OPTION_ENABLED: &str = "1";

#[derive(Error, Debug)]
pub enum LimitadorServerError {
    #[error("please set either the Redis or the Infinispan URL, but not both")]
    IncompatibleStorages,
}

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

enum StorageType {
    Redis { url: String },
    Infinispan { url: String },
    InMemory,
}

impl Limiter {
    pub async fn new() -> Result<Self, LimitadorServerError> {
        let rate_limiter = match Self::storage_type()? {
            StorageType::Redis { url } => Self::redis_limiter(&url).await,
            StorageType::Infinispan { url } => Self::infinispan_limiter(&url).await,
            StorageType::InMemory => Self::in_memory_limiter(),
        };

        rate_limiter.load_limits_from_file().await;

        Ok(rate_limiter)
    }

    fn storage_type() -> Result<StorageType, LimitadorServerError> {
        let redis_url = env::var(REDIS_URL_ENV);
        let infinispan_url = env::var(INFINISPAN_URL_ENV);

        match (redis_url, infinispan_url) {
            (Ok(_), Ok(_)) => Err(LimitadorServerError::IncompatibleStorages),
            (Ok(url), Err(_)) => Ok(StorageType::Redis { url }),
            (Err(_), Ok(url)) => Ok(StorageType::Infinispan { url }),
            _ => Ok(StorageType::InMemory),
        }
    }

    async fn redis_limiter(url: &str) -> Self {
        let storage = Self::storage_using_redis(url).await;
        let mut rate_limiter_builder = AsyncRateLimiterBuilder::new(storage);

        if Self::env_option_is_enabled(LIMIT_NAME_IN_PROMETHEUS_LABELS_ENV) {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Async(rate_limiter_builder.build())
    }

    async fn storage_using_redis(redis_url: &str) -> Box<dyn AsyncStorage> {
        if Self::env_option_is_enabled("REDIS_LOCAL_CACHE_ENABLED") {
            Box::new(Self::storage_using_redis_and_local_cache(redis_url).await)
        } else {
            // Let's use the async impl. This could be configurable if needed.
            Box::new(Self::storage_using_async_redis(redis_url).await)
        }
    }

    async fn storage_using_async_redis(redis_url: &str) -> AsyncRedisStorage {
        AsyncRedisStorage::new(redis_url).await
    }

    async fn storage_using_redis_and_local_cache(redis_url: &str) -> CachedRedisStorage {
        // TODO: Not all the options are configurable via ENV. Add them as needed.

        let mut cached_redis_storage = CachedRedisStorageBuilder::new(redis_url);

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

        cached_redis_storage.build().await
    }

    async fn infinispan_limiter(url: &str) -> Self {
        let parsed_url = Url::parse(url).unwrap();

        let mut builder = InfinispanStorageBuilder::new(
            &format!(
                "{}://{}:{}",
                parsed_url.scheme(),
                parsed_url.host_str().unwrap(),
                parsed_url.port().unwrap(),
            ),
            parsed_url.username(),
            parsed_url.password().unwrap_or_default(),
        );

        let consistency: Option<Consistency> = match env::var(INFINISPAN_COUNTERS_CONSISTENCY_ENV) {
            Ok(env_value) => match env_value.try_into() {
                Ok(consistency) => Some(consistency),
                Err(_) => {
                    eprintln!("Invalid consistency mode, will apply the default");
                    None
                }
            },
            Err(_) => None,
        };

        if let Some(consistency) = consistency {
            builder = builder.counters_consistency(consistency);
        }

        let storage = match env::var(INFINISPAN_CACHE_NAME_ENV) {
            Ok(cache_name) => builder.cache_name(cache_name).build().await,
            Err(_) => builder.build().await,
        };

        let mut rate_limiter_builder = AsyncRateLimiterBuilder::new(Box::new(storage));

        if Self::env_option_is_enabled(LIMIT_NAME_IN_PROMETHEUS_LABELS_ENV) {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Async(rate_limiter_builder.build())
    }

    fn in_memory_limiter() -> Self {
        let mut rate_limiter_builder = RateLimiterBuilder::new();

        if Self::env_option_is_enabled(LIMIT_NAME_IN_PROMETHEUS_LABELS_ENV) {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Blocking(rate_limiter_builder.build())
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
                Self::Blocking(limiter) => limiter.configure_with(limits).unwrap(),
                Self::Async(limiter) => limiter.configure_with(limits).await.unwrap(),
            }
        }
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let rate_limiter: Arc<Limiter> = match Limiter::new().await {
        Ok(limiter) => Arc::new(limiter),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1)
        }
    };

    let envoy_rls_host = env::var("ENVOY_RLS_HOST").unwrap_or_else(|_| String::from(DEFAULT_HOST));
    let envoy_rls_port =
        env::var("ENVOY_RLS_PORT").unwrap_or_else(|_| DEFAULT_ENVOY_RLS_PORT.to_string());
    let envoy_rls_address = format!("{}:{}", envoy_rls_host, envoy_rls_port);
    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
    ));

    let http_api_host = env::var("HTTP_API_HOST").unwrap_or_else(|_| String::from(DEFAULT_HOST));
    let http_api_port =
        env::var("HTTP_API_PORT").unwrap_or_else(|_| DEFAULT_HTTP_API_PORT.to_string());
    let http_api_address = format!("{}:{}", http_api_host, http_api_port);
    run_http_server(&http_api_address, rate_limiter.clone()).await?;

    Ok(())
}
