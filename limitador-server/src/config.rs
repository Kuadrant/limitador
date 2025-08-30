// LIMITS_FILE: Path
//
// LIMIT_NAME_IN_PROMETHEUS_LABELS: bool
//
// REDIS_URL: StorageType { String }
// └ REDIS_LOCAL_CACHE_ENABLED: bool
//   └ REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS: i64 ?!
//   └ REDIS_LOCAL_CACHE_BATCH_SIZE: u64
//
// ENVOY_RLS_HOST: host // just to become ENVOY_RLS_HOST:ENVOY_RLS_PORT as String
// ENVOY_RLS_PORT: port
//
// HTTP_API_HOST: host // just to become HTTP_API_HOST:HTTP_API_PORT as &str
// HTTP_API_PORT: port

use crate::envoy_rls::server::RateLimitHeaders;
use limitador::storage;
use std::fmt;
use tracing::level_filters::LevelFilter;
use url::Url;

pub fn redacted_url(url: String) -> String {
    match Url::parse(url.as_str()) {
        Ok(url_object) => {
            if url_object.password().is_some() {
                let mut owned_url = url_object.clone();
                if owned_url.set_password(Some("****")).is_ok() {
                    String::from(owned_url)
                } else {
                    url.clone()
                }
            } else {
                url.clone()
            }
        }
        Err(_) => url.clone(),
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub limits_file: String,
    pub storage: StorageConfiguration,
    rls_host: String,
    rls_port: u16,
    http_host: String,
    http_port: u16,
    pub limit_name_in_labels: bool,
    pub metric_labels_file: Option<String>,
    pub tracing_endpoint: String,
    pub log_level: Option<LevelFilter>,
    pub rate_limit_headers: RateLimitHeaders,
    pub grpc_reflection_service: bool,
}

pub mod env {
    use lazy_static::lazy_static;
    use std::env;

    lazy_static! {
        pub static ref LIMITS_FILE: Option<&'static str> = value_for("LIMITS_FILE");
        pub static ref ENVOY_RLS_HOST: Option<&'static str> = value_for("ENVOY_RLS_HOST");
        pub static ref ENVOY_RLS_PORT: Option<&'static str> = value_for("ENVOY_RLS_PORT");
        pub static ref HTTP_API_HOST: Option<&'static str> = value_for("HTTP_API_HOST");
        pub static ref HTTP_API_PORT: Option<&'static str> = value_for("HTTP_API_PORT");
        pub static ref TRACING_ENDPOINT: Option<&'static str> = value_for("TRACING_ENDPOINT");
        pub static ref LIMIT_NAME_IN_PROMETHEUS_LABELS: bool =
            env_option_is_enabled("LIMIT_NAME_IN_PROMETHEUS_LABELS");
        pub static ref DISK_PATH: Option<&'static str> = value_for("DISK_PATH");
        pub static ref DISK_OPTIMIZE: Option<&'static str> = value_for("DISK_OPTIMIZE");
        pub static ref REDIS_URL: Option<&'static str> = value_for("REDIS_URL");
        pub static ref REDIS_LOCAL_CACHE_ENABLED: bool =
            env_option_is_enabled("REDIS_LOCAL_CACHE_ENABLED");
        pub static ref REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS: Option<&'static str> =
            value_for("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS");
        pub static ref REDIS_LOCAL_CACHE_BATCH_SIZE: Option<&'static str> =
            value_for("REDIS_LOCAL_CACHE_BATCH_SIZE");
        pub static ref RATE_LIMIT_HEADERS: Option<&'static str> = value_for("RATE_LIMIT_HEADERS");
    }

    fn value_for(env_key: &'static str) -> Option<&'static str> {
        match std::env::var(env_key) {
            Ok(s) => Some(Box::leak(s.into_boxed_str())),
            Err(_) => None,
        }
    }

    fn env_option_is_enabled(env_name: &str) -> bool {
        match env::var(env_name) {
            Ok(value) => value == "1",
            Err(_) => false,
        }
    }
}

impl Configuration {
    pub const DEFAULT_RLS_PORT: &'static str = "8081";
    pub const DEFAULT_HTTP_PORT: &'static str = "8080";
    pub const DEFAULT_IP_BIND: &'static str = "0.0.0.0";

    #[allow(clippy::too_many_arguments)]
    pub fn with(
        storage: StorageConfiguration,
        limits_file: String,
        rls_host: String,
        rls_port: u16,
        http_host: String,
        http_port: u16,
        limit_name_in_labels: bool,
        metric_labels_file: Option<String>,
        tracing_endpoint: String,
        rate_limit_headers: RateLimitHeaders,
        grpc_reflection_service: bool,
    ) -> Self {
        Self {
            limits_file,
            storage,
            rls_host,
            rls_port,
            http_host,
            http_port,
            limit_name_in_labels,
            metric_labels_file,
            tracing_endpoint,
            log_level: None,
            rate_limit_headers,
            grpc_reflection_service,
        }
    }

    pub fn rlp_address(&self) -> String {
        format!("{}:{}", self.rls_host, self.rls_port)
    }

    pub fn http_address(&self) -> String {
        format!("{}:{}", self.http_host, self.http_port)
    }
}

#[cfg(test)]
impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            limits_file: "".to_string(),
            storage: StorageConfiguration::InMemory(InMemoryStorageConfiguration {
                cache_size: Some(10_000),
            }),
            rls_host: "".to_string(),
            rls_port: 0,
            http_host: "".to_string(),
            http_port: 0,
            limit_name_in_labels: false,
            metric_labels_file: None,
            tracing_endpoint: "".to_string(),
            log_level: None,
            rate_limit_headers: RateLimitHeaders::None,
            grpc_reflection_service: false,
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum StorageConfiguration {
    InMemory(InMemoryStorageConfiguration),
    Disk(DiskStorageConfiguration),
    Redis(RedisStorageConfiguration),
    #[cfg(feature = "distributed_storage")]
    Distributed(DistributedStorageConfiguration),
}

#[derive(PartialEq, Eq, Debug)]
pub struct InMemoryStorageConfiguration {
    pub cache_size: Option<u64>,
}

#[derive(PartialEq, Eq, Debug)]
#[cfg(feature = "distributed_storage")]
pub struct DistributedStorageConfiguration {
    pub name: String,
    pub cache_size: Option<u64>,
    pub listen_address: String,
    pub peer_urls: Vec<String>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct DiskStorageConfiguration {
    pub path: String,
    pub optimization: storage::disk::OptimizeFor,
}

#[derive(PartialEq, Eq)]
pub struct RedisStorageConfiguration {
    pub url: String,
    pub cache: Option<RedisStorageCacheConfiguration>,
}

impl fmt::Debug for RedisStorageConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Foo")
            .field("cache", &self.cache)
            .field(
                "url",
                &format_args!("{}", redacted_url(self.url.clone()).as_str()),
            )
            .finish()
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct RedisStorageCacheConfiguration {
    pub batch_size: usize,
    pub flushing_period: i64,
    pub max_counters: usize,
    pub response_timeout: u64,
}
