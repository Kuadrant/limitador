use opentelemetry::global;
use opentelemetry::propagation::Extractor;
use std::collections::HashMap;
use std::sync::Arc;

use tonic::codegen::http::HeaderMap;
use tonic::{transport, transport::Server, Request, Response, Status};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use limitador::counter::Counter;

use crate::envoy_rls::server::envoy::config::core::v3::HeaderValue;
use crate::envoy_rls::server::envoy::service::ratelimit::v3::rate_limit_response::Code;
use crate::envoy_rls::server::envoy::service::ratelimit::v3::rate_limit_service_server::{
    RateLimitService, RateLimitServiceServer,
};
use crate::envoy_rls::server::envoy::service::ratelimit::v3::{
    RateLimitRequest, RateLimitResponse,
};
use crate::prometheus_metrics::PrometheusMetrics;
use crate::Limiter;

include!("envoy_types.rs");

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum RateLimitHeaders {
    None,
    DraftVersion03,
}

pub struct MyRateLimiter {
    limiter: Arc<Limiter>,
    rate_limit_headers: RateLimitHeaders,
    metrics: Arc<PrometheusMetrics>,
}

impl MyRateLimiter {
    pub fn new(
        limiter: Arc<Limiter>,
        rate_limit_headers: RateLimitHeaders,
        metrics: Arc<PrometheusMetrics>,
    ) -> Self {
        Self {
            limiter,
            rate_limit_headers,
            metrics,
        }
    }
}

#[tonic::async_trait]
impl RateLimitService for MyRateLimiter {
    #[tracing::instrument(skip_all)]
    async fn should_rate_limit(
        &self,
        request: Request<RateLimitRequest>,
    ) -> Result<Response<RateLimitResponse>, Status> {
        debug!("Request received: {:?}", request);

        let mut values: HashMap<String, String> = HashMap::new();
        let (metadata, _ext, req) = request.into_parts();
        let namespace = req.domain;
        let rl_headers = RateLimitRequestHeaders::new(metadata.into_headers());
        let parent_context =
            global::get_text_map_propagator(|propagator| propagator.extract(&rl_headers));
        let span = Span::current();
        span.set_parent(parent_context);

        if namespace.is_empty() {
            return Ok(Response::new(RateLimitResponse {
                overall_code: Code::Unknown.into(),
                statuses: vec![],
                request_headers_to_add: vec![],
                response_headers_to_add: vec![],
                raw_body: vec![],
                dynamic_metadata: None,
                quota: None,
            }));
        }

        let namespace = namespace.into();

        for descriptor in &req.descriptors {
            for entry in &descriptor.entries {
                values.insert(entry.key.clone(), entry.value.clone());
            }
        }

        // "hits_addend" is optional according to the spec, and should default
        // to 1, However, with the autogenerated structs it defaults to 0.
        let hits_addend = if req.hits_addend == 0 {
            1
        } else {
            req.hits_addend
        };

        let rate_limited_resp = match &*self.limiter {
            Limiter::Blocking(limiter) => limiter.check_rate_limited_and_update(
                &namespace,
                &values,
                u64::from(hits_addend),
                self.rate_limit_headers != RateLimitHeaders::None,
            ),
            Limiter::Async(limiter) => {
                limiter
                    .check_rate_limited_and_update(
                        &namespace,
                        &values,
                        u64::from(hits_addend),
                        self.rate_limit_headers != RateLimitHeaders::None,
                    )
                    .await
            }
        };

        if let Err(e) = rate_limited_resp {
            // In this case we could return "Code::Unknown" but that's not
            // very helpful. When envoy receives "Unknown" it simply lets
            // the request pass and this cannot be configured using the
            // "failure_mode_deny" attribute, so it's equivalent to
            // returning "Code::Ok". That's why we return an "unavailable"
            // error here. What envoy does after receiving that kind of
            // error can be configured with "failure_mode_deny". The only
            // errors that can happen here have to do with connecting to the
            // limits storage, which should be temporary.
            error!("Error: {:?}", e);
            return Err(Status::unavailable("Service unavailable"));
        }

        let mut rate_limited_resp = rate_limited_resp.unwrap();
        let resp_code = if rate_limited_resp.limited {
            self.metrics
                .incr_limited_calls(&namespace, rate_limited_resp.limit_name.as_deref());
            Code::OverLimit
        } else {
            self.metrics.incr_authorized_calls(&namespace);
            Code::Ok
        };

        let reply = RateLimitResponse {
            overall_code: resp_code.into(),
            statuses: vec![],
            request_headers_to_add: vec![],
            response_headers_to_add: to_response_header(
                &self.rate_limit_headers,
                &mut rate_limited_resp.counters,
            ),
            raw_body: vec![],
            dynamic_metadata: None,
            quota: None,
        };

        Ok(Response::new(reply))
    }
}

