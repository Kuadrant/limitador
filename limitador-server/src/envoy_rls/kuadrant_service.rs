use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tonic::{Request, Response, Status};

use super::server::custom::service::ratelimit::v1::rate_limit_service_server::RateLimitService;
use super::server::custom::service::ratelimit::v1::{
    CommitRequest, CommitResponse, ReserveRequest, ReserveResponse,
};
use super::server::envoy::service::ratelimit::v3::rate_limit_response::Code;
use super::server::envoy::service::ratelimit::v3::{RateLimitRequest, RateLimitResponse};
use crate::prometheus_metrics::PrometheusMetrics;
use crate::Limiter;
use limitador::limit::Context;
use limitador::reservation::ReservationId;

// RFC 0021 defaults. Not yet wired to the `--max-reservation-ttl`/`--max-reservation-fraction`
// server flags (separate, not-yet-implemented follow-up) - a caller-requested ttl is clamped
// to this ceiling, and it's also used when the caller doesn't set `ttl` at all. There is no
// equivalent clamp yet for `amount` (the fraction-of-limit clamp needs each matching counter's
// own max_value, which isn't known until inside `RateLimiter::reserve`).
const MAX_RESERVATION_TTL: Duration = Duration::from_secs(60);

pub struct KuadrantService {
    limiter: Arc<Limiter>,
    metrics: Arc<PrometheusMetrics>,
}

impl KuadrantService {
    pub fn new(limiter: Arc<Limiter>, metrics: Arc<PrometheusMetrics>) -> Self {
        Self { limiter, metrics }
    }
}

#[tonic::async_trait]
impl RateLimitService for KuadrantService {
    #[tracing::instrument(skip_all)]
    async fn check_rate_limit(
        &self,
        request: Request<RateLimitRequest>,
    ) -> Result<Response<RateLimitResponse>, Status> {
        debug!("CheckRateLimit request received: {:?}", request);

        let mut values: Vec<HashMap<String, String>> = Vec::default();
        let (_metadata, _ext, req) = request.into_parts();
        let namespace = req.domain;

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
            let mut map = HashMap::default();
            for entry in &descriptor.entries {
                map.insert(entry.key.clone(), entry.value.clone());
            }
            values.push(map);
        }

        let mut ctx = Context::default();
        ctx.list_binding("descriptors".to_string(), values);

        let rate_limited_resp = match &*self.limiter {
            Limiter::Blocking(limiter) => limiter.is_rate_limited(&namespace, &ctx, 1),
            Limiter::Async(limiter) => limiter.is_rate_limited(&namespace, &ctx, 1).await,
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

        let rate_limited_resp = rate_limited_resp.unwrap();
        let resp_code = if rate_limited_resp.limited {
            self.metrics.incr_limited_calls(
                &namespace,
                rate_limited_resp.limit_name.as_deref(),
                &ctx,
            );
            Code::OverLimit
        } else {
            self.metrics.incr_authorized_calls(&namespace, &ctx);
            Code::Ok
        };

        let reply = RateLimitResponse {
            overall_code: resp_code.into(),
            statuses: vec![],
            request_headers_to_add: vec![],
            response_headers_to_add: vec![],
            raw_body: vec![],
            dynamic_metadata: None,
            quota: None,
        };

        Ok(Response::new(reply))
    }

    #[tracing::instrument(skip_all)]
    async fn report(
        &self,
        request: Request<RateLimitRequest>,
    ) -> Result<Response<RateLimitResponse>, Status> {
        debug!("Report request received: {:?}", request);

        let mut values: Vec<HashMap<String, String>> = Vec::default();
        let (_metadata, _ext, req) = request.into_parts();
        let namespace = req.domain;

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
            let mut map = HashMap::default();
            for entry in &descriptor.entries {
                map.insert(entry.key.clone(), entry.value.clone());
            }
            values.push(map);
        }

