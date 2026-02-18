use crate::http_api::request_types::{CheckAndReportInfo, Counter, Limit};
use crate::prometheus_metrics::PrometheusMetrics;
use crate::{Limiter, Status};
use actix_web::{dev::Service, http::StatusCode, HttpResponse, HttpResponseBuilder, ResponseError};
use actix_web::{App, HttpServer};
use limitador::limit::Context;
use limitador::CheckResult;
use paperclip::actix::{
    api_v2_errors,
    api_v2_operation,
    // use this instead of actix_web::web
    web::{self, Json},
    // extension trait for actix_web::App and proc-macro attributes
    OpenApiExt,
};
use std::fmt;
use std::sync::{Arc, RwLock};
use tracing::{Instrument, Level};

struct RateLimitData {
    limiter: Arc<Limiter>,
    metrics: Arc<PrometheusMetrics>,
    status: Arc<RwLock<Status>>,
}

impl RateLimitData {
    fn new(
        limiter: Arc<Limiter>,
        metrics: Arc<PrometheusMetrics>,
        status: Arc<RwLock<Status>>,
    ) -> Self {
        Self {
            limiter,
            metrics,
            status,
        }
    }
    fn limiter(&self) -> &Limiter {
        self.limiter.as_ref()
    }

    fn metrics(&self) -> &PrometheusMetrics {
        self.metrics.as_ref()
    }

    fn status(&self) -> Status {
        *self.status.read().unwrap()
    }
}

#[api_v2_errors(500)]
#[derive(Debug)]
enum ErrorResponse {
    InternalServerError,
}

impl fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InternalServerError => write!(f, "Internal server error"),
        }
    }
}