pub fn to_response_header(
    rate_limit_headers: &RateLimitHeaders,
    counters: &mut [Counter],
) -> Vec<HeaderValue> {
    let mut headers = Vec::new();
    match rate_limit_headers {
        RateLimitHeaders::None => {}

        // creates response headers per https://datatracker.ietf.org/doc/id/draft-polli-ratelimit-headers-03.html
        RateLimitHeaders::DraftVersion03 => {
            // sort by the limit remaining..
            counters.sort_by(|a, b| {
                let a_remaining = a.remaining().unwrap_or(a.max_value());
                let b_remaining = b.remaining().unwrap_or(b.max_value());
                a_remaining.cmp(&b_remaining)
            });

            let mut all_limits_text = String::with_capacity(20 * counters.len());
            counters.iter_mut().for_each(|counter| {
                all_limits_text.push_str(
                    format!(", {};w={}", counter.max_value(), counter.seconds()).as_str(),
                );
                if let Some(name) = counter.limit().name() {
                    all_limits_text
                        .push_str(format!(";name=\"{}\"", name.replace('"', "'")).as_str());
                }
            });

            if let Some(counter) = counters.first() {
                headers.push(HeaderValue {
                    key: "X-RateLimit-Limit".to_string(),
                    value: format!("{}{}", counter.max_value(), all_limits_text),
                });

                let remaining = counter.remaining().unwrap_or(counter.max_value());
                headers.push(HeaderValue {
                    key: "X-RateLimit-Remaining".to_string(),
                    value: format!("{}", remaining),
                });

                if let Some(duration) = counter.expires_in() {
                    headers.push(HeaderValue {
                        key: "X-RateLimit-Reset".to_string(),
                        value: format!("{}", duration.as_secs()),
                    });
                }
            }
        }
    };
    headers
}

struct RateLimitRequestHeaders {
    inner: HeaderMap,
}
impl RateLimitRequestHeaders {
    pub fn new(inner: HeaderMap) -> Self {
        Self { inner }
    }
}
impl Extractor for RateLimitRequestHeaders {
    fn get(&self, key: &str) -> Option<&str> {
        match self.inner.get(key) {
            Some(v) => v.to_str().ok(),
            None => None,
        }
    }

    fn keys(&self) -> Vec<&str> {
        self.inner.keys().map(|k| k.as_str()).collect()
    }
}

mod rls_proto {
    pub(crate) const RLS_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("rls");
}

pub async fn run_envoy_rls_server(
    address: String,
    limiter: Arc<Limiter>,
    rate_limit_headers: RateLimitHeaders,
    metrics: Arc<PrometheusMetrics>,
    grpc_reflection_service: bool,
) -> Result<(), transport::Error> {
    let rate_limiter = MyRateLimiter::new(limiter, rate_limit_headers, metrics);
    let svc = RateLimitServiceServer::new(rate_limiter);

    let reflection_service = match grpc_reflection_service {
        false => None,
        true => Some(
            tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(rls_proto::RLS_DESCRIPTOR_SET)
                .build()
                .unwrap(),
        ),
    };

    Server::builder()
        .add_service(svc)
        .add_optional_service(reflection_service)
        .serve(address.parse().unwrap())
        .await
}

#[cfg(test)]
mod tests {
    use tonic::IntoRequest;

    use limitador::limit::Limit;
    use limitador::RateLimiter;

    use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::rate_limit_descriptor::Entry;
    use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::RateLimitDescriptor;
    use crate::prometheus_metrics::tests::TEST_PROMETHEUS_HANDLE;
    use crate::Configuration;

    use super::*;

    fn header_value(key: &str, value: &str) -> HeaderValue {
        HeaderValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    // All these tests use the in-memory storage implementation to simplify. We
    // know that some storage implementations like the Redis one trade
    // rate-limiting accuracy for performance. That would be a bit more
    // complicated to test.
    // Also, the logic behind these endpoints is well tested in the library,
    // that's why running some simple tests here should be enough.

    #[tokio::test]
    async fn test_returns_ok_and_overlimit_correctly() {
        let namespace = "test_namespace";
        let limit = Limit::new(
            namespace,
            1,
            60,
            vec!["req.method == 'GET'"],
            vec!["app_id"],
        );

        let limiter = RateLimiter::new(10_000);
        limiter.add_limit(limit);

        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::Blocking(limiter)),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: namespace.to_string(),
            descriptors: vec![RateLimitDescriptor {
                entries: vec![
                    Entry {
                        key: "req.method".to_string(),
                        value: "GET".to_string(),
                    },
                    Entry {
                        key: "app_id".to_string(),
                        value: "1".to_string(),
                    },
                ],
                limit: None,
            }],
            hits_addend: 1,
        };

