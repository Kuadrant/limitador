#[macro_use]
extern crate log;

use crate::envoy_rls::server::run_envoy_rls_server;
use crate::http_api::server::run_http_server;
use limitador::limit::Limit;
use limitador::storage::redis::AsyncRedisStorage;
use limitador::{AsyncRateLimiter, RateLimiter};
use std::env;
use std::sync::Arc;

mod envoy_rls;
mod http_api;

const LIMITS_FILE_ENV: &str = "LIMITS_FILE";

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

impl Limiter {
    pub async fn new() -> Limiter {
        let rate_limiter = match env::var("REDIS_URL") {
            // Let's use the async impl. This could be configurable if needed.
            Ok(redis_url) => {
                let async_limiter = AsyncRateLimiter::new_with_storage(Box::new(
                    AsyncRedisStorage::new(&redis_url).await,
                ));
                Limiter::Async(async_limiter)
            }
            Err(_) => Limiter::Blocking(RateLimiter::default()),
        };

        rate_limiter.load_limits_from_file().await;

        rate_limiter
    }

    async fn load_limits_from_file(&self) {
        if let Ok(limits_file_path) = env::var(LIMITS_FILE_ENV) {
            let f = std::fs::File::open(limits_file_path).unwrap();
            let limits: Vec<Limit> = serde_yaml::from_reader(f).unwrap();

            match &self {
                Limiter::Blocking(limiter) => limiter.configure_with(limits).unwrap(),
                Limiter::Async(limiter) => limiter.configure_with(limits).await.unwrap(),
            }
        }
    }
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await);

    let envoy_rls_host = env::var("ENVOY_RLS_HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let envoy_rls_port = env::var("ENVOY_RLS_PORT").unwrap_or_else(|_| String::from("50052"));
    let envoy_rls_address = format!("{}:{}", envoy_rls_host, envoy_rls_port);
    info!("Envoy RLS server starting on {}", envoy_rls_address);
    tokio::spawn(run_envoy_rls_server(
        envoy_rls_address.to_string(),
        rate_limiter.clone(),
    ));

    let http_api_host = env::var("HTTP_API_HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let http_api_port = env::var("HTTP_API_PORT").unwrap_or_else(|_| String::from("8081"));
    let http_api_address = format!("{}:{}", http_api_host, http_api_port);
    run_http_server(&http_api_address, rate_limiter.clone()).await?;

    Ok(())
}