        // "hits_addend" is optional according to the spec, and should default
        // to 1, However, with the autogenerated structs it defaults to 0.
        let hits_addend = if req.hits_addend == 0 {
            1
        } else {
            req.hits_addend
        } as u64;

        let mut ctx = Context::default();
        ctx.list_binding("descriptors".to_string(), values);

        let rate_limited_resp = match &*self.limiter {
            Limiter::Blocking(limiter) => limiter.update_counters(&namespace, &ctx, hits_addend),
            Limiter::Async(limiter) => limiter.update_counters(&namespace, &ctx, hits_addend).await,
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

        self.metrics.incr_report_calls(&namespace, &ctx);
        self.metrics
            .incr_authorized_hits(&namespace, &ctx, hits_addend);

        let reply = RateLimitResponse {
            overall_code: Code::Ok as i32,
            statuses: vec![],
            request_headers_to_add: vec![],
            response_headers_to_add: vec![],
            raw_body: vec![],
            dynamic_metadata: None,
            quota: None,
        };

        Ok(Response::new(reply))
    }

    #[tracing::instrument(skip_all)]
    async fn reserve(
        &self,
        request: Request<ReserveRequest>,
    ) -> Result<Response<ReserveResponse>, Status> {
        debug!("Reserve request received: {:?}", request);

        let mut values: Vec<HashMap<String, String>> = Vec::default();
        let (_metadata, _ext, req) = request.into_parts();
        let namespace = req.domain;

        if namespace.is_empty() {
            return Ok(Response::new(ReserveResponse {
                code: Code::Unknown.into(),
                reservation_id: String::new(),
            }));
        }

        let namespace = namespace.into();

        for descriptor in &req.descriptors {
            let mut map = HashMap::default();
            for entry in &descriptor.entries {
                map.insert(entry.key.clone(), entry.value.clone());
            }
            values.push(map);
        }

        let mut ctx = Context::default();
        ctx.list_binding("descriptors".to_string(), values);

        let ttl = req
            .ttl
            .and_then(|d| Duration::try_from(d).ok())
            .unwrap_or(MAX_RESERVATION_TTL)
            .min(MAX_RESERVATION_TTL);

        let reserve_resp = match &*self.limiter {
            Limiter::Blocking(limiter) => limiter.reserve(&namespace, &ctx, req.amount, ttl, false),
            Limiter::Async(limiter) => {
                limiter
                    .reserve(&namespace, &ctx, req.amount, ttl, false)
                    .await
            }
        };

        if let Err(e) = reserve_resp {
            // See the comment on the same pattern in `check_rate_limit` above: an
            // "unavailable" error lets `failure_mode_deny` decide the outcome, rather than
            // silently letting the request through.
            error!("Error: {:?}", e);
            return Err(Status::unavailable("Service unavailable"));
        }

        let reserve_resp = reserve_resp.unwrap();
        let (code, reservation_id) = if reserve_resp.limited {
            self.metrics
                .incr_limited_calls(&namespace, reserve_resp.limit_name.as_deref(), &ctx);
            (Code::OverLimit, String::new())
        } else {
            self.metrics.incr_authorized_calls(&namespace, &ctx);
            let reservation_id = reserve_resp
                .reservation_id
                .map(|id| id.to_string())
                .unwrap_or_default();
            (Code::Ok, reservation_id)
        };

        Ok(Response::new(ReserveResponse {
            code: code.into(),
            reservation_id,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        debug!("Commit request received: {:?}", request);

        let mut values: Vec<HashMap<String, String>> = Vec::default();
        let (_metadata, _ext, req) = request.into_parts();
        let namespace = req.domain;

        if namespace.is_empty() {
            return Ok(Response::new(CommitResponse {
                reservation_released: false,
            }));
        }

        let namespace = namespace.into();

        for descriptor in &req.descriptors {
            let mut map = HashMap::default();
            for entry in &descriptor.entries {
                map.insert(entry.key.clone(), entry.value.clone());
            }
            values.push(map);
        }

        let mut ctx = Context::default();
        ctx.list_binding("descriptors".to_string(), values);

        let reservation_id = ReservationId::from(req.reservation_id);

        let commit_resp = match &*self.limiter {
            Limiter::Blocking(limiter) => {
                limiter.commit_reservation(&namespace, &ctx, &reservation_id, req.actual_amount)
            }
            Limiter::Async(limiter) => {
                limiter
                    .commit_reservation(&namespace, &ctx, &reservation_id, req.actual_amount)
                    .await
            }
        };

        if let Err(e) = commit_resp {
            error!("Error: {:?}", e);
            return Err(Status::unavailable("Service unavailable"));
        }

        let commit_resp = commit_resp.unwrap();
        self.metrics.incr_report_calls(&namespace, &ctx);
        self.metrics
            .incr_authorized_hits(&namespace, &ctx, req.actual_amount);

        Ok(Response::new(CommitResponse {
            reservation_released: commit_resp.reservation_released,
        }))
    }
}

#[cfg(test)]
mod tests {
    mod check_rate_limit {
        use tonic::IntoRequest;

        use limitador::limit::Limit;
        use limitador::RateLimiter;

        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::rate_limit_descriptor::Entry;
        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::RateLimitDescriptor;
        use crate::envoy_rls::server::envoy::service::ratelimit::v3::RateLimitRequest;
        use crate::envoy_rls::server::tests::TEST_PROMETHEUS_HANDLE;

        use super::super::*;

        // All these tests use the in-memory storage implementation to simplify. We
        // know that some storage implementations like the Redis one trade
        // rate-limiting accuracy for performance. That would be a bit more
        // complicated to test.
        // Also, the logic behind these endpoints is well tested in the library,
        // that's why running some simple tests here should be enough.

        #[tokio::test]
        async fn test_returns_ok_correctly() {
            let namespace = "test_namespace";
            let limit = Limit::new(
                namespace,
                1,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );

            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);

            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                            key: "app.id".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                }],
                hits_addend: 1, // irrelevant for this test
            };

            // There's a limit of 1, so the first request should return "OK"

            let response = rate_limiter
                .check_rate_limit(req.clone().into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Ok));

