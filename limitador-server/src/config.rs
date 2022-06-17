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

use std::env;

pub struct Configuration {
    pub limits_file: Option<String>,
    pub storage: StorageConfiguration,
    rls_host: String,
    rls_port: u16,
    http_host: String,
    http_port: u16,
    pub limit_name_in_labels: bool,
}

impl Configuration {
    pub fn from_env() -> Result<Self, ()> {
        let rls_port = env::var("ENVOY_RLS_PORT").unwrap_or_else(|_| "8081".to_string());
        let http_port = env::var("HTTP_API_PORT").unwrap_or_else(|_| "8080".to_string());
        Ok(Self {
            limits_file: env::var("LIMITS_FILE").ok(),
            storage: storage_config_from_env()?,
            rls_host: env::var("ENVOY_RLS_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            rls_port: rls_port.parse().expect("Expected a port number!"),
            http_host: env::var("HTTP_API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            http_port: http_port.parse().expect("Expected a port number!"),
            limit_name_in_labels: env_option_is_enabled("LIMIT_NAME_IN_PROMETHEUS_LABELS"),
        })
    }

    pub fn rlp_address(&self) -> String {
        format!("{}:{}", self.rls_host, self.rls_port)
    }

    pub fn http_address(&self) -> String {
        format!("{}:{}", self.http_host, self.http_port)
    }
}

fn storage_config_from_env() -> Result<StorageConfiguration, ()> {
    let redis_url = env::var("REDIS_URL");
    let infinispan_url = env::var("INFINISPAN_URL");

    match (redis_url, infinispan_url) {
        (Ok(_), Ok(_)) => Err(()),
        (Ok(url), Err(_)) => Ok(StorageConfiguration::Redis(RedisStorageConfiguration {
            url,
            cache: if env_option_is_enabled("REDIS_LOCAL_CACHE_ENABLED") {
                Some(RedisStorageCacheConfiguration {
                    flushing_period: env::var("REDIS_LOCAL_CACHE_FLUSHING_PERIOD_MS")
                        .unwrap_or_else(|_| "1".to_string())
                        .parse()
                        .expect("Expected an i64"),
                    max_ttl: env::var("REDIS_LOCAL_CACHE_MAX_TTL_CACHED_COUNTERS_MS")
                        .unwrap_or_else(|_| "5000".to_string())
                        .parse()
                        .expect("Expected an u64"),
                    ttl_ratio: env::var("REDIS_LOCAL_CACHE_TTL_RATIO_CACHED_COUNTERS")
                        .unwrap_or_else(|_| "10".to_string())
                        .parse()
                        .expect("Expected an u64"),
                })
            } else {
                None
            },
        })),
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

#[derive(PartialEq, Debug)]
pub enum StorageConfiguration {
    InMemory,
    Redis(RedisStorageConfiguration),
    Infinispan(InfinispanStorageConfiguration),
}

#[derive(PartialEq, Debug)]
pub struct RedisStorageConfiguration {
    pub url: String,
    pub cache: Option<RedisStorageCacheConfiguration>,
}

#[derive(PartialEq, Debug)]
pub struct RedisStorageCacheConfiguration {
    pub flushing_period: i64,
    pub max_ttl: u64,
    pub ttl_ratio: u64,
}

#[derive(PartialEq, Debug)]
pub struct InfinispanStorageConfiguration {
    pub url: String,
    pub cache: Option<String>,
    pub consistency: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::config::{Configuration, StorageConfiguration};
    use serial_test::serial;
    use std::env;

    #[test]
    #[serial]
    fn test_config_defaults() {
        env::remove_var("REDIS_URL");
        env::remove_var("INFINISPAN_URL");

        println!("{:?}", env::var("REDIS_URL"));
        println!("{:?}", env::var("INFINISPAN_URL"));

        let config = Configuration::from_env().unwrap();
        assert_eq!(config.limits_file, None);
        assert_eq!(config.storage, StorageConfiguration::InMemory);
        assert_eq!(config.http_address(), "0.0.0.0:8080".to_string());
        assert_eq!(config.rlp_address(), "0.0.0.0:8081".to_string());
        assert_eq!(config.limit_name_in_labels, false);
    }

    #[test]
    #[serial]
    fn test_config_redis_defaults() {
        env::remove_var("REDIS_URL");
        env::remove_var("INFINISPAN_URL");

        println!("{:?}", env::var("REDIS_URL"));
        println!("{:?}", env::var("INFINISPAN_URL"));

        let url = "127.0.1.1:7654";
        env::set_var("REDIS_URL", url);
        let config = Configuration::from_env().unwrap();
        assert_eq!(config.limits_file, None);
        if let StorageConfiguration::Redis(ref redis_config) = config.storage {
            assert_eq!(redis_config.url, url);
            assert_eq!(redis_config.cache, None);
        } else {
            panic!("Should be a Redis config!");
        }
        assert_eq!(config.http_address(), "0.0.0.0:8080".to_string());
        assert_eq!(config.rlp_address(), "0.0.0.0:8081".to_string());
        assert_eq!(config.limit_name_in_labels, false);
    }

    #[test]
    #[serial]
    fn test_config_infinispan_defaults() {
        env::remove_var("REDIS_URL");
        env::remove_var("INFINISPAN_URL");

        println!("{:?}", env::var("REDIS_URL"));
        println!("{:?}", env::var("INFINISPAN_URL"));

        let url = "127.0.2.2:9876";
        env::set_var("INFINISPAN_URL", url);
        let config = Configuration::from_env().unwrap();
        assert_eq!(config.limits_file, None);
        if let StorageConfiguration::Infinispan(ref infinispan_config) = config.storage {
            assert_eq!(infinispan_config.url, url);
            assert_eq!(infinispan_config.cache, None);
            assert_eq!(infinispan_config.consistency, None);
        } else {
            panic!("Should be an Infinispan config!");
        }
        assert_eq!(config.http_address(), "0.0.0.0:8080".to_string());
        assert_eq!(config.rlp_address(), "0.0.0.0:8081".to_string());
        assert_eq!(config.limit_name_in_labels, false);
    }
}
