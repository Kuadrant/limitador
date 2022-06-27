#![deny(clippy::all, clippy::cargo)]

#[macro_use]
extern crate log;

use crate::config::{
    Configuration, InfinispanStorageConfiguration, RedisStorageCacheConfiguration,
    RedisStorageConfiguration, StorageConfiguration,
};
use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use limitador::errors::LimitadorError;
use limitador::limit::Limit;
use limitador::storage::infinispan::{Consistency, InfinispanStorageBuilder};
use limitador::storage::redis::{AsyncRedisStorage, CachedRedisStorage, CachedRedisStorageBuilder};
use limitador::storage::{AsyncCounterStorage, AsyncStorage};
use limitador::{AsyncRateLimiter, AsyncRateLimiterBuilder, RateLimiter, RateLimiterBuilder};
use notify::event::{DataChange, ModifyKind};
use notify::{Error, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::convert::TryInto;
use std::path::Path;
use std::process;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::runtime::Handle;
use url::Url;

mod envoy_rls;
mod http_api;

mod config;

#[derive(Error, Debug)]
pub enum LimitadorServerError {
    #[error("please set either the Redis or the Infinispan URL, but not both")]
    IncompatibleStorages,
    #[error("Invalid limit file: {0}")]
    ConfigFile(String),
    #[error("Internal error: {0}")]
    Internal(LimitadorError),
}

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

impl From<LimitadorError> for LimitadorServerError {
    fn from(e: LimitadorError) -> Self {
        Self::Internal(e)
    }
}

impl Limiter {
    pub async fn new(config: Configuration) -> Result<Self, LimitadorServerError> {
        let rate_limiter = match config.storage {
            StorageConfiguration::Redis(cfg) => {
                Self::redis_limiter(cfg, config.limit_name_in_labels).await
            }
            StorageConfiguration::Infinispan(cfg) => {
                Self::infinispan_limiter(cfg, config.limit_name_in_labels).await
            }
            StorageConfiguration::InMemory => Self::in_memory_limiter(config),
        };

        Ok(rate_limiter)
    }

    async fn redis_limiter(cfg: RedisStorageConfiguration, limit_name_labels: bool) -> Self {
        let storage = Self::storage_using_redis(cfg).await;
        let mut rate_limiter_builder = AsyncRateLimiterBuilder::new(storage);

        if limit_name_labels {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Async(rate_limiter_builder.build())
    }

    async fn storage_using_redis(cfg: RedisStorageConfiguration) -> AsyncStorage {
        let counters: Box<dyn AsyncCounterStorage> = if let Some(cache) = &cfg.cache {
            Box::new(Self::storage_using_redis_and_local_cache(&cfg.url, cache).await)
        } else {
            // Let's use the async impl. This could be configurable if needed.
            Box::new(Self::storage_using_async_redis(&cfg.url).await)
        };
        AsyncStorage::with_counter_storage(counters)
    }

    async fn storage_using_async_redis(redis_url: &str) -> AsyncRedisStorage {
        AsyncRedisStorage::new(redis_url).await
    }

    async fn storage_using_redis_and_local_cache(
        redis_url: &str,
        cache_cfg: &RedisStorageCacheConfiguration,
    ) -> CachedRedisStorage {
        // TODO: Not all the options are configurable via ENV. Add them as needed.

        let mut cached_redis_storage = CachedRedisStorageBuilder::new(redis_url);

        if cache_cfg.flushing_period < 0 {
            cached_redis_storage = cached_redis_storage.flushing_period(None)
        } else {
            cached_redis_storage = cached_redis_storage.flushing_period(Some(
                Duration::from_millis(cache_cfg.flushing_period as u64),
            ))
        }

        cached_redis_storage =
            cached_redis_storage.max_ttl_cached_counters(Duration::from_millis(cache_cfg.max_ttl));

        cached_redis_storage = cached_redis_storage.ttl_ratio_cached_counters(cache_cfg.ttl_ratio);

        cached_redis_storage.build().await
    }

    async fn infinispan_limiter(
        cfg: InfinispanStorageConfiguration,
        limit_name_labels: bool,
    ) -> Self {
        let parsed_url = Url::parse(&cfg.url).unwrap();

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

        let consistency: Option<Consistency> = match cfg.consistency {
            Some(cfg_value) => match cfg_value.try_into() {
                Ok(consistency) => Some(consistency),
                Err(_) => {
                    eprintln!("Invalid consistency mode, will apply the default");
                    None
                }
            },
            None => None,
        };

        if let Some(consistency) = consistency {
            builder = builder.counters_consistency(consistency);
        }

        let storage = match &cfg.cache {
            Some(cache_name) => builder.cache_name(cache_name).build().await,
            None => builder.build().await,
        };

        let mut rate_limiter_builder =
            AsyncRateLimiterBuilder::new(AsyncStorage::with_counter_storage(Box::new(storage)));

        if limit_name_labels {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Async(rate_limiter_builder.build())
    }

    fn in_memory_limiter(cfg: Configuration) -> Self {
        let mut rate_limiter_builder = RateLimiterBuilder::new();

        if cfg.limit_name_in_labels {
            rate_limiter_builder = rate_limiter_builder.with_prometheus_limit_name_labels()
        }

        Self::Blocking(rate_limiter_builder.build())
    }

    pub async fn load_limits_from_file<P: AsRef<Path>>(
        &self,
        path: &P,
    ) -> Result<(), LimitadorServerError> {
        match std::fs::File::open(path) {
            Ok(f) => {
                let parsed_limits: Result<Vec<Limit>, _> = serde_yaml::from_reader(f);
                match parsed_limits {
                    Ok(limits) => {
                        match &self {
                            Self::Blocking(limiter) => limiter.configure_with(limits)?,
                            Self::Async(limiter) => limiter.configure_with(limits).await?,
                        }
                        Ok(())
                    }
                    Err(e) => Err(LimitadorServerError::ConfigFile(format!(
                        "Couldn't parse: {}",
                        e
                    ))),
                }
            }
            Err(e) => Err(LimitadorServerError::ConfigFile(format!(
                "Couldn't read file '{}': {}",
                path.as_ref().display(),
                e
            ))),
        }
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = match Configuration::from_env() {
        Ok(config) => config,
        Err(_) => {
            eprintln!("Error: please set either the Redis or the Infinispan URL, but not both");
            process::exit(1)
        }
    };

    let limit_file = config.limits_file.clone();
    let envoy_rls_address = config.rlp_address();
    let http_api_address = config.http_address();

    let rate_limiter: Arc<Limiter> = match Limiter::new(config).await {
        Ok(limiter) => Arc::new(limiter),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1)
        }
    };

    let _watcher = if let Some(limits_file_path) = limit_file {
        if let Err(e) = rate_limiter.load_limits_from_file(&limits_file_path).await {
            eprintln!("Failed to load limit file: {}", e);
            process::exit(1)
        }

        let limiter = Arc::clone(&rate_limiter);
        let handle = Handle::current();

        let mut watcher =
            RecommendedWatcher::new(move |result: Result<Event, Error>| match result {
                Ok(ref event) => {
                    if let EventKind::Modify(ModifyKind::Data(DataChange::Content)) = event.kind {
                        let limiter = limiter.clone();
                        let location = event.paths.first().unwrap().clone();
                        handle.spawn(async move {
                            match limiter.load_limits_from_file(&location).await {
                                Ok(_) => info!("Reloaded limit file"),
                                Err(e) => error!("Failed reloading limit file: {}", e),
                            }
                        });
                    }
                }
                Err(ref e) => {
                    warn!("Something went wrong while watching limit file: {}", e);
                }
            })?;

        watcher.watch(Path::new(&limits_file_path), RecursiveMode::Recursive)?;
        Some(watcher)
    } else {
        None
    };

    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
    ));

    run_http_server(&http_api_address, rate_limiter.clone()).await?;

    Ok(())
}
