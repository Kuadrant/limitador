#![deny(clippy::all, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

#[macro_use]
extern crate tracing;
extern crate clap;

#[cfg(feature = "distributed_storage")]
use crate::config::DistributedStorageConfiguration;
use crate::config::{
    redacted_url, Configuration, DiskStorageConfiguration, InMemoryStorageConfiguration,
    RedisStorageCacheConfiguration, RedisStorageConfiguration, StorageConfiguration,
};
use crate::envoy_rls::server::{run_envoy_rls_server, RateLimitHeaders};
use crate::http_api::server::run_http_server;
use crate::metrics::MetricsLayer;
use chrono::{NaiveDateTime, Utc};
use clap::builder::ValueParser;
#[cfg(feature = "distributed_storage")]
use clap::parser::ValuesRef;
use clap::{value_parser, Arg, ArgAction, Command};
use const_format::formatcp;
use limitador::counter::Counter;
use limitador::errors::LimitadorError;
use limitador::limit::{Expression, Limit};
use limitador::storage::disk::DiskStorage;
use limitador::storage::redis::{
    AsyncRedisStorage, CachedRedisStorage, CachedRedisStorageBuilder, DEFAULT_BATCH_SIZE,
    DEFAULT_FLUSHING_PERIOD_SEC, DEFAULT_MAX_CACHED_COUNTERS, DEFAULT_RESPONSE_TIMEOUT_MS,
};
#[cfg(feature = "distributed_storage")]
use limitador::storage::DistributedInMemoryStorage;
use limitador::storage::{AsyncCounterStorage, AsyncStorage, Storage};
use limitador::{
    storage, AsyncRateLimiter, AsyncRateLimiterBuilder, RateLimiter, RateLimiterBuilder,
};
use notify::event::{CreateKind, ModifyKind, RenameMode};
use notify::{Error, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::{trace, Resource};
use paperclip::actix::Apiv2Schema;
use prometheus_metrics::PrometheusMetrics;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{env, process};
use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use thiserror::Error;
use tokio::runtime::Handle;
use tracing::level_filters::LevelFilter;
use tracing::Subscriber;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{layer::SubscriberExt, Layer};

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
            StorageConfiguration::InMemory(cfg) => Self::in_memory_limiter(cfg),
            #[cfg(feature = "distributed_storage")]
            StorageConfiguration::Distributed(cfg) => Self::distributed_limiter(cfg),
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
        AsyncRedisStorage::new(redis_url)
            .await
            .unwrap_or_else(|err| {
                let redacted_redis_url = redacted_url(String::from(redis_url));
                eprintln!("Failed to connect to Redis at {redacted_redis_url}: {err}");
                process::exit(1)
            })
    }

    async fn storage_using_redis_and_local_cache(
        redis_url: &str,
        cache_cfg: &RedisStorageCacheConfiguration,
    ) -> CachedRedisStorage {
        // TODO: Not all the options are configurable via ENV. Add them as needed.

        let cached_redis_storage = CachedRedisStorageBuilder::new(redis_url)
            .batch_size(cache_cfg.batch_size)
            .flushing_period(Duration::from_millis(cache_cfg.flushing_period as u64))
            .max_cached_counters(cache_cfg.max_counters)
            .response_timeout(Duration::from_millis(cache_cfg.response_timeout));

        cached_redis_storage.build().await.unwrap_or_else(|err| {
            let redacted_redis_url = redacted_url(String::from(redis_url));
            eprintln!("Failed to connect to Redis at {redacted_redis_url}: {err}");
            process::exit(1)
        })
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

    #[cfg(feature = "distributed_storage")]
    fn distributed_limiter(cfg: DistributedStorageConfiguration) -> Self {
        let storage = DistributedInMemoryStorage::new(
            cfg.name,
            cfg.cache_size.or_else(guess_cache_size).unwrap(),
            cfg.listen_address,
            cfg.peer_urls,
        );
        let rate_limiter_builder =
            RateLimiterBuilder::with_storage(Storage::with_counter_storage(Box::new(storage)));

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

#[derive(Copy, Clone, Debug, Serialize, Apiv2Schema)]
pub struct Status {
    pub started_at: NaiveDateTime,
    pub config_applied_at: NaiveDateTime,
    pub config_version: u64,
    pub config_err_since: u64,
}

impl Status {
    pub fn config_success(&mut self) {
        self.config_applied_at = Utc::now().naive_utc();
        self.config_version += 1;
        self.config_err_since = 0;
    }

    pub fn config_failure(&mut self) {
        self.config_err_since += 1;
    }
}

impl Default for Status {
    fn default() -> Self {
        let now = Utc::now().naive_utc();
        Self {
            started_at: now,
            config_applied_at: now,
            config_version: 0,
            config_err_since: 0,
        }
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = {
        let (config, version) = create_config();
        eprintln!("{LIMITADOR_HEADER} {version}");

        configure_tracing_subscriber(&config);

        info!("Version: {}", version);
        info!("Using config: {:?}", config);
        config
    };

    let prometheus_metrics = Arc::new(PrometheusMetrics::new_with_options(
        config.limit_name_in_labels,
        config.metric_labels_default.clone(),
    ));

    let limit_file = config.limits_file.clone();
    let labels_file = config.metric_labels_file.clone();
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

    if let Some(labels_file) = &labels_file {
        match parse_custom_labels_file(Path::new(labels_file)).await {
            Ok(labels) => {
                if let Err(e) = prometheus_metrics.set_custom_labels(labels) {
                    eprintln!("Failed to load labels file: {e}");
                    process::exit(1)
                }
            }
            Err(e) => {
                eprintln!("Failed to load labels file: {e}");
                process::exit(1)
            }
        }
    }

    let limiter = Arc::clone(&rate_limiter);
    let handle = Handle::current();
    // it should not fail because the limits file has already been read
    let mut limits_file_dir = Path::new(&limit_file).parent().unwrap();
    if limits_file_dir.as_os_str().is_empty() {
        limits_file_dir = Path::new(".");
    }
    // structure needed to keep state of the last known canonical limits file path
    let limit_cfg = std::path::absolute(&limit_file).unwrap();
    let mut canonical_cfg = std::fs::canonicalize(&limit_cfg).unwrap();

    let labels_cfg = labels_file.map(std::path::absolute).map(Result::unwrap);
    let mut labels_canonical_cfg = labels_cfg
        .clone()
        .map(std::fs::canonicalize)
        .map(Result::unwrap);
    let labels_file_dir = labels_cfg.clone().map(|f| {
        let mut labels_file_dir: PathBuf = f.parent().unwrap().into();
        if labels_file_dir.as_os_str().is_empty() {
            labels_file_dir = Path::new(".").into();
        }
        labels_file_dir
    });

    let status: Arc<RwLock<Status>> = Arc::default();
    let status_updater = Arc::clone(&status);
    let metrics = prometheus_metrics.clone();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, Error>| match result {
            Ok(ref event) => {
                match event.kind {
                    EventKind::Modify(ModifyKind::Data(_))
                    | EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                    | EventKind::Create(CreateKind::Other) => {
                        if let Some(location) = event.paths.first() {
                            if let Ok(actual_location) = std::fs::canonicalize(&limit_cfg) {
                                if location == &limit_cfg || canonical_cfg != actual_location {
                                    canonical_cfg = actual_location;
                                    let limiter = limiter.clone();
                                    let status_updater = Arc::clone(&status_updater);
                                    let limit_cfg = limit_cfg.clone();

                                    handle.spawn(async move {
                                        match limiter.load_limits_from_file(&limit_cfg).await {
                                            Ok(_) => {
                                                status_updater.write().unwrap().config_success();
                                                info!("data modified; reloaded limit file")
                                            }
                                            Err(e) => {
                                                status_updater.write().unwrap().config_failure();
                                                error!("Failed reloading limit file: {}", e)
                                            }
                                        }
                                    });
                                }
                            }
                            if let Some(labels_cfg) = &labels_cfg {
                                if let Some(location) = event.paths.first() {
                                    if let Ok(actual_location) = std::fs::canonicalize(labels_cfg) {
                                        if location == labels_cfg
                                            || labels_canonical_cfg.as_ref().unwrap()
                                                != &actual_location
                                        {
                                            labels_canonical_cfg = Some(actual_location);
                                            let metrics = metrics.clone();
                                            let labels_cfg = labels_cfg.clone();

                                            handle.spawn(async move {
                                                match parse_custom_labels_file(&labels_cfg).await {
                                                    Ok(labels) => {
                                                        match metrics.set_custom_labels(labels) {
                                                            Ok(_) => {
                                                                info!("custom labels reloaded")
                                                            }
                                                            Err(e) => error!(
                                                                "Failed to set custom labels: {e}"
                                                            ),
                                                        }
                                                    }
                                                    Err(e) => error!(
                                                        "Failed to reload custom labels: {e}"
                                                    ),
                                                }
                                            });
                                        }
                                    }
                                }
                            }
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
    if let Some(labels_file_dir) = labels_file_dir {
        if labels_file_dir.as_path() != limits_file_dir {
            watcher.watch(&labels_file_dir, RecursiveMode::Recursive)?;
        }
    }

    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
        rate_limit_headers,
        prometheus_metrics.clone(),
        grpc_reflection_service,
    ));

    info!("HTTP server starting on {}", http_api_address);
    run_http_server(
        &http_api_address,
        rate_limiter.clone(),
        prometheus_metrics,
        status,
    )
    .await?;

    Ok(())
}

async fn parse_custom_labels_file(path: &Path) -> Result<HashMap<String, Expression>, String> {
    match std::fs::File::open(path) {
        Ok(f) => {
            let parsed_labels: Result<HashMap<String, String>, _> = serde_yaml::from_reader(f);
            match parsed_labels {
                Ok(labels) => Ok(labels
                    .into_iter()
                    .filter_map(|(k, v)| match Expression::parse(&v) {
                        Ok(expr) => Some((k, expr)),
                        Err(e) => {
                            error!("Failed to parse custom label `{k}` `{v}`: {e}");
                            None
                        }
                    })
                    .collect()),
                Err(e) => Err(format!("Couldn't parse: {e}")),
            }
        }
        Err(e) => Err(format!("Couldn't read file '{}': {e}", path.display())),
    }
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
                .display_order(10)
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
                .display_order(20)
                .help("The port to listen on for RLS"),
        )
        .arg(
            Arg::new("http_ip")
                .short('B')
                .long("http-ip")
                .default_value(config::env::HTTP_API_HOST.unwrap_or(Configuration::DEFAULT_IP_BIND))
                .display_order(30)
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
                .display_order(40)
                .help("The port to listen on for HTTP"),
        )
        .arg(
            Arg::new("limit_name_in_labels")
                .short('l')
                .long("limit-name-in-labels")
                .action(ArgAction::SetTrue)
                .display_order(50)
                .help("Include the Limit Name in prometheus label"),
        )
        .arg(
            Arg::new("custom_metric_labels")
                .short('L')
                .long("custom-metric-labels")
                .action(ArgAction::Set)
                .value_parser(value_parser!(String))
                .display_order(55)
                .help("File with custom labels for prometheus metrics"),
        )
        .arg(
            Arg::new("metric_labels_default")
                .long("metric-labels-default")
                .action(ArgAction::Set)
                .value_parser(ValueParser::new(|arg: &str| {
                    Expression::parse(arg.to_owned())
                }))
                .display_order(56)
                .help("A CEL expression resolving to a Map with labels & their values to use"),
        )
        .arg(
            Arg::new("tracing_endpoint")
                .long("tracing-endpoint")
                .default_value(config::env::TRACING_ENDPOINT.unwrap_or(""))
                .display_order(60)
                .help("The host for the tracing service"),
        )
        .arg(
            Arg::new("v")
                .short('v')
                .action(ArgAction::Count)
                .value_parser(value_parser!(u8).range(..5))
                .display_order(70)
                .help("Sets the level of verbosity"),
        )
        .arg(
            Arg::new("validate")
                .long("validate")
                .action(ArgAction::SetTrue)
                .display_order(80)
                .help("Validates the LIMITS_FILE and exits"),
        )
        .arg(
            Arg::new("rate_limit_headers")
                .long("rate-limit-headers")
                .short('H')
                .display_order(90)
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
                .display_order(100)
                .help("Enables gRPC server reflection service"),
        )
        .subcommand(
            Command::new("memory")
                .display_order(10)
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
                .display_order(20)
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
                .display_order(30)
                .about("Uses Redis to store counters")
                .arg(redis_url_arg.clone()),
        )
        .subcommand(
            Command::new("redis_cached")
                .about("Uses Redis to store counters, with an in-memory cache")
                .display_order(40)
                .arg(redis_url_arg)
                .arg(
                    Arg::new("batch")
                        .long("batch-size")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(usize))
                        .default_value(
                            config::env::REDIS_LOCAL_CACHE_BATCH_SIZE
                                .unwrap_or(leak(DEFAULT_BATCH_SIZE)),
                        )
                        .display_order(30)
                        .help("Size of entries to flush in as single flush"),
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
                        .display_order(40)
                        .help("Flushing period for counters in milliseconds"),
                )
                .arg(
                    Arg::new("max")
                        .long("max-cached")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(usize))
                        .default_value(leak(DEFAULT_MAX_CACHED_COUNTERS))
                        .display_order(50)
                        .help("Maximum amount of counters cached"),
                )
                .arg(
                    Arg::new("timeout")
                        .long("response-timeout")
                        .action(ArgAction::Set)
                        .value_parser(clap::value_parser!(u64))
                        .default_value(leak(DEFAULT_RESPONSE_TIMEOUT_MS))
                        .display_order(60)
                        .help("Timeout for Redis commands in milliseconds"),
                ),
        );

    #[cfg(feature = "distributed_storage")]
    let cmdline = cmdline.subcommand(
        Command::new("distributed")
            .about("Replicates CRDT-based counters across multiple Limitador servers")
            .display_order(5)
            .arg(
                Arg::new("NAME")
                    .action(ArgAction::Set)
                    .required(true)
                    .display_order(2)
                    .help("Unique name to identify this Limitador instance"),
            )
            .arg(
                Arg::new("LISTEN_ADDRESS")
                    .action(ArgAction::Set)
                    .required(true)
                    .display_order(2)
                    .help("Local IP:PORT to listen on for replication"),
            )
            .arg(
                Arg::new("PEER_URLS")
                    .action(ArgAction::Append)
                    .required(false)
                    .display_order(3)
                    .help("A replication peer url that this instance will connect to"),
            )
            .arg(
                Arg::new("CACHE_SIZE")
                    .long("cache")
                    .short('c')
                    .action(ArgAction::Set)
                    .value_parser(value_parser!(u64))
                    .display_order(4)
                    .help("Sets the size of the cache for 'qualified counters'"),
            ),
    );

    let matches = cmdline.get_matches();

    let limits_file = matches.get_one::<String>("LIMITS_FILE").unwrap();

    if matches.get_flag("validate") {
        let error = match std::fs::File::open(limits_file) {
            Ok(f) => {
                let parsed_limits: Result<Vec<Limit>, _> = serde_yaml::from_reader(f);
                match parsed_limits {
                    Ok(limits) => {
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
                batch_size: *sub.get_one("batch").unwrap(),
                flushing_period: *sub.get_one("flush").unwrap(),
                max_counters: *sub.get_one("max").unwrap(),
                response_timeout: *sub.get_one("timeout").unwrap(),
            }),
        }),
        Some(("memory", sub)) => StorageConfiguration::InMemory(InMemoryStorageConfiguration {
            cache_size: sub.get_one::<u64>("CACHE_SIZE").copied(),
        }),
        #[cfg(feature = "distributed_storage")]
        Some(("distributed", sub)) => {
            StorageConfiguration::Distributed(DistributedStorageConfiguration {
                name: sub.get_one::<String>("NAME").unwrap().to_owned(),
                listen_address: sub.get_one::<String>("LISTEN_ADDRESS").unwrap().to_owned(),
                peer_urls: sub
                    .get_many::<String>("PEER_URLS")
                    .unwrap_or(ValuesRef::default())
                    .map(|x| x.to_owned())
                    .collect(),
                cache_size: sub.get_one::<u64>("CACHE_SIZE").copied(),
            })
        }
        None => storage_config_from_env(),
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
        matches.get_flag("limit_name_in_labels") || *config::env::LIMIT_NAME_IN_PROMETHEUS_LABELS,
        matches.get_one::<String>("custom_metric_labels").cloned(),
        matches
            .get_one::<Expression>("metric_labels_default")
            .cloned(),
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

fn storage_config_from_env() -> StorageConfiguration {
    if let Some(url) = config::env::REDIS_URL.map(str::to_owned) {
        StorageConfiguration::Redis(RedisStorageConfiguration {
            url,
            cache: if *config::env::REDIS_LOCAL_CACHE_ENABLED {
                Some(RedisStorageCacheConfiguration {
                    batch_size: config::env::REDIS_LOCAL_CACHE_BATCH_SIZE
                        .map(str::to_owned)
                        .unwrap_or_else(|| (DEFAULT_BATCH_SIZE).to_string())
                        .parse()
                        .expect("Expected an usize"),
                    flushing_period: config::env::REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS
                        .map(str::to_owned)
                        .unwrap_or_else(|| (DEFAULT_FLUSHING_PERIOD_SEC * 1000).to_string())
                        .parse()
                        .expect("Expected an i64"),
                    max_counters: DEFAULT_MAX_CACHED_COUNTERS,
                    response_timeout: DEFAULT_RESPONSE_TIMEOUT_MS,
                })
            } else {
                None
            },
        })
    } else {
        StorageConfiguration::InMemory(InMemoryStorageConfiguration { cache_size: None })
    }
}

fn guess_cache_size() -> Option<u64> {
    let sys = System::new_with_specifics(
        RefreshKind::new().with_memory(MemoryRefreshKind::everything().without_swap()),
    );
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

fn leak<D: Display>(s: D) -> &'static str {
    Box::leak(format!("{s}").into_boxed_str())
}

fn configure_tracing_subscriber(config: &Configuration) {
    let level = config.log_level.unwrap_or_else(|| {
        tracing_subscriber::filter::EnvFilter::from_default_env()
            .max_level_hint()
            .unwrap_or(LevelFilter::ERROR)
    });

    let metrics_layer = MetricsLayer::default()
        .gather(
            "should_rate_limit",
            PrometheusMetrics::record_datastore_latency,
            vec!["datastore"],
        )
        .gather(
            "flush_batcher_and_update_counters",
            PrometheusMetrics::record_datastore_latency,
            vec!["datastore"],
        );

    if !config.tracing_endpoint.is_empty() {
        // Init tracing subscriber with telemetry
        // If running in memory initialize without metrics
        match config.storage {
            StorageConfiguration::InMemory(_) => tracing_subscriber::registry()
                .with(fmt_layer(level))
                .with(telemetry_layer(&config.tracing_endpoint, level))
                .init(),
            _ => tracing_subscriber::registry()
                .with(metrics_layer)
                .with(fmt_layer(level))
                .with(telemetry_layer(&config.tracing_endpoint, level))
                .init(),
        }
    } else {
        // If running in memory initialize without metrics
        match config.storage {
            StorageConfiguration::InMemory(_) => {
                tracing_subscriber::registry().with(fmt_layer(level)).init()
            }
            _ => tracing_subscriber::registry()
                .with(metrics_layer)
                .with(fmt_layer(level))
                .init(),
        }
    }
}

fn fmt_layer<S>(level: LevelFilter) -> impl Layer<S>
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::fmt::layer()
        .with_span_events(if level >= LevelFilter::DEBUG {
            FmtSpan::CLOSE
        } else {
            FmtSpan::NONE
        })
        .with_filter(level)
}

fn telemetry_layer<S>(tracing_endpoint: &String, level: LevelFilter) -> impl Layer<S>
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    global::set_text_map_propagator(TraceContextPropagator::new());

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(tracing_endpoint),
        )
        .with_trace_config(
            trace::config().with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                "limitador",
            )])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .expect("error installing tokio tracing exporter");

    // Set the level to minimum info if tracing enabled
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(level.max(LevelFilter::INFO))
}
