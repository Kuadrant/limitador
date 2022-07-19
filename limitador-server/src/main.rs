#![deny(clippy::all, clippy::cargo)]

#[macro_use]
extern crate log;
extern crate clap;

use crate::config::{
    Configuration, InfinispanStorageConfiguration, RedisStorageCacheConfiguration,
    RedisStorageConfiguration, StorageConfiguration,
};
use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use clap::builder::PossibleValuesParser;
use clap::{App, Arg, SubCommand};
use env_logger::Builder;
use limitador::errors::LimitadorError;
use limitador::limit::Limit;
use limitador::storage::infinispan::{Consistency, InfinispanStorageBuilder};
use limitador::storage::infinispan::{
    DEFAULT_INFINISPAN_CONSISTENCY, DEFAULT_INFINISPAN_LIMITS_CACHE_NAME,
};
use limitador::storage::redis::{
    AsyncRedisStorage, CachedRedisStorage, CachedRedisStorageBuilder, DEFAULT_FLUSHING_PERIOD_SEC,
    DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC,
    DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use limitador::storage::{AsyncCounterStorage, AsyncStorage};
use limitador::{AsyncRateLimiter, AsyncRateLimiterBuilder, RateLimiter, RateLimiterBuilder};
use log::LevelFilter;
use notify::event::ModifyKind;
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

const LIMITADOR_VERSION: &str = env!("CARGO_PKG_VERSION");

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
        cached_redis_storage = cached_redis_storage.max_cached_counters(cache_cfg.max_counters);

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
    let infinispan_consistency_default = format!("{}", DEFAULT_INFINISPAN_CONSISTENCY);

    let redis_cached_ttl_default = DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC.to_string();
    let redis_flushing_period_default = DEFAULT_FLUSHING_PERIOD_SEC.to_string();
    let redis_max_cached_counters_default = DEFAULT_MAX_CACHED_COUNTERS.to_string();
    let redis_ttl_ratio_default = DEFAULT_TTL_RATIO_CACHED_COUNTERS.to_string();

    let cmdline = App::new("Limitador Server")
        .version(LIMITADOR_VERSION)
        .author("The Kuadrant team - github.com/Kuadrant")
        .about("Rate Limiting Server")
        .disable_help_subcommand(true)
        .subcommand_negates_reqs(false)
        .subcommand_value_name("STORAGE")
        .subcommand_help_heading("STORAGES")
        .subcommand_required(false)
        .arg(
            Arg::with_name("config_from_env")
                .short('E')
                .long("use-env-vars")
                .help("Sets the server up from ENV VARS instead of these options")
                .exclusive(true),
        )
        .arg(
            Arg::with_name("LIMITS_FILE")
                .help("The limit file to use")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("ip")
                .short('b')
                .long("rls-ip")
                .default_value(Configuration::DEFAULT_IP_BIND)
                .display_order(1)
                .help("The IP to listen on for RLS"),
        )
        .arg(
            Arg::with_name("port")
                .short('p')
                .long("rls-port")
                .default_value(Configuration::DEFAULT_RLS_PORT)
                .display_order(2)
                .help("The port to listen on for RLS"),
        )
        .arg(
            Arg::with_name("http_ip")
                .short('B')
                .long("http-ip")
                .default_value(Configuration::DEFAULT_IP_BIND)
                .display_order(3)
                .help("The IP to listen on for HTTP"),
        )
        .arg(
            Arg::with_name("http_port")
                .short('P')
                .long("http-port")
                .default_value(Configuration::DEFAULT_HTTP_PORT)
                .display_order(4)
                .help("The port to listen on for HTTP"),
        )
        .arg(
            Arg::with_name("limit_name_in_labels")
                .short('l')
                .long("limit-name-in-labels")
                .display_order(5)
                .help("Include the Limit Name in prometheus label"),
        )
        .arg(
            Arg::with_name("v")
                .short('v')
                .multiple_occurrences(true)
                .max_occurrences(4)
                .display_order(6)
                .help("Sets the level of verbosity"),
        )
        .subcommand(
            SubCommand::with_name("memory")
                .display_order(1)
                .about("Counters are held in Limitador (ephemeral) [default storage]"),
        )
        .subcommand(
            SubCommand::with_name("redis")
                .display_order(2)
                .about("Uses Redis to store counters")
                .arg(
                    Arg::with_name("URL")
                        .help("Redis URL to use")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(
            SubCommand::with_name("redis_cached")
                .about("Uses Redis to store counters, with an in-memory cache")
                .display_order(3)
                .arg(
                    Arg::with_name("URL")
                        .help("Redis URL to use")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("TTL")
                        .long("ttl")
                        .takes_value(true)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(&redis_cached_ttl_default)
                        .display_order(2)
                        .help("TTL for cached counters in seconds"),
                )
                .arg(
                    Arg::with_name("ratio")
                        .long("ratio")
                        .takes_value(true)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(&redis_ttl_ratio_default)
                        .display_order(3)
                        .help("Ratio to apply to the TTL from Redis on cached counters"),
                )
                .arg(
                    Arg::with_name("flush")
                        .long("flush-period")
                        .takes_value(true)
                        .value_parser(clap::value_parser!(i64))
                        .default_value(&redis_flushing_period_default)
                        .display_order(4)
                        .help("Flushing period for counters in seconds"),
                )
                .arg(
                    Arg::with_name("max")
                        .long("max-cached")
                        .takes_value(true)
                        .value_parser(clap::value_parser!(usize))
                        .default_value(&redis_max_cached_counters_default)
                        .display_order(5)
                        .help("Maximum amount of counters cached"),
                ),
        )
        .subcommand(
            SubCommand::with_name("infinispan")
                .about("Uses Infinispan to store counters")
                .display_order(4)
                .arg(
                    Arg::with_name("URL")
                        .help("Infinispan URL to use")
                        .display_order(1)
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("cache name")
                        .short('n')
                        .long("cache-name")
                        .takes_value(true)
                        .default_value(DEFAULT_INFINISPAN_LIMITS_CACHE_NAME)
                        .display_order(2)
                        .help("Name of the cache to store counters in"),
                )
                .arg(
                    Arg::with_name("consistency")
                        .short('c')
                        .long("consistency")
                        .takes_value(true)
                        .default_value(&infinispan_consistency_default)
                        .value_parser(PossibleValuesParser::new(["Strong", "Weak"]))
                        .display_order(3)
                        .help("The consistency to use to read from the cache"),
                ),
        );
    let matches = cmdline.get_matches();

    let config = if matches.contains_id("config_from_env") {
        if matches.subcommand_name().is_some() {
            eprintln!("error: The argument '--use-env-vars' cannot be used with any subcommand");
            process::exit(1)
        }
        match Configuration::from_env() {
            Ok(config) => config,
            Err(_) => {
                eprintln!("error: please set either the Redis or the Infinispan URL, but not both");
                process::exit(1)
            }
        }
    } else {
        let limits_file = matches.value_of("LIMITS_FILE").unwrap();
        let storage = match matches.subcommand() {
            Some(("redis", sub)) => StorageConfiguration::Redis(RedisStorageConfiguration {
                url: sub.value_of("URL").unwrap().to_owned(),
                cache: None,
            }),
            Some(("redis_cached", sub)) => StorageConfiguration::Redis(RedisStorageConfiguration {
                url: sub.value_of("URL").unwrap().to_owned(),
                cache: Some(RedisStorageCacheConfiguration {
                    flushing_period: *sub.get_one("flush").unwrap(),
                    max_ttl: *sub.get_one("TTL").unwrap(),
                    ttl_ratio: *sub.get_one("ratio").unwrap(),
                    max_counters: *sub.get_one("max").unwrap(),
                }),
            }),
            Some(("infinispan", sub)) => {
                StorageConfiguration::Infinispan(InfinispanStorageConfiguration {
                    url: sub.value_of("URL").unwrap().to_owned(),
                    cache: Some(sub.value_of("cache name").unwrap().to_string()),
                    consistency: Some(sub.value_of("consistency").unwrap().to_string()),
                })
            }
            Some(("memory", _sub)) => StorageConfiguration::InMemory,
            None => StorageConfiguration::InMemory,
            _ => unreachable!("Some storage wasn't configured!"),
        };

        Configuration::with(
            storage,
            limits_file.to_string(),
            matches.value_of("ip").unwrap().into(),
            matches.value_of("port").unwrap().parse().unwrap(),
            matches.value_of("http_ip").unwrap().into(),
            matches.value_of("http_port").unwrap().parse().unwrap(),
            matches.value_of("limit_name_in_labels").is_some(),
        )
    };

    let level_filter = match matches.occurrences_of("v") {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        4 => LevelFilter::Trace,
        _ => unreachable!("Verbosity should at most be 4!"),
    };
    let mut builder = Builder::new();

    builder
        .filter(None, level_filter)
        .parse_default_env()
        .init();

    info!("Using config: {:?}", config);

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

    if let Err(e) = rate_limiter.load_limits_from_file(&limit_file).await {
        eprintln!("Failed to load limit file: {}", e);
        process::exit(1)
    }

    let limiter = Arc::clone(&rate_limiter);
    let handle = Handle::current();

    let mut watcher = RecommendedWatcher::new(move |result: Result<Event, Error>| match result {
        Ok(ref event) => {
            if let EventKind::Modify(ModifyKind::Data(_)) = event.kind {
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

    watcher.watch(Path::new(&limit_file), RecursiveMode::Recursive)?;

    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
    ));

    info!("HTTP server starting on {}", http_api_address);
    run_http_server(&http_api_address, rate_limiter.clone()).await?;

    Ok(())
}
