use crate::envoy_rls::server::envoy::service::ratelimit::v2::rate_limit_response::Code;
use crate::envoy_rls::server::envoy::service::ratelimit::v2::rate_limit_service_server::{
    RateLimitService, RateLimitServiceServer,
};
use crate::envoy_rls::server::envoy::service::ratelimit::v2::{
    RateLimitRequest, RateLimitResponse,
};
use crate::Limiter;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{transport, transport::Server, Request, Response, Status};

include!("envoy_types.rs");

pub struct MyRateLimiter {
    limiter: Arc<Limiter>,
}

impl MyRateLimiter {
    pub fn new(limiter: Arc<Limiter>) -> MyRateLimiter {
        MyRateLimiter { limiter }
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

pub async fn run_envoy_rls_server(
    address: String,
    limiter: Arc<Limiter>,
) -> Result<(), transport::Error> {
    let rate_limiter = MyRateLimiter::new(limiter);
    let svc = RateLimitServiceServer::new(rate_limiter);

    Server::builder()
        .add_service(svc)
        .serve(address.parse().unwrap())
        .await
}