            let response = rate_limiter
                .check_rate_limit(req.clone().into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Ok));
        }

        #[tokio::test]
        async fn test_returns_overlimit_correctly() {
            let namespace = "test_namespace";
            let limit = Limit::new(
                namespace,
                0,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );

            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);

            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                            key: "app.id".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                }],
                hits_addend: 1, // irrelevant for this test
            };

            // There's a limit of 1, so the first request should return "OK" and the
            // second "OverLimit".

            let response = rate_limiter
                .check_rate_limit(req.into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        }

        #[tokio::test]
        async fn test_returns_ok_when_no_limits_apply() {
            // No limits saved
            let limiter = RateLimiter::new(10_000);
            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                .check_rate_limit(req)
                .await
                .unwrap()
                .into_inner();

            assert_eq!(response.overall_code, i32::from(Code::Ok));
        }

        #[tokio::test]
        async fn test_returns_unknown_when_domain_is_empty() {
            let limiter = RateLimiter::new(10_000);
            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                .check_rate_limit(req)
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Unknown));
        }

        #[tokio::test]
        async fn test_takes_into_account_all_the_descriptors() {
            let limiter = RateLimiter::new(10_000);

            let namespace = "test_namespace";

            vec![
                Limit::new(
                    namespace,
                    10,
                    60,
                    vec!["descriptors[0].x == '1'"
                        .try_into()
                        .expect("failed parsing!")],
                    vec!["descriptors[0].z".try_into().expect("failed parsing!")],
                ),
                Limit::new(
                    namespace,
                    0,
                    60,
                    vec![
                        "descriptors[0].x == '1'"
                            .try_into()
                            .expect("failed parsing!"),
                        "descriptors[1].y == '2'"
                            .try_into()
                            .expect("failed parsing!"),
                    ],
                    vec!["descriptors[0].z".try_into().expect("failed parsing!")],
                ),
            ]
            .into_iter()
            .for_each(|limit| {
                limiter.add_limit(limit);
            });

            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                .check_rate_limit(req.into_request())
                .await
                .unwrap()
                .into_inner();

            assert_eq!(response.overall_code, i32::from(Code::OverLimit));
        }
    }

    mod report {
        use tonic::IntoRequest;

        use limitador::limit::Limit;
        use limitador::RateLimiter;

        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::rate_limit_descriptor::Entry;
        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::RateLimitDescriptor;
        use crate::envoy_rls::server::envoy::service::ratelimit::v3::RateLimitRequest;
        use crate::envoy_rls::server::tests::TEST_PROMETHEUS_HANDLE;

        use super::super::*;

        #[tokio::test]
        async fn test_returns_ok_correctly() {
            let namespace = "test_namespace";
            let limit = Limit::new(
                namespace,
                10,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );

            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);

            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                            key: "app.id".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                }],
                hits_addend: 4,
            };

            let response = rate_limiter
                .report(req.clone().into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Ok));
        }

        #[tokio::test]
        async fn test_going_overlimit_is_ok() {
            let namespace = "test_namespace";
            let limit = Limit::new(
                namespace,
                5,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );

            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);

            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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
                            key: "app.id".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                }],
                hits_addend: 20,
            };

            let response = rate_limiter
                .report(req.into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Ok));
        }

        #[tokio::test]
        async fn test_returns_ok_when_no_limits_apply() {
            // No limits saved
            let limiter = RateLimiter::new(10_000);
            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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

            let response = rate_limiter.report(req).await.unwrap().into_inner();

            assert_eq!(response.overall_code, i32::from(Code::Ok));
        }

        #[test]
        fn test_increments_report_calls_metric() {
            let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
            let handle: Arc<metrics_exporter_prometheus::PrometheusHandle> =
                recorder.handle().into();

            let namespace = "report_calls_test_namespace";
            let limit = Limit::new(
                namespace,
                10,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );

            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);

            let prometheus_metrics =
                Arc::new(PrometheusMetrics::new_with_handle(false, handle.clone()));
            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
                prometheus_metrics.clone(),
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
                            key: "app.id".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                    limit: None,
                }],
                hits_addend: 1,
            };

            // Use a fresh runtime (not inside an existing one) so block_on works
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();

            metrics::with_local_recorder(&recorder, || {
                rt.block_on(async {
                    rate_limiter
                        .report(req.clone().into_request())
                        .await
                        .unwrap();
                    rate_limiter
                        .report(req.clone().into_request())
                        .await
                        .unwrap();
                });
            });

            let metrics_output = handle.render();
            assert!(
                metrics_output.contains(&format!(
                    "report_calls{{limitador_namespace=\"{}\"}} 2",
                    namespace
                )),
                "Expected report_calls counter to be 2, got:\n{}",
                metrics_output
            );
        }

        #[tokio::test]
        async fn test_returns_unknown_when_domain_is_empty() {
            let limiter = RateLimiter::new(10_000);
            let rate_limiter = KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
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

            let response = rate_limiter.report(req).await.unwrap().into_inner();
            assert_eq!(response.overall_code, i32::from(Code::Unknown));
        }
    }

    mod reserve_and_commit {
        use tonic::IntoRequest;

        use limitador::limit::Limit;
        use limitador::RateLimiter;

        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::rate_limit_descriptor::Entry;
        use crate::envoy_rls::server::envoy::extensions::common::ratelimit::v3::RateLimitDescriptor;
        use crate::envoy_rls::server::tests::TEST_PROMETHEUS_HANDLE;

        use super::super::*;

        fn descriptor(app_id: &str) -> RateLimitDescriptor {
            RateLimitDescriptor {
                entries: vec![
                    Entry {
                        key: "req.method".to_string(),
                        value: "GET".to_string(),
                    },
                    Entry {
                        key: "app.id".to_string(),
                        value: app_id.to_string(),
                    },
                ],
                limit: None,
            }
        }

        fn service_with_limit(namespace: &str, max_value: u64) -> KuadrantService {
            let limit = Limit::new(
                namespace,
                max_value,
                60,
                vec!["descriptors[0]['req.method'] == 'GET'"
                    .try_into()
                    .expect("failed parsing!")],
                vec!["descriptors[0]['app.id']"
                    .try_into()
                    .expect("failed parsing!")],
            );
            let limiter = RateLimiter::new(10_000);
            limiter.add_limit(limit);
            KuadrantService::new(
                Arc::new(Limiter::Blocking(limiter)),
                Arc::new(PrometheusMetrics::new_with_handle(
                    false,
                    TEST_PROMETHEUS_HANDLE.clone(),
                )),
            )
        }

        #[tokio::test]
        async fn reserve_admits_and_commit_releases() {
            let namespace = "test_namespace";
            let service = service_with_limit(namespace, 10);

            let req = ReserveRequest {
                domain: namespace.to_string(),
                descriptors: vec![descriptor("1")],
                amount: 6,
                ttl: None,
            };

            let response = service
                .reserve(req.clone().into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.code, i32::from(Code::Ok));
            assert!(!response.reservation_id.is_empty());

            // Still held: 0 + outstanding(6) + 6 = 12 > 10
            let blocked = service
                .reserve(req.clone().into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(blocked.code, i32::from(Code::OverLimit));
            assert!(blocked.reservation_id.is_empty());

            let commit_req = CommitRequest {
                domain: namespace.to_string(),
                descriptors: vec![descriptor("1")],
                reservation_id: response.reservation_id,
                actual_amount: 2,
            };
            let commit_response = service
                .commit(commit_req.into_request())
                .await
                .unwrap()
                .into_inner();
            assert!(commit_response.reservation_released);

            // Counter now at 2, no outstanding reservations: 2 + 6 = 8 <= 10
            let after = service
                .reserve(req.into_request())
                .await
                .unwrap()
                .into_inner();
            assert_eq!(after.code, i32::from(Code::Ok));
        }

        #[tokio::test]
        async fn reserve_returns_unknown_when_domain_is_empty() {
            let service = service_with_limit("test_namespace", 10);

            let req = ReserveRequest {
                domain: "".to_string(),
                descriptors: vec![descriptor("1")],
                amount: 1,
                ttl: None,
            }
            .into_request();

            let response = service.reserve(req).await.unwrap().into_inner();
            assert_eq!(response.code, i32::from(Code::Unknown));
            assert!(response.reservation_id.is_empty());
        }

        #[tokio::test]
        async fn commit_returns_not_released_when_domain_is_empty() {
            let service = service_with_limit("test_namespace", 10);

            let req = CommitRequest {
                domain: "".to_string(),
                descriptors: vec![descriptor("1")],
                reservation_id: "some-id".to_string(),
                actual_amount: 1,
            }
            .into_request();

            let response = service.commit(req).await.unwrap().into_inner();
            assert!(!response.reservation_released);
        }

        #[tokio::test]
        async fn commit_degrades_gracefully_for_unknown_reservation() {
            let namespace = "test_namespace";
            let service = service_with_limit(namespace, 10);

            let commit_req = CommitRequest {
                domain: namespace.to_string(),
                descriptors: vec![descriptor("1")],
                reservation_id: "never-reserved".to_string(),
                actual_amount: 3,
            }
            .into_request();

            let response = service.commit(commit_req).await.unwrap().into_inner();
            assert!(!response.reservation_released);

            // actual_amount is still applied unconditionally: 3 + 8 = 11 > 10
            let req = ReserveRequest {
                domain: namespace.to_string(),
                descriptors: vec![descriptor("1")],
                amount: 8,
                ttl: None,
            }
            .into_request();
            let response = service.reserve(req).await.unwrap().into_inner();
            assert_eq!(response.code, i32::from(Code::OverLimit));
        }
    }
}
