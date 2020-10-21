#[macro_use]
extern crate log;

use crate::envoy::service::ratelimit::v2::rate_limit_response::Code;
use crate::envoy::service::ratelimit::v2::rate_limit_service_server::{
    RateLimitService, RateLimitServiceServer,
};
use crate::envoy::service::ratelimit::v2::{RateLimitRequest, RateLimitResponse};
use limitador::limit::Limit;
use limitador::storage::redis::AsyncRedisStorage;
use limitador::{AsyncRateLimiter, RateLimiter};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::signal;
use tonic::{transport::Server, Request, Response, Status};

const LIMITS_FILE_ENV: &str = "LIMITS_FILE";

include!("envoy.rs");

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

pub struct MyRateLimiter {
    limiter: Arc<Limiter>,
}

async fn new_limiter() -> Limiter {
    match env::var("REDIS_URL") {
        // Let's use the async impl. This could be configurable if needed.
        Ok(redis_url) => {
            let async_limiter = AsyncRateLimiter::new_with_storage(Box::new(
                AsyncRedisStorage::new(&redis_url).await,
            ));
            Limiter::Async(async_limiter)
        }
        Err(_) => Limiter::Blocking(RateLimiter::default()),
    }
}

impl MyRateLimiter {
    pub async fn new() -> MyRateLimiter {
        match env::var(LIMITS_FILE_ENV) {
            Ok(val) => {
                let f = std::fs::File::open(val).unwrap();
                let limits: Vec<Limit> = serde_yaml::from_reader(f).unwrap();

                let rate_limiter = new_limiter().await;

                match &rate_limiter {
                    Limiter::Blocking(limiter) => limiter.configure_with(limits).unwrap(),
                    Limiter::Async(limiter) => limiter.configure_with(limits).await.unwrap(),
                }

                MyRateLimiter {
                    limiter: Arc::new(rate_limiter),
                }
            }
            _ => panic!("LIMITS_FILE env not set"),
        }
    }
}

#[tonic::async_trait]
impl RateLimitService for MyRateLimiter {
    async fn should_rate_limit(
        &self,
        request: Request<RateLimitRequest>,
    ) -> Result<Response<RateLimitResponse>, Status> {
        debug!("Request received: {:?}", request);

        let mut values: HashMap<String, String> = HashMap::new();
        let req = request.into_inner();
        let namespace = req.domain;

        if namespace.is_empty() {
            return Ok(Response::new(RateLimitResponse {
                overall_code: Code::Unknown.into(),
                statuses: vec![],
                headers: vec![],
                request_headers_to_add: vec![],
            }));
        }

        for descriptor in &req.descriptors {
            for entry in &descriptor.entries {
                values.insert(entry.key.clone(), entry.value.clone());
            }
        }

        let is_rate_limited_res = match &*self.limiter {
            Limiter::Blocking(limiter) => {
                limiter.check_rate_limited_and_update(namespace, &values, 1)
            }
            Limiter::Async(limiter) => {
                limiter
                    .check_rate_limited_and_update(namespace, &values, 1)
                    .await
            }
        };

        let resp_code = match is_rate_limited_res {
            Ok(rate_limited) => {
                if rate_limited {
                    Code::OverLimit
                } else {
                    Code::Ok
                }
            }
            // TODO: For now, deny the request if there's an error
            Err(e) => {
                error!("Error: {:?}", e);
                Code::OverLimit
            }
        };

        let reply = RateLimitResponse {
            overall_code: resp_code.into(),
            statuses: vec![],
            headers: vec![],
            request_headers_to_add: vec![],
        };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let host = env::var("HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let port = env::var("PORT").unwrap_or_else(|_| String::from("50052"));

    let addr = format!("{host}:{port}", host = host, port = port).parse()?;

    info!("Listening on {}", addr);

    let rate_limiter = MyRateLimiter::new().await;
    let svc = RateLimitServiceServer::new(rate_limiter);

    Server::builder().add_service(svc).serve(addr).await?;

    signal::ctrl_c().await?;

    Ok(())
}
