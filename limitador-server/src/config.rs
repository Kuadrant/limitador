// LIMITS_FILE: Path
//
// LIMIT_NAME_IN_PROMETHEUS_LABELS: bool
//
// REDIS_URL: StorageType { String }
// └ REDIS_LOCAL_CACHE_ENABLED: bool
//   └ REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS: i64 ?!
//   └ REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS: u64 -> Duration
//   └ REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS: u64
//
// INFINISPAN_URL: StorageType { String }
//  └ INFINISPAN_CACHE_NAME: String
//  └ INFINISPAN_COUNTERS_CONSISTENCY: enum Consistency { Weak, Strong }
//
// ENVOY_RLS_HOST: host // just to become ENVOY_RLS_HOST:ENVOY_RLS_PORT as String
// ENVOY_RLS_PORT: port
//
// HTTP_API_HOST: host // just to become HTTP_API_HOST:HTTP_API_PORT as &str
// HTTP_API_PORT: port

use crate::envoy_rls::server::RateLimitHeaders;
use limitador::storage;
use log::LevelFilter;

#[derive(Debug)]
pub struct Configuration {
    pub limits_file: String,
    pub storage: StorageConfiguration,
    rls_host: String,
    rls_port: u16,
    http_host: String,
    http_port: u16,
    pub limit_name_in_labels: bool,
    pub log_level: Option<LevelFilter>,
    pub rate_limit_headers: RateLimitHeaders,
}

pub mod env {
    use lazy_static::lazy_static;

    lazy_static! {
        pub static ref LIMITS_FILE: Option<&'static str> = value_for("LIMITS_FILE");
        pub static ref ENVOY_RLS_HOST: Option<&'static str> = value_for("ENVOY_RLS_HOST");
        pub static ref ENVOY_RLS_PORT: Option<&'static str> = value_for("ENVOY_RLS_PORT");
        pub static ref HTTP_API_HOST: Option<&'static str> = value_for("HTTP_API_HOST");
        pub static ref HTTP_API_PORT: Option<&'static str> = value_for("HTTP_API_PORT");
        pub static ref DISK_PATH: Option<&'static str> = value_for("DISK_PATH");
        pub static ref DISK_OPTIMIZE: Option<&'static str> = value_for("DISK_OPTIMIZE");
        pub static ref REDIS_URL: Option<&'static str> = value_for("REDIS_URL");
        pub static ref REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS: Option<&'static str> =
            value_for("REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS");
        pub static ref REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS: Option<&'static str> =
            value_for("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS");
        pub static ref REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS: Option<&'static str> =
            value_for("REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS");
        pub static ref RATE_LIMIT_HEADERS: Option<&'static str> = value_for("RATE_LIMIT_HEADERS");
        pub static ref INFINISPAN_CACHE_NAME: Option<&'static str> =
            value_for("INFINISPAN_CACHE_NAME");
        pub static ref INFINISPAN_COUNTERS_CONSISTENCY: Option<&'static str> =
            value_for("INFINISPAN_COUNTERS_CONSISTENCY");
    }

    fn value_for(env_key: &'static str) -> Option<&'static str> {
        match std::env::var(env_key) {
            Ok(s) => Some(Box::leak(s.into_boxed_str())),
            Err(_) => None,
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
        rate_limit_headers: RateLimitHeaders,
    ) -> Self {
        Self {
            limits_file,
            storage,
            rls_host,
            rls_port,
            http_host,
            http_port,
            limit_name_in_labels,
            log_level: None,
            rate_limit_headers,
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
            storage: StorageConfiguration::InMemory,
            rls_host: "".to_string(),
            rls_port: 0,
            http_host: "".to_string(),
            http_port: 0,
            limit_name_in_labels: false,
            log_level: None,
            rate_limit_headers: RateLimitHeaders::None,
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum StorageConfiguration {
    InMemory(InMemoryStorageConfiguration),
    Disk(DiskStorageConfiguration),
    Redis(RedisStorageConfiguration),
    #[cfg(feature = "infinispan")]
    Infinispan(InfinispanStorageConfiguration),
}

#[derive(PartialEq, Eq, Debug)]
pub struct InMemoryStorageConfiguration {
    pub cache_size: Option<u64>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct DiskStorageConfiguration {
    pub path: String,
    pub optimization: storage::disk::OptimizeFor,
}

#[derive(PartialEq, Eq, Debug)]
pub struct RedisStorageConfiguration {
    pub url: String,
    pub cache: Option<RedisStorageCacheConfiguration>,
}

#[derive(PartialEq, Eq, Debug)]
pub struct RedisStorageCacheConfiguration {
    pub flushing_period: i64,
    pub max_ttl: u64,
    pub ttl_ratio: u64,
    pub max_counters: usize,
}

#[derive(PartialEq, Eq, Debug)]
#[cfg(feature = "infinispan")]
pub struct InfinispanStorageConfiguration {
    pub url: String,
    pub cache: Option<String>,
    pub consistency: Option<String>,
}
