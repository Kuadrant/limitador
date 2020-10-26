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
            Err(e) => {
                // In this case we could return "Code::Unknown" but that's not
                // very helpful. When envoy receives "Unknown" it simply lets
                // the request pass and this cannot be configured using the
                // "failure_mode_allow" attribute, so it's equivalent to
                // returning "Code::Ok". That's why we return an "unavailable"
                // error here. What envoy does after receiving that kind of
                // error can be configured with "failure_mode_deny".
                // The only errors that can happen here have to do with
                // connecting to the limits storage, which should be temporary.
                error!("Error: {:?}", e);
                return Err(Status::unavailable("Service unavailable"));
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