impl ResponseError for ErrorResponse {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// Used for health checks
#[api_v2_operation]
async fn status(data: web::Data<RateLimitData>) -> web::Json<Status> {
    Json(data.get_ref().status())
}

#[tracing::instrument(skip(data))]
#[api_v2_operation]
async fn metrics(data: web::Data<RateLimitData>) -> String {
    data.get_ref().metrics().gather_metrics()
}

#[api_v2_operation]
#[tracing::instrument(skip(data))]
async fn get_limits(
    data: web::Data<RateLimitData>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Limit>>, ErrorResponse> {
    let namespace = &namespace.into_inner().into();
    let limits = match data.get_ref().limiter() {
        Limiter::Blocking(limiter) => limiter.get_limits(namespace),
        Limiter::Async(limiter) => limiter.get_limits(namespace),
    };
    let resp_limits: Vec<Limit> = limits.iter().map(|l| l.into()).collect();
    Ok(Json(resp_limits))
}

#[tracing::instrument(skip(data))]
#[api_v2_operation]
async fn get_counters(
    data: web::Data<RateLimitData>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Counter>>, ErrorResponse> {
    let namespace = namespace.into_inner().into();
    let get_counters_result = match data.get_ref().limiter() {
        Limiter::Blocking(limiter) => limiter.get_counters(&namespace),
        Limiter::Async(limiter) => limiter.get_counters(&namespace).await,
    };

    match get_counters_result {
        Ok(counters) => {
            let mut resp_counters: Vec<Counter> = vec![];
            for c in &counters {
                resp_counters.push(c.into());
            }
            Ok(Json(resp_counters))
        }
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[tracing::instrument(skip(state))]
#[api_v2_operation]
async fn check(
    state: web::Data<RateLimitData>,
    request: web::Json<CheckAndReportInfo>,
) -> HttpResponse {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
        response_headers,
    } = request.into_inner();
    let namespace = namespace.into();
    let mut ctx = Context::default();
    ctx.list_binding("descriptors".to_string(), vec![values]);
    let is_rate_limited_result = match state.get_ref().limiter() {
        Limiter::Blocking(limiter) => limiter.is_rate_limited(&namespace, &ctx, delta, response_headers.is_some()),
        Limiter::Async(limiter) => limiter.is_rate_limited(&namespace, &ctx, delta, response_headers.is_some()).await,
    };

    match is_rate_limited_result {
        Ok(mut rate_limited) => {
            if rate_limited.limited {
                match response_headers {
                    None => HttpResponse::TooManyRequests().json(()),
                    Some(response_headers) => {
                        let mut resp = HttpResponse::TooManyRequests();
                        add_response_header(
                            &mut resp,
                            response_headers.as_str(),
                            &mut rate_limited,
                        );
                        resp.json(())
                    }
                }
            } else {
                match response_headers {
                    None => HttpResponse::Ok().json(()),
                    Some(response_headers) => {
                        let mut resp = HttpResponse::Ok();
                        add_response_header(
                            &mut resp,
                            response_headers.as_str(),
                            &mut rate_limited,
                        );
                        resp.json(())
                    }
                }
            }
        }
        Err(_) => HttpResponse::InternalServerError().json(()),
    }
}

#[tracing::instrument(skip(data))]
#[api_v2_operation]
async fn report(
    data: web::Data<RateLimitData>,
    request: web::Json<CheckAndReportInfo>,
) -> HttpResponse {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
        response_headers,
    } = request.into_inner();
    let namespace = namespace.into();
    let mut ctx = Context::default();
    ctx.list_binding("descriptors".to_string(), vec![values.clone()]);
    let update_counters_result = match data.get_ref().limiter() {
        Limiter::Blocking(limiter) => limiter.update_counters(&namespace, &ctx, delta, response_headers.is_some()),
        Limiter::Async(limiter) => limiter.update_counters(&namespace, &ctx, delta, response_headers.is_some()).await,
    };

    match update_counters_result {
        Ok(_) => {
            match response_headers {
                None => HttpResponse::Ok().json(()),
                Some(response_headers) => {
                    // Get current limit state for headers by checking with delta=0
                    let check_result = match data.get_ref().limiter() {
                        Limiter::Blocking(limiter) => limiter.is_rate_limited(&namespace, &ctx, 0, true),
                        Limiter::Async(limiter) => limiter.is_rate_limited(&namespace, &ctx, 0, true).await,
                    };

                    match check_result {
                        Ok(mut rate_limited) => {
                            let mut resp = HttpResponse::Ok();
                            add_response_header(
                                &mut resp,
                                response_headers.as_str(),
                                &mut rate_limited,
                            );
                            resp.json(())
                        }
                        Err(_) => HttpResponse::Ok().json(()),
                    }
                }
            }
        }
        Err(_) => HttpResponse::InternalServerError().json(()),
    }
}

#[tracing::instrument(skip(data))]
#[api_v2_operation]
async fn check_and_report(
    data: web::Data<RateLimitData>,
    request: web::Json<CheckAndReportInfo>,
) -> HttpResponse {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
        response_headers,
    } = request.into_inner();
    let namespace = namespace.into();
    let mut ctx = Context::default();
    ctx.list_binding("descriptors".to_string(), vec![values]);
    let rate_limit_data = data.get_ref();
    let rate_limited_and_update_result = match rate_limit_data.limiter() {
        Limiter::Blocking(limiter) => limiter.check_rate_limited_and_update(
            &namespace,
            &ctx,
            delta,
            response_headers.is_some(),
        ),
        Limiter::Async(limiter) => {
            limiter
                .check_rate_limited_and_update(&namespace, &ctx, delta, response_headers.is_some())
                .await
        }
    };

    match rate_limited_and_update_result {
        Ok(mut is_rate_limited) => {
            if is_rate_limited.limited {
                rate_limit_data.metrics().incr_limited_calls(
                    &namespace,
                    is_rate_limited.limit_name.as_deref(),
                    &ctx,
                );

                match response_headers {
                    None => HttpResponse::TooManyRequests().json(()),
                    Some(response_headers) => {
                        let mut resp = HttpResponse::TooManyRequests();
                        add_response_header(
                            &mut resp,
                            response_headers.as_str(),
                            &mut is_rate_limited,
                        );
                        resp.json(())
                    }
                }
            } else {
                rate_limit_data
                    .metrics()
                    .incr_authorized_calls(&namespace, &ctx);
                rate_limit_data
                    .metrics()
                    .incr_authorized_hits(&namespace, &ctx, delta);

                match response_headers {
                    None => HttpResponse::Ok().json(()),
                    Some(response_headers) => {
                        let mut resp = HttpResponse::Ok();
                        add_response_header(
                            &mut resp,
                            response_headers.as_str(),
                            &mut is_rate_limited,
                        );
                        resp.json(())
                    }
                }
            }
        }
        Err(_) => HttpResponse::InternalServerError().json(()),
    }
}

pub fn add_response_header(
    resp: &mut HttpResponseBuilder,
    rate_limit_headers: &str,
    result: &mut CheckResult,
) {
    if rate_limit_headers == "DraftVersion03" {
        // creates response headers per https://datatracker.ietf.org/doc/id/draft-polli-ratelimit-headers-03.html
        let headers = result.response_header();
        if let Some(limit) = headers.get("X-RateLimit-Limit") {
            resp.insert_header(("X-RateLimit-Limit", limit.clone()));
        }
        if let Some(remaining) = headers.get("X-RateLimit-Remaining") {
            resp.insert_header(("X-RateLimit-Remaining".to_string(), remaining.clone()));
            if let Some(duration) = headers.get("X-RateLimit-Reset") {
                resp.insert_header(("X-RateLimit-Reset", duration.clone()));
            }
        }
    }
}

pub async fn run_http_server(
    address: &str,
    rate_limiter: Arc<Limiter>,
    prometheus_metrics: Arc<PrometheusMetrics>,
    status_reader: Arc<RwLock<Status>>,
) -> std::io::Result<()> {
    let data = web::Data::new(RateLimitData::new(
        rate_limiter,
        prometheus_metrics,
        status_reader,
    ));

    // This uses the paperclip crate to generate an OpenAPI spec.
    // Ref: https://paperclip.waffles.space/actix-plugin.html

    HttpServer::new(move || {
        App::new()
            .wrap_fn(|req, srv| {
                let span = if let Some(rid) = req.headers().get("X-Request-Id") {
                    let rid = rid.to_str().unwrap_or("invalid");
                    info!(
                        "x-request-id" = rid,
                        "Serving HTTP request {} {}",
                        req.method(),
                        req.path()
                    );
                    span!(Level::INFO, "http", "x-request-id" = rid)
                } else {
                    info!("Serving HTTP request {} {}", req.method(), req.path());
                    span!(Level::INFO, "http")
                };
                srv.call(req).instrument(span)
            })
            .wrap_api()
            .with_json_spec_at("/api/spec")
            .app_data(data.clone())
            .route("/status", web::get().to(status))
            .route("/metrics", web::get().to(metrics))
            .route("/limits/{namespace}", web::get().to(get_limits))
            .route("/counters/{namespace}", web::get().to(get_counters))
            .route("/check_and_report", web::post().to(check_and_report))
            .route("/check", web::post().to(check))
            .route("/report", web::post().to(report))
            .build()
    })
    .bind(address)?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envoy_rls::server::tests::TEST_PROMETHEUS_HANDLE;
    use crate::Configuration;
    use actix_web::{test, web};
    use limitador::limit::Limit as LimitadorLimit;
    use std::collections::HashMap;

    // All these tests use the in-memory storage implementation to simplify. We
    // know that some storage implementations like the Redis one trade
    // rate-limiting accuracy for performance. That would be a bit more
    // complicated to test.
    // Also, the logic behind these endpoints is well tested in the library,
    // that's why running some simple tests here should be enough.

    #[actix_rt::test]
    async fn test_status() {
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data)
                .route("/status", web::get().to(status)),
        )
        .await;

        let req = test::TestRequest::with_uri("/status").to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_metrics() {
        let rate_limiter: Arc<Limiter> =
            Arc::new(Limiter::new(Configuration::default()).await.unwrap());
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/metrics", web::get().to(metrics)),
        )
        .await;

        let req = test::TestRequest::get().uri("/metrics").to_request();
        let resp = test::call_and_read_body(&app, req).await;
        let resp_string = String::from_utf8(resp.to_vec()).unwrap();

        // No need to check the whole output. We just want to make sure that it
        // returns something with the prometheus format.
        assert!(resp_string.contains("# HELP limitador_up Limitador is running"));
    }

