#![deny(clippy::all, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

#[macro_use]
extern crate log;
extern crate clap;

#[cfg(feature = "infinispan")]
use crate::config::InfinispanStorageConfiguration;
use crate::config::{
    Configuration, DiskStorageConfiguration, InMemoryStorageConfiguration,
    RedisStorageCacheConfiguration, RedisStorageConfiguration, StorageConfiguration,
};
use crate::envoy_rls::server::{run_envoy_rls_server, RateLimitHeaders};
use crate::http_api::server::run_http_server;
use crate::metrics::MetricsLayer;
use clap::{value_parser, Arg, ArgAction, Command};
use const_format::formatcp;
use limitador::counter::Counter;
use limitador::errors::LimitadorError;
use limitador::limit::Limit;
use limitador::storage::disk::DiskStorage;
#[cfg(feature = "infinispan")]
use limitador::storage::infinispan::{Consistency, InfinispanStorageBuilder};
use limitador::storage::redis::{
    AsyncRedisStorage, CachedRedisStorage, CachedRedisStorageBuilder, DEFAULT_FLUSHING_PERIOD_SEC,
    DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC, DEFAULT_RESPONSE_TIMEOUT_MS,
    DEFAULT_TTL_RATIO_CACHED_COUNTERS,
};
use limitador::storage::{AsyncCounterStorage, AsyncStorage, Storage};
use limitador::{
    storage, AsyncRateLimiter, AsyncRateLimiterBuilder, RateLimiter, RateLimiterBuilder,
};
use notify::event::{ModifyKind, RenameMode};
use notify::{Error, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::{trace, Resource};
use prometheus_metrics::PrometheusMetrics;
use std::env::VarError;
use std::fmt::Display;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::{env, process};
use sysinfo::{RefreshKind, System, SystemExt};
use thiserror::Error;
use tokio::runtime::Handle;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod envoy_rls;
mod http_api;

mod config;
mod metrics;
pub mod prometheus_metrics;

const LIMITADOR_VERSION: &str = env!("CARGO_PKG_VERSION");
const LIMITADOR_PROFILE: &str = env!("LIMITADOR_PROFILE");
const LIMITADOR_FEATURES: &str = env!("LIMITADOR_FEATURES");
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
            StorageConfiguration::Redis(cfg) => Self::redis_limiter(cfg).await,
            #[cfg(feature = "infinispan")]
            StorageConfiguration::Infinispan(cfg) => Self::infinispan_limiter(cfg).await,
            StorageConfiguration::InMemory(cfg) => Self::in_memory_limiter(cfg),
            StorageConfiguration::Disk(cfg) => Self::disk_limiter(cfg),
        };

        Ok(rate_limiter)
    }

    async fn redis_limiter(cfg: RedisStorageConfiguration) -> Self {
        let storage = Self::storage_using_redis(cfg).await;
        let rate_limiter_builder = AsyncRateLimiterBuilder::new(storage);

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
        match AsyncRedisStorage::new(redis_url).await {
            Ok(storage) => storage,
            Err(err) => {
                eprintln!("Failed to connect to Redis at {redis_url}: {err}");
                process::exit(1)
            }
        }
    }

    async fn storage_using_redis_and_local_cache(
        redis_url: &str,
        cache_cfg: &RedisStorageCacheConfiguration,
    ) -> CachedRedisStorage {
        // TODO: Not all the options are configurable via ENV. Add them as needed.

        let cached_redis_storage = CachedRedisStorageBuilder::new(redis_url)
            .flushing_period(Duration::from_millis(cache_cfg.flushing_period as u64))
            .max_ttl_cached_counters(Duration::from_millis(cache_cfg.max_ttl))
            .ttl_ratio_cached_counters(cache_cfg.ttl_ratio)
            .max_cached_counters(cache_cfg.max_counters)
            .response_timeout(Duration::from_millis(cache_cfg.response_timeout));

        cached_redis_storage.build().await.unwrap_or_else(|err| {
            eprintln!("Failed to connect to Redis at {redis_url}: {err}");
            process::exit(1)
        })
    }

    #[cfg(feature = "infinispan")]
    async fn infinispan_limiter(cfg: InfinispanStorageConfiguration) -> Self {
        use url::Url;

        let parsed_url = Url::parse(&cfg.url).unwrap();

        let mut builder = InfinispanStorageBuilder::new(
            format!(
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

        let rate_limiter_builder =
            AsyncRateLimiterBuilder::new(AsyncStorage::with_counter_storage(Box::new(storage)));

        Self::Async(rate_limiter_builder.build())
    }

    fn disk_limiter(cfg: DiskStorageConfiguration) -> Self {
        let storage = match DiskStorage::open(cfg.path.as_str(), cfg.optimization) {
            Ok(storage) => storage,
            Err(err) => {
                eprintln!("Failed to open DB at {}: {err}", cfg.path);
                process::exit(1)
            }
        };
        let rate_limiter_builder =
            RateLimiterBuilder::with_storage(Storage::with_counter_storage(Box::new(storage)));

        Self::Blocking(rate_limiter_builder.build())
    }

    fn in_memory_limiter(cfg: InMemoryStorageConfiguration) -> Self {
        let rate_limiter_builder =
            RateLimiterBuilder::new(cfg.cache_size.or_else(guess_cache_size).unwrap());

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
                    Ok(limits) => match find_first_negative_limit(&limits) {
                        None => {
                            match &self {
                                Self::Blocking(limiter) => limiter.configure_with(limits)?,
                                Self::Async(limiter) => limiter.configure_with(limits).await?,
                            }
                            if limitador::limit::check_deprecated_syntax_usages_and_reset() {
                                error!("You are using deprecated syntax for your conditions! See the migration guide https://docs.kuadrant.io/limitador/doc/migrations/conditions/")
                            }
                            Ok(())
                        }
                        Some(index) => Err(LimitadorServerError::ConfigFile(format!(
                            ".[{index}]: invalid value for `max_value`: positive integer expected"
                        ))),
                    },
                    Err(e) => Err(LimitadorServerError::ConfigFile(format!(
                        "Couldn't parse: {e}"
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

fn find_first_negative_limit(limits: &[Limit]) -> Option<usize> {
    for (index, limit) in limits.iter().enumerate() {
        if limit.max_value() < 0 {
            return Some(index);
        }
    }
    None
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (config, prometheus_metrics) = {
        let (config, version) = create_config();
        println!("{LIMITADOR_HEADER} {version}");
        let level = config.log_level.unwrap_or_else(|| {
            tracing_subscriber::filter::EnvFilter::from_default_env()
                .max_level_hint()
                .unwrap_or(LevelFilter::ERROR)
        });
        let fmt_layer = if level >= LevelFilter::DEBUG {
            tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE)
        } else {
            tracing_subscriber::fmt::layer()
        };

        let limit_name_in_metrics = config.limit_name_in_labels;
        let prometheus_metrics =
            Arc::new(PrometheusMetrics::new_with_options(limit_name_in_metrics));
        let metrics = prometheus_metrics.clone();

        let metrics_layer = MetricsLayer::new().gather(
            "should_rate_limit",
            move |timings| metrics.counter_access(Duration::from(timings)),
            vec!["datastore"],
        );

        if !config.tracing_endpoint.is_empty() {
            global::set_text_map_propagator(TraceContextPropagator::new());
            let tracer = opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .tonic()
                        .with_endpoint(config.tracing_endpoint.clone()),
                )
                .with_trace_config(trace::config().with_resource(Resource::new(vec![
                    KeyValue::new("service.name", "limitador"),
                ])))
                .install_batch(opentelemetry_sdk::runtime::Tokio)?;
            let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            tracing_subscriber::registry()
                .with(level)
                .with(metrics_layer)
                .with(fmt_layer)
                .with(telemetry_layer)
                .init();
        } else {
            tracing_subscriber::registry()
                .with(level)
                .with(metrics_layer)
                .with(fmt_layer)
                .init();
        };

        prometheus_metrics.set_use_limit_name_in_label(limit_name_in_metrics);

        info!("Version: {}", version);
        info!("Using config: {:?}", config);
        (config, prometheus_metrics)
    };

    let limit_file = config.limits_file.clone();
    let envoy_rls_address = config.rlp_address();
    let http_api_address = config.http_address();
    let rate_limit_headers = config.rate_limit_headers.clone();
    let grpc_reflection_service = config.grpc_reflection_service;

    let rate_limiter: Arc<Limiter> = match Limiter::new(config).await {
        Ok(limiter) => Arc::new(limiter),
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1)
        }
    };

    info!("limits file path: {}", limit_file);
    if let Err(e) = rate_limiter.load_limits_from_file(&limit_file).await {
        eprintln!("Failed to load limit file: {e}");
        process::exit(1)
    }

    let limiter = Arc::clone(&rate_limiter);
    let handle = Handle::current();
    // it should not fail because the limits file has already been read
    let mut limits_file_dir = Path::new(&limit_file).parent().unwrap();
    if limits_file_dir.as_os_str().is_empty() {
        limits_file_dir = Path::new(".");
    }
    let limits_file_path_cloned = limit_file.to_owned();
    // structure needed to keep state of the last known canonical limits file path
    let mut last_known_canonical_path = fs::canonicalize(&limit_file).unwrap();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, Error>| match result {
            Ok(ref event) => {
                match event.kind {
                    EventKind::Modify(ModifyKind::Data(_)) => {
                        // Content has been changed
                        // Usually happens in local or dockerized envs
                        let location = event.paths.first().unwrap().clone();

                        // Sometimes this event happens in k8s envs when
                        // content source is a configmap and it is replaced
                        // As the move event always occurs,
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
                        let canonical_limit_file =
                            fs::canonicalize(&limits_file_path_cloned).unwrap();
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
            }
            Err(ref e) => {
                warn!("Something went wrong while watching limit file: {}", e);
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(limits_file_dir, RecursiveMode::Recursive)?;

    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
        rate_limit_headers,
        prometheus_metrics.clone(),
        grpc_reflection_service,
    ));

    info!("HTTP server starting on {}", http_api_address);
    run_http_server(&http_api_address, rate_limiter.clone(), prometheus_metrics).await?;

    Ok(())
}

fn create_config() -> (Configuration, &'static str) {
    let full_version: &'static str = formatcp!(
        "v{} ({}) {} {}",
        LIMITADOR_VERSION,
        env!("LIMITADOR_GIT_HASH"),
        LIMITADOR_FEATURES,
        LIMITADOR_PROFILE,
    );

    // wire args based of defaults
    let limit_arg = Arg::new("LIMITS_FILE")
        .action(ArgAction::Set)
        .help("The limit file to use")
        .index(1);
    let limit_arg = match *config::env::LIMITS_FILE {
        None => limit_arg.required(true),
        Some(file) => limit_arg.default_value(file),
    };

    let redis_url_arg = Arg::new("URL").help("Redis URL to use").index(1);
    let redis_url_arg = match *config::env::REDIS_URL {
        None => redis_url_arg.required(true),
        Some(url) => redis_url_arg.default_value(url),
    };

    let disk_path_arg = Arg::new("PATH").help("Path to counter DB").index(1);
    let disk_path_arg = match *config::env::DISK_PATH {
        None => disk_path_arg.required(true),
        Some(path) => disk_path_arg.default_value(path),
    };

    // build app
    let cmdline = Command::new(LIMITADOR_HEADER)
        .version(full_version)
        .author("The Kuadrant team - github.com/Kuadrant")
        .about("Rate Limiting Server")
        .disable_help_subcommand(true)
        .subcommand_negates_reqs(false)
        .subcommand_value_name("STORAGE")
        .subcommand_help_heading("STORAGES")
        .subcommand_required(false)
        .arg(limit_arg)
        .arg(
            Arg::new("ip")
                .short('b')
                .long("rls-ip")
                .default_value(
                    config::env::ENVOY_RLS_HOST.unwrap_or(Configuration::DEFAULT_IP_BIND),
                )
                .display_order(1)
                .help("The IP to listen on for RLS"),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("rls-port")
                .default_value(
                    config::env::ENVOY_RLS_PORT.unwrap_or(Configuration::DEFAULT_RLS_PORT),
                )
                .value_parser(value_parser!(u16))
                .display_order(2)
                .help("The port to listen on for RLS"),
        )
        .arg(
            Arg::new("http_ip")
                .short('B')
                .long("http-ip")
                .default_value(config::env::HTTP_API_HOST.unwrap_or(Configuration::DEFAULT_IP_BIND))
                .display_order(3)
                .help("The IP to listen on for HTTP"),
        )
        .arg(
            Arg::new("http_port")
                .short('P')
                .long("http-port")
                .default_value(
                    config::env::HTTP_API_PORT.unwrap_or(Configuration::DEFAULT_HTTP_PORT),
                )
                .value_parser(value_parser!(u16))
                .display_order(4)
                .help("The port to listen on for HTTP"),
        )
        .arg(
            Arg::new("limit_name_in_labels")
                .short('l')
                .long("limit-name-in-labels")
                .action(ArgAction::SetTrue)
                .display_order(5)
                .help("Include the Limit Name in prometheus label"),
        )
        .arg(
            Arg::new("tracing_endpoint")
                .long("tracing-endpoint")
                .default_value(config::env::TRACING_ENDPOINT.unwrap_or(""))
                .display_order(6)
                .help("The host for the tracing service"),
        )
        .arg(
            Arg::new("v")
                .short('v')
                .action(ArgAction::Count)
                .value_parser(value_parser!(u8).range(..5))
                .display_order(7)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("validate")
                .long("validate")
                .action(ArgAction::SetTrue)
                .display_order(8)
                .help("Validates the LIMITS_FILE and exits"),
        )
        .arg(
            Arg::new("rate_limit_headers")
                .long("rate-limit-headers")
                .short('H')
                .display_order(9)
                .default_value(config::env::RATE_LIMIT_HEADERS.unwrap_or("NONE"))
                .value_parser(clap::builder::PossibleValuesParser::new([
                    "NONE",
                    "DRAFT_VERSION_03",
                ]))
                .help("Enables rate limit response headers"),
        )
        .arg(
            Arg::new("grpc_reflection_service")
                .long("grpc-reflection-service")
                .action(ArgAction::SetTrue)
                .display_order(10)
                .help("Enables gRPC server reflection service"),
        )
        .subcommand(
            Command::new("memory")
                .display_order(1)
                .about("Counters are held in Limitador (ephemeral)")
                .arg(
                    Arg::new("CACHE_SIZE")
                        .long("cache")
                        .short('c')
                        .action(ArgAction::Set)
                        .value_parser(value_parser!(u64))
                        .display_order(1)
                        .help("Sets the size of the cache for 'qualified counters'"),
                ),
        )
        .subcommand(
            Command::new("disk")
                .display_order(2)
                .about("Counters are held on disk (persistent)")
                .arg(disk_path_arg)
                .arg(
                    Arg::new("OPTIMIZE")
                        .long("optimize")
                        .action(ArgAction::Set)
                        .display_order(1)
                        .default_value(config::env::DISK_OPTIMIZE.unwrap_or("throughput"))
                        .value_parser(clap::builder::PossibleValuesParser::new([
                            "throughput",
                            "disk",
                        ]))
                        .help("Optimizes either to save disk space or higher throughput"),
                ),
        )
        .subcommand(
            Command::new("redis")
                .display_order(3)
                .about("Uses Redis to store counters")
                .arg(redis_url_arg.clone()),
        )
        .subcommand(
            Command::new("redis_cached")
                .about("Uses Redis to store counters, with an in-memory cache")
                .display_order(4)
                .arg(redis_url_arg)
                .arg(
                    Arg::new("TTL")
                        .long("ttl")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(
                            config::env::REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS
                                .unwrap_or(leak(DEFAULT_MAX_TTL_CACHED_COUNTERS_SEC * 1000)),
                        )
                        .display_order(2)
                        .help("TTL for cached counters in milliseconds"),
                )
                .arg(
                    Arg::new("ratio")
                        .long("ratio")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(
                            config::env::REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS
                                .unwrap_or(leak(DEFAULT_TTL_RATIO_CACHED_COUNTERS)),
                        )
                        .display_order(3)
                        .help("Ratio to apply to the TTL from Redis on cached counters"),
                )
                .arg(
                    Arg::new("flush")
                        .long("flush-period")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(i64))
                        .default_value(
                            config::env::REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS
                                .unwrap_or(leak(DEFAULT_FLUSHING_PERIOD_SEC * 1000)),
                        )
                        .display_order(4)
                        .help("Flushing period for counters in milliseconds"),
                )
                .arg(
                    Arg::new("max")
                        .long("max-cached")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(usize))
                        .default_value(leak(DEFAULT_MAX_CACHED_COUNTERS))
                        .display_order(5)
                        .help("Maximum amount of counters cached"),
                )
                .arg(
                    Arg::new("timeout")
                        .long("response-timeout")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(leak(DEFAULT_RESPONSE_TIMEOUT_MS))
                        .display_order(6)
                        .help("Timeout for Redis commands in milliseconds"),
                ),
        );

    #[cfg(feature = "infinispan")]
    let cmdline = cmdline.subcommand(
        Command::new("infinispan")
            .about("Uses Infinispan to store counters")
            .display_order(5)
            .arg(
                Arg::new("URL")
                    .help("Infinispan URL to use")
                    .display_order(1)
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("cache name")
                    .short('n')
                    .long("cache-name")
                    .action(ArgAction::Set)
                    .default_value(
                        config::env::INFINISPAN_CACHE_NAME
                            .unwrap_or(storage::infinispan::DEFAULT_INFINISPAN_LIMITS_CACHE_NAME),
                    )
                    .display_order(2)
                    .help("Name of the cache to store counters in"),
            )
            .arg(
                Arg::new("consistency")
                    .short('c')
                    .long("consistency")
                    .action(ArgAction::Set)
                    .default_value(
                        config::env::INFINISPAN_COUNTERS_CONSISTENCY
                            .unwrap_or(leak(storage::infinispan::DEFAULT_INFINISPAN_CONSISTENCY)),
                    )
                    .value_parser(clap::builder::PossibleValuesParser::new(["Strong", "Weak"]))
                    .display_order(3)
                    .help("The consistency to use to read from the cache"),
            ),
    );

    let matches = cmdline.get_matches();

    let limits_file = matches.get_one::<String>("LIMITS_FILE").unwrap();

    if matches.get_flag("validate") {
        let error = match std::fs::File::open(limits_file) {
            Ok(f) => {
                let parsed_limits: Result<Vec<Limit>, _> = serde_yaml::from_reader(f);
                match parsed_limits {
                    Ok(limits) => match find_first_negative_limit(&limits) {
                        Some(index) => LimitadorServerError::ConfigFile(format!(
                            ".[{index}]: invalid value for `max_value`: positive integer expected"
                        )),
                        None => {
                            if limitador::limit::check_deprecated_syntax_usages_and_reset() {
                                eprintln!("Deprecated syntax for conditions corrected!\n")
                            }

                            let output: Vec<http_api::LimitVO> =
                                limits.iter().map(|l| l.into()).collect();
                            match serde_yaml::to_string(&output) {
                                Ok(cfg) => {
                                    println!("{cfg}");
                                }
                                Err(err) => {
                                    eprintln!("Config file is valid, but can't be output: {err}");
                                }
                            }
                            process::exit(0);
                        }
                    },
                    Err(e) => LimitadorServerError::ConfigFile(format!("Couldn't parse: {e}")),
                }
            }
            Err(e) => {
                LimitadorServerError::ConfigFile(format!("Couldn't read file '{limits_file}': {e}"))
            }
        };
        eprintln!("{error}");
        process::exit(1);
    }

    let storage = match matches.subcommand() {
        Some(("redis", sub)) => StorageConfiguration::Redis(RedisStorageConfiguration {
            url: sub.get_one::<String>("URL").unwrap().to_owned(),
            cache: None,
        }),
        Some(("disk", sub)) => StorageConfiguration::Disk(DiskStorageConfiguration {
            path: sub
                .get_one::<String>("PATH")
                .expect("We need a path!")
                .to_string(),
            optimization: match sub.get_one::<String>("OPTIMIZE").map(String::as_str) {
                Some("disk") => storage::disk::OptimizeFor::Space,
                Some("throughput") => storage::disk::OptimizeFor::Throughput,
                _ => unreachable!("Some disk OptimizeFor wasn't configured!"),
            },
        }),
        Some(("redis_cached", sub)) => StorageConfiguration::Redis(RedisStorageConfiguration {
            url: sub.get_one::<String>("URL").unwrap().to_owned(),
            cache: Some(RedisStorageCacheConfiguration {
                flushing_period: *sub.get_one("flush").unwrap(),
                max_ttl: *sub.get_one("TTL").unwrap(),
                ttl_ratio: *sub.get_one("ratio").unwrap(),
                max_counters: *sub.get_one("max").unwrap(),
                response_timeout: *sub.get_one("timeout").unwrap(),
            }),
        }),
        #[cfg(feature = "infinispan")]
        Some(("infinispan", sub)) => {
            StorageConfiguration::Infinispan(InfinispanStorageConfiguration {
                url: sub.get_one::<String>("URL").unwrap().to_owned(),
                cache: Some(sub.get_one::<String>("cache name").unwrap().to_string()),
                consistency: Some(sub.get_one::<String>("consistency").unwrap().to_string()),
            })
        }
        Some(("memory", sub)) => StorageConfiguration::InMemory(InMemoryStorageConfiguration {
            cache_size: sub.get_one::<u64>("CACHE_SIZE").copied(),
        }),
        None => match storage_config_from_env() {
            Ok(storage_cfg) => storage_cfg,
            Err(_) => {
                eprintln!("Set either REDIS_URL or INFINISPAN_URL, but not both!");
                process::exit(1);
            }
        },
        _ => unreachable!("Some storage wasn't configured!"),
    };

    let rate_limit_headers = match matches
        .get_one::<String>("rate_limit_headers")
        .unwrap()
        .as_str()
    {
        "NONE" => RateLimitHeaders::None,
        "DRAFT_VERSION_03" => RateLimitHeaders::DraftVersion03,
        _ => unreachable!("invalid --rate-limit-headers value"),
    };

    let mut config = Configuration::with(
        storage,
        limits_file.to_string(),
        matches.get_one::<String>("ip").unwrap().into(),
        *matches.get_one::<u16>("port").unwrap(),
        matches.get_one::<String>("http_ip").unwrap().into(),
        *matches.get_one::<u16>("http_port").unwrap(),
        matches.get_flag("limit_name_in_labels")
            || env_option_is_enabled("LIMIT_NAME_IN_PROMETHEUS_LABELS"),
        matches
            .get_one::<String>("tracing_endpoint")
            .unwrap()
            .into(),
        rate_limit_headers,
        matches.get_flag("grpc_reflection_service"),
    );

    config.log_level = match matches.get_count("v") {
        0 => None,
        1 => Some(LevelFilter::WARN),
        2 => Some(LevelFilter::INFO),
        3 => Some(LevelFilter::DEBUG),
        4 => Some(LevelFilter::TRACE),
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
                    response_timeout: DEFAULT_RESPONSE_TIMEOUT_MS,
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
        _ => Ok(StorageConfiguration::InMemory(
            InMemoryStorageConfiguration { cache_size: None },
        )),
    }
}

fn guess_cache_size() -> Option<u64> {
    let sys = System::new_with_specifics(RefreshKind::new().with_memory());
    let free_mem = sys.available_memory();
    let memory = free_mem as f64 * 0.7;
    let size = (memory
        / (std::mem::size_of::<Counter>() + 16/* size_of::<AtomicExpiringValue>() */) as f64)
        as u64;
    warn!(
        "No cache size provided, aiming at 70% of {}MB, i.e. {size} entries",
        free_mem / 1024 / 1024
    );
    Some(size)
}

fn env_option_is_enabled(env_name: &str) -> bool {
    match env::var(env_name) {
        Ok(value) => value == "1",
        Err(_) => false,
    }
}

fn leak<D: Display>(s: D) -> &'static str {
    return Box::leak(format!("{}", s).into_boxed_str());
}

#[cfg(test)]
mod tests {
    use crate::find_first_negative_limit;
    use limitador::limit::Limit;

    #[test]
    fn finds_negative_limits() {
        let variables: [&str; 0] = [];
        let mut limits: Vec<Limit> = vec![
            Limit::new::<_, &str>("foo", 42, 10, [], variables),
            Limit::new::<_, &str>("foo", -42, 10, [], variables),
        ];

        assert_eq!(find_first_negative_limit(&limits), Some(1));
        limits[0].set_max_value(-42);
        assert_eq!(find_first_negative_limit(&limits), Some(0));
        limits[1].set_max_value(42);
        assert_eq!(find_first_negative_limit(&limits), Some(0));
        limits[0].set_max_value(42);
        assert_eq!(find_first_negative_limit(&limits), None);

        let nothing: [Limit; 0] = [];
        assert_eq!(find_first_negative_limit(&nothing), None);
    }
}
