#![deny(clippy::all, clippy::cargo)]

#[macro_use]
extern crate log;
extern crate clap;

#[cfg(feature = "infinispan")]
use crate::config::InfinispanStorageConfiguration;
use crate::config::{
    Configuration, RedisStorageCacheConfiguration, RedisStorageConfiguration, StorageConfiguration,
};
use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use clap::{App, Arg, SubCommand};
use env_logger::Builder;
use limitador::errors::LimitadorError;
use limitador::limit::Limit;
#[cfg(feature = "infinispan")]
use limitador::storage::infinispan::{Consistency, InfinispanStorageBuilder};
#[cfg(feature = "infinispan")]
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
use notify::event::{ModifyKind, RenameMode};
use notify::{Error, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::env::VarError;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::{env, process};
use thiserror::Error;
use tokio::runtime::Handle;

mod envoy_rls;
mod http_api;

mod config;

const LIMITADOR_VERSION: &str = env!("CARGO_PKG_VERSION");
const LIMITADOR_PROFILE: &str = env!("LIMITADOR_PROFILE");
const LIMITADOR_FEATURES: Option<&'static str> = option_env!("LIMITADOR_FEATURES");
const LIMITADOR_HEADER: &str = "Limitador Server";

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
            #[cfg(feature = "infinispan")]
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

    #[cfg(feature = "infinispan")]
    async fn infinispan_limiter(
        cfg: InfinispanStorageConfiguration,
        limit_name_labels: bool,
    ) -> Self {
        use url::Url;

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
    let config = {
        let (config, version) = create_config();
        println!("{} {}", LIMITADOR_HEADER, version);
        let mut builder = Builder::new();
        if let Some(level) = config.log_level {
            builder.filter(None, level);
        } else {
            builder.parse_default_env();
        }
        builder.init();

        info!("Version: {}", version);
        info!("Using config: {:?}", config);
        config
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

    info!("limits file path: {}", limit_file);
    if let Err(e) = rate_limiter.load_limits_from_file(&limit_file).await {
        eprintln!("Failed to load limit file: {}", e);
        process::exit(1)
    }

    let limiter = Arc::clone(&rate_limiter);
    let handle = Handle::current();
    // it should not fail because the limits file has already been read
    let limits_file_dir = Path::new(&limit_file).parent().unwrap();
    let limits_file_path_cloned = limit_file.to_owned();
    // structure needed to keep state of the last known canonical limits file path
    let mut last_known_canonical_path = fs::canonicalize(&limit_file).unwrap();

    let mut watcher = RecommendedWatcher::new(move |result: Result<Event, Error>| match result {
        Ok(ref event) => {
            match event.kind {
                EventKind::Modify(ModifyKind::Data(_)) => {
                    // Content has been changed
                    // Usually happens in local or dockerized envs
                    let location = event.paths.first().unwrap().clone();

                    // Sometimes this event happens in k8s envs when
                    // content source is a configmap and it is replaced
                    // As the move event always occurrs,
                    // skip reloading limit file in this event.

                    // the parent dir is being watched
                    // only reload when the limits file content changed
                    if location == last_known_canonical_path {
                        let limiter = limiter.clone();
                        handle.spawn(async move {
                            match limiter.load_limits_from_file(&location).await {
                                Ok(_) => info!("data modified; reloaded limit file"),
                                Err(e) => error!("Failed reloading limit file: {}", e),
                            }
                        });
                    }
                }
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                    // Move operation occurred
                    // Usually happens in k8s envs when content source is a configmap

                    // symbolic links resolved.
                    let canonical_limit_file = fs::canonicalize(&limits_file_path_cloned).unwrap();
                    // check if the real path to the config file changed
                    // (eg: k8s ConfigMap replacement)
                    if canonical_limit_file != last_known_canonical_path {
                        last_known_canonical_path = canonical_limit_file.clone();
                        let limiter = limiter.clone();
                        handle.spawn(async move {
                            match limiter.load_limits_from_file(&canonical_limit_file).await {
                                Ok(_) => info!("file moved; reloaded limit file"),
                                Err(e) => error!("Failed reloading limit file: {}", e),
                            }
                        });
                    }
                }
                _ => (), // /dev/null
            }

            if let EventKind::Modify(ModifyKind::Data(_)) = event.kind {};
        }
        Err(ref e) => {
            warn!("Something went wrong while watching limit file: {}", e);
        }
    })?;
    watcher.watch(limits_file_dir, RecursiveMode::Recursive)?;

    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
    ));

    info!("HTTP server starting on {}", http_api_address);
    run_http_server(&http_api_address, rate_limiter.clone()).await?;

    Ok(())
}