    #[actix_rt::test]
    async fn test_limits_read() {
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let namespace = "test_namespace";

        let limit = create_test_limit(&limiter, namespace, 10).await;
        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits/{namespace}", web::get().to(get_limits)),
        )
        .await;

        // Read limit created
        let req = test::TestRequest::get()
            .uri(&format!("/limits/{namespace}"))
            .data(data.clone())
            .to_request();
        let resp_limits: Vec<Limit> = test::call_and_read_body_json(&app, req).await;
        assert_eq!(resp_limits.len(), 1);
        assert_eq!(*resp_limits.first().unwrap(), Limit::from(&limit));
    }

    #[actix_rt::test]
    async fn test_check_and_report() {
        let limiter = Limiter::new(Configuration::default()).await.unwrap();

        // Create a limit with max == 1
        let namespace = "test_namespace";
        let _limit = create_test_limit(&limiter, namespace, 1).await;
        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/check_and_report", web::post().to(check_and_report)),
        )
        .await;

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: None,
        };

        // The first request should be OK
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("X-RateLimit-Limit"), None);
        assert_eq!(resp.headers().get("X-RateLimit-Remaining"), None);

        // The second request should be rate-limited
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[actix_rt::test]
    async fn test_check_and_report_with_draftversion03_response_headers() {
        let limiter = Limiter::new(Configuration::default()).await.unwrap();

        // Create a limit with max == 1
        let namespace = "test_namespace";
        let _limit = create_test_limit(&limiter, namespace, 2).await;
        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/check_and_report", web::post().to(check_and_report)),
        )
        .await;

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: Some("DraftVersion03".to_string()),
        };

        // The first request should be OK
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "1");

        // The 2nd request should be OK
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");

        // The 3rd request should be rate-limited
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");
    }

    #[actix_rt::test]
    async fn test_check_and_report_endpoints_separately() {
        let namespace = "test_namespace";
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let _limit = create_test_limit(&limiter, namespace, 1).await;

        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/check", web::post().to(check))
                .route("/report", web::post().to(report)),
        )
        .await;

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: None,
        };

        // Without making any requests, check should return OK
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Do the first report
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Should be rate-limited now
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[actix_rt::test]
    async fn test_check_with_response_headers() {
        let namespace = "test_namespace";
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let _limit = create_test_limit(&limiter, namespace, 2).await;

        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/check", web::post().to(check)),
        )
        .await;

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: Some("DraftVersion03".to_string()),
        };

        // First check should be OK with headers showing 2 remaining (check doesn't update counters)
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "2");

        // Second check should still be OK (check doesn't update counters)
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "2");
    }

    #[actix_rt::test]
    async fn test_report_with_response_headers() {
        let namespace = "test_namespace";
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let _limit = create_test_limit(&limiter, namespace, 2).await;

        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/report", web::post().to(report)),
        )
        .await;

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: Some("DraftVersion03".to_string()),
        };

        // First report should succeed with headers showing 1 remaining
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "1");

        // Second report should succeed with headers showing 0 remaining
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");

        // Third report should still succeed (report doesn't check limits) but show 0 remaining
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(
            resp.headers().get("X-RateLimit-Limit").unwrap(),
            "2, 2;w=60"
        );
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");
    }

    #[actix_rt::test]
    async fn test_check_and_report_with_response_headers() {
        let namespace = "test_namespace";
        let limiter = Limiter::new(Configuration::default()).await.unwrap();
        let _limit = create_test_limit(&limiter, namespace, 2).await;

        let rate_limiter: Arc<Limiter> = Arc::new(limiter);
        let prometheus_metrics: Arc<PrometheusMetrics> = Arc::new(
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone()),
        );
        let data = web::Data::new(RateLimitData::new(
            rate_limiter,
            prometheus_metrics,
            Default::default(),
        ));
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/check", web::post().to(check))
                .route("/report", web::post().to(report)),
        )
        .await;

        // Prepare values
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app.id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
            response_headers: Some("DraftVersion03".to_string()),
        };

        // Check should show 2 remaining (no counter updates yet)
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "2");

        // Report once - counter becomes 1, headers show 1 remaining (2 - 1)
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "1");

        // Check should show 1 remaining (current counter is 1, so 2-1=1 remaining)
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "1");

        // Report again - counter becomes 2, headers show 0 remaining (2 - 2)
        let req = test::TestRequest::post()
            .uri("/report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");

        // Check should now fail with 429 (counter at 2, delta 1 would exceed limit of 2)
        let req = test::TestRequest::post()
            .uri("/check")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers().get("X-RateLimit-Remaining").unwrap(), "0");
    }

    async fn create_test_limit(limiter: &Limiter, namespace: &str, max: u64) -> LimitadorLimit {
        // Create a limit
        let limit = LimitadorLimit::new(
            namespace,
            max,
            60,
            vec!["descriptors[0]['req.method'] == 'GET'"
                .try_into()
                .expect("failed parsing!")],
            vec!["descriptors[0]['app.id']"
                .try_into()
                .expect("failed parsing!")],
        );

        match &limiter {
            Limiter::Blocking(limiter) => limiter.add_limit(limit.clone()),
            Limiter::Async(limiter) => limiter.add_limit(limit.clone()),
        };
        limit
    }
}