        // There's a limit of 1, so the first request should return "OK" and the
        // second "OverLimit".

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::Ok));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "1, 1;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "1, 1;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );
    }

    #[tokio::test]
    async fn test_returns_ok_when_no_limits_apply() {
        // No limits saved
        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::new(Configuration::default()).await.unwrap()),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: "test_namespace".to_string(),
            descriptors: vec![RateLimitDescriptor {
                entries: vec![Entry {
                    key: "req.method".to_string(),
                    value: "GET".to_string(),
                }],
                limit: None,
            }],
            hits_addend: 1,
        }
        .into_request();

        let response = rate_limiter
            .should_rate_limit(req)
            .await
            .unwrap()
            .into_inner();

        assert_eq!(response.overall_code, i32::from(Code::Ok));
        assert_eq!(response.response_headers_to_add, vec![],);
    }

    #[tokio::test]
    async fn test_returns_unknown_when_domain_is_empty() {
        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::new(Configuration::default()).await.unwrap()),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: "".to_string(),
            descriptors: vec![RateLimitDescriptor {
                entries: vec![Entry {
                    key: "req.method".to_string(),
                    value: "GET".to_string(),
                }],
                limit: None,
            }],
            hits_addend: 1,
        }
        .into_request();

        let response = rate_limiter
            .should_rate_limit(req)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::Unknown));
        assert_eq!(response.response_headers_to_add, vec![],);
    }

    #[tokio::test]
    async fn test_takes_into_account_all_the_descriptors() {
        let limiter = RateLimiter::new(10_000);

        let namespace = "test_namespace";

        vec![
            Limit::new(namespace, 10, 60, vec!["x == '1'"], vec!["z"]),
            Limit::new(namespace, 0, 60, vec!["x == '1'", "y == '2'"], vec!["z"]),
        ]
        .into_iter()
        .for_each(|limit| {
            limiter.add_limit(limit);
        });

        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::Blocking(limiter)),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: namespace.to_string(),
            descriptors: vec![
                RateLimitDescriptor {
                    entries: vec![
                        Entry {
                            key: "x".to_string(),
                            value: "1".to_string(),
                        },
                        Entry {
                            key: "z".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                },
                // If this is taken into account, the result will be "overlimit"
                // because of the second limit that has a max of 0.
                RateLimitDescriptor {
                    entries: vec![Entry {
                        key: "y".to_string(),
                        value: "2".to_string(),
                    }],
                    limit: None,
                },
            ],
            hits_addend: 1,
        };

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();

        assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "0, 0;w=60, 10;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );
    }

    #[tokio::test]
    async fn test_takes_into_account_the_hits_addend_param() {
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 10, 60, vec!["x == '1'"], vec!["y"]);

        let limiter = RateLimiter::new(10_000);
        limiter.add_limit(limit);

        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::Blocking(limiter)),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: namespace.to_string(),
            descriptors: vec![RateLimitDescriptor {
                entries: vec![
                    Entry {
                        key: "x".to_string(),
                        value: "1".to_string(),
                    },
                    Entry {
                        key: "y".to_string(),
                        value: "1".to_string(),
                    },
                ],
                limit: None,
            }],
            hits_addend: 6,
        };

        // There's a limit of 10, "hits_addend" is 6, so the first request
        // should return "Ok" and the second "OverLimit".

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::Ok));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "10, 10;w=60"),
                header_value("X-RateLimit-Remaining", "4"),
            ],
        );

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "10, 10;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );
    }

    #[tokio::test]
    async fn test_0_hits_addend_is_converted_to_1() {
        // "hits_addend" is optional according to the spec, and should default
        // to 1, However, with the autogenerated structs it defaults to 0.
        let namespace = "test_namespace";
        let limit = Limit::new(namespace, 1, 60, vec!["x == '1'"], vec!["y"]);

        let limiter = RateLimiter::new(10_000);
        limiter.add_limit(limit);

        let rate_limiter = MyRateLimiter::new(
            Arc::new(Limiter::Blocking(limiter)),
            RateLimitHeaders::DraftVersion03,
            Arc::new(PrometheusMetrics::new_with_handle(
                false,
                TEST_PROMETHEUS_HANDLE.clone(),
            )),
        );

        let req = RateLimitRequest {
            domain: namespace.to_string(),
            descriptors: vec![RateLimitDescriptor {
                entries: vec![
                    Entry {
                        key: "x".to_string(),
                        value: "1".to_string(),
                    },
                    Entry {
                        key: "y".to_string(),
                        value: "2".to_string(),
                    },
                ],
                limit: None,
            }],
            hits_addend: 0,
        };

        // There's a limit of 1, and hits_addend is converted to 1, so the first
        // request should return "OK" and the second "OverLimit".

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::Ok));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "1, 1;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );

        let response = rate_limiter
            .should_rate_limit(req.clone().into_request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        assert_eq!(
            response.response_headers_to_add,
            vec![
                header_value("X-RateLimit-Limit", "1, 1;w=60"),
                header_value("X-RateLimit-Remaining", "0"),
            ],
        );
    }
}