fn create_config() -> (Configuration, String) {
    let full_version = {
        let build = match LIMITADOR_PROFILE {
            "release" => "".to_owned(),
            other => format!(" {} build", other),
        };

        format!(
            "v{} ({}) {}{}",
            LIMITADOR_VERSION,
            env!("LIMITADOR_GIT_HASH"),
            LIMITADOR_FEATURES.unwrap_or(""),
            build,
        )
    };

    // figure defaults out
    let default_limit_file = env::var("LIMITS_FILE").unwrap_or_else(|_| "".to_string());

    let default_rls_host =
        env::var("ENVOY_RLS_HOST").unwrap_or_else(|_| Configuration::DEFAULT_IP_BIND.to_string());
    let default_rls_port =
        env::var("ENVOY_RLS_PORT").unwrap_or_else(|_| Configuration::DEFAULT_RLS_PORT.to_string());

    let default_http_host =
        env::var("HTTP_API_HOST").unwrap_or_else(|_| Configuration::DEFAULT_IP_BIND.to_string());
    let default_http_port =
        env::var("HTTP_API_PORT").unwrap_or_else(|_| Configuration::DEFAULT_HTTP_PORT.to_string());

    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "".to_string());

    let redis_cached_ttl_default = env::var("REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS")
        .unwrap_or_else(|_| (DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC * 1000).to_string());
    let redis_flushing_period_default = env::var("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS")
        .unwrap_or_else(|_| (DEFAULT_FLUSHING_PERIOD_SEC * 1000).to_string());
    let redis_max_cached_counters_default = DEFAULT_MAX_CACHED_COUNTERS.to_string();
    let redis_ttl_ratio_default = env::var("REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS")
        .unwrap_or_else(|_| DEFAULT_TTL_RATIO_CACHED_COUNTERS.to_string());

    #[cfg(feature = "infinispan")]
    let infinispan_cache_default = env::var("INFINISPAN_CACHE_NAME")
        .unwrap_or_else(|_| DEFAULT_INFINISPAN_LIMITS_CACHE_NAME.to_string());
    #[cfg(feature = "infinispan")]
    let infinispan_consistency_default = env::var("INFINISPAN_COUNTERS_CONSISTENCY")
        .unwrap_or_else(|_| DEFAULT_INFINISPAN_CONSISTENCY.to_string());

    // wire args based of defaults
    let limit_arg = Arg::with_name("LIMITS_FILE")
        .help("The limit file to use")
        .index(1);
    let limit_arg = if default_limit_file.is_empty() {
        limit_arg.required(true)
    } else {
        limit_arg.default_value(&default_limit_file)
    };

    let redis_url_arg = Arg::with_name("URL").help("Redis URL to use").index(1);
    let redis_url_arg = if redis_url.is_empty() {
        redis_url_arg.required(true)
    } else {
        redis_url_arg.default_value(&redis_url)
    };

    // build app
    let cmdline = App::new(LIMITADOR_HEADER)
        .version(full_version.as_str())
        .author("The Kuadrant team - github.com/Kuadrant")
        .about("Rate Limiting Server")
        .disable_help_subcommand(true)
        .subcommand_negates_reqs(false)
        .subcommand_value_name("STORAGE")
        .subcommand_help_heading("STORAGES")
        .subcommand_required(false)
        .arg(limit_arg)
        .arg(
            Arg::with_name("ip")
                .short('b')
                .long("rls-ip")
                .default_value(&default_rls_host)
                .display_order(1)
                .help("The IP to listen on for RLS"),
        )
        .arg(
            Arg::with_name("port")
                .short('p')
                .long("rls-port")
                .default_value(&default_rls_port)
                .display_order(2)
                .help("The port to listen on for RLS"),
        )
        .arg(
            Arg::with_name("http_ip")
                .short('B')
                .long("http-ip")
                .default_value(&default_http_host)
                .display_order(3)
                .help("The IP to listen on for HTTP"),
        )
        .arg(
            Arg::with_name("http_port")
                .short('P')
                .long("http-port")
                .default_value(&default_http_port)
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
                .about("Counters are held in Limitador (ephemeral)"),
        )
        .subcommand(
            SubCommand::with_name("redis")
                .display_order(2)
                .about("Uses Redis to store counters")
                .arg(redis_url_arg.clone()),
        )
        .subcommand(
            SubCommand::with_name("redis_cached")
                .about("Uses Redis to store counters, with an in-memory cache")
                .display_order(3)
                .arg(redis_url_arg)
                .arg(
                    Arg::with_name("TTL")
                        .long("ttl")
                        .takes_value(true)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(&redis_cached_ttl_default)
                        .display_order(2)
                        .help("TTL for cached counters in milliseconds"),
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
                        .help("Flushing period for counters in milliseconds"),
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
        );

    #[cfg(feature = "infinispan")]
    let cmdline = cmdline.subcommand(
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
                    .default_value(&infinispan_cache_default)
                    .display_order(2)
                    .help("Name of the cache to store counters in"),
            )
            .arg(
                Arg::with_name("consistency")
                    .short('c')
                    .long("consistency")
                    .takes_value(true)
                    .default_value(&infinispan_consistency_default)
                    .value_parser(clap::builder::PossibleValuesParser::new(["Strong", "Weak"]))
                    .display_order(3)
                    .help("The consistency to use to read from the cache"),
            ),
    );

    let matches = cmdline.get_matches();

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
        #[cfg(feature = "infinispan")]
        Some(("infinispan", sub)) => {
            StorageConfiguration::Infinispan(InfinispanStorageConfiguration {
                url: sub.value_of("URL").unwrap().to_owned(),
                cache: Some(sub.value_of("cache name").unwrap().to_string()),
                consistency: Some(sub.value_of("consistency").unwrap().to_string()),
            })
        }
        Some(("memory", _sub)) => StorageConfiguration::InMemory,
        None => match storage_config_from_env() {
            Ok(storage_cfg) => storage_cfg,
            Err(_) => {
                eprintln!("Set either REDIS_URL or INFINISPAN_URL, but not both!");
                process::exit(1);
            }
        },
        _ => unreachable!("Some storage wasn't configured!"),
    };

    let mut config = Configuration::with(
        storage,
        limits_file.to_string(),
        matches.value_of("ip").unwrap().into(),
        matches.value_of("port").unwrap().parse().unwrap(),
        matches.value_of("http_ip").unwrap().into(),
        matches.value_of("http_port").unwrap().parse().unwrap(),
        matches.value_of("limit_name_in_labels").is_some()
            || env_option_is_enabled("LIMIT_NAME_IN_PROMETHEUS_LABELS"),
    );

    config.log_level = match matches.occurrences_of("v") {
        0 => None,
        1 => Some(LevelFilter::Warn),
        2 => Some(LevelFilter::Info),
        3 => Some(LevelFilter::Debug),
        4 => Some(LevelFilter::Trace),
        _ => unreachable!("Verbosity should at most be 4!"),
    };

    (config, full_version)
}

