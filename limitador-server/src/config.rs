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
}

impl Configuration {
    pub const DEFAULT_RLS_PORT: &'static str = "8081";
    pub const DEFAULT_HTTP_PORT: &'static str = "8080";
    pub const DEFAULT_IP_BIND: &'static str = "0.0.0.0";

    pub fn with(
        storage: StorageConfiguration,
        limits_file: String,
        rls_host: String,
        rls_port: u16,
        http_host: String,
        http_port: u16,
        limit_name_in_labels: bool,
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
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub enum StorageConfiguration {
    InMemory,
    Redis(RedisStorageConfiguration),
    Infinispan(InfinispanStorageConfiguration),
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
pub struct InfinispanStorageConfiguration {
    pub url: String,
    pub cache: Option<String>,
    pub consistency: Option<String>,
}