fn storage_config_from_env() -> Result<StorageConfiguration, ()> {
    let redis_url = env::var("REDIS_URL");
    let infinispan_url = if cfg!(feature = "infinispan") {
        env::var("INFINISPAN_URL")
    } else {
        Err(VarError::NotPresent)
    };

    match (redis_url, infinispan_url) {
        (Ok(_), Ok(_)) => Err(()),
        (Ok(url), Err(_)) => Ok(StorageConfiguration::Redis(RedisStorageConfiguration {
            url,
            cache: if env_option_is_enabled("REDIS_LOCAL_CACHE_ENABLED") {
                Some(RedisStorageCacheConfiguration {
                    flushing_period: env::var("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS")
                        .unwrap_or_else(|_| (DEFAULT_FLUSHING_PERIOD_SEC * 1000).to_string())
                        .parse()
                        .expect("Expected an i64"),
                    max_ttl: env::var("REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS")
                        .unwrap_or_else(|_| {
                            (DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC * 1000).to_string()
                        })
                        .parse()
                        .expect("Expected an u64"),
                    ttl_ratio: env::var("REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS")
                        .unwrap_or_else(|_| DEFAULT_TTL_RATIO_CACHED_COUNTERS.to_string())
                        .parse()
                        .expect("Expected an u64"),
                    max_counters: DEFAULT_MAX_CACHED_COUNTERS,
                })
            } else {
                None
            },
        })),
        #[cfg(feature = "infinispan")]
        (Err(_), Ok(url)) => Ok(StorageConfiguration::Infinispan(
            InfinispanStorageConfiguration {
                url,
                cache: env::var("INFINISPAN_CACHE_NAME").ok(),
                consistency: env::var("INFINISPAN_COUNTERS_CONSISTENCY").ok(),
            },
        )),
        _ => Ok(StorageConfiguration::InMemory),
    }
}

fn env_option_is_enabled(env_name: &str) -> bool {
    match env::var(env_name) {
        Ok(value) => value == "1",
        Err(_) => false,
    }
}
