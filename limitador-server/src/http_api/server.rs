use crate::http_api::request_types::{CheckAndReportInfo, Counter, Limit};
use crate::Limiter;
use actix_web::{http::StatusCode, ResponseError};
use actix_web::{App, HttpServer};
use limitador::limit::Namespace;
use paperclip::actix::{
    api_v2_errors,
    api_v2_operation,
    // use this instead of actix_web::web
    web::{self, Json},
    // extension trait for actix_web::App and proc-macro attributes
    OpenApiExt,
};
use std::fmt;
use std::sync::Arc;

#[api_v2_errors(429, 500)]
#[derive(Debug)]
enum ErrorResponse {
    TooManyRequests,
    InternalServerError,
}

impl fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRequests => write!(f, "Too many requests"),
            Self::InternalServerError => write!(f, "Internal server error"),
        }
    }
}

impl ResponseError for ErrorResponse {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// Used for health checks
#[api_v2_operation]
async fn status() -> web::Json<()> {
    Json(())
}

#[api_v2_operation]
async fn metrics(data: web::Data<Arc<Limiter>>) -> String {
    match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.gather_prometheus_metrics(),
        Limiter::Async(limiter) => limiter.gather_prometheus_metrics(),
    }
}

#[api_v2_operation]
async fn create_limit(
    data: web::Data<Arc<Limiter>>,
    limit: web::Json<Limit>,
) -> Result<web::Json<()>, ErrorResponse> {
    let add_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.add_limit(&limit.into_inner().into()),
        Limiter::Async(limiter) => limiter.add_limit(&limit.into_inner().into()).await,
    };

    match add_result {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn get_limits(
    data: web::Data<Arc<Limiter>>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Limit>>, ErrorResponse> {
    let get_limits_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.get_limits(namespace)
        }
        Limiter::Async(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.get_limits(namespace).await
        }
    };

    match get_limits_result {
        Ok(limits) => {
            let resp_limits: Vec<Limit> = limits.iter().map(|l| l.into()).collect();
            Ok(Json(resp_limits))
        }
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn delete_limit(
    data: web::Data<Arc<Limiter>>,
    limit: web::Json<Limit>,
) -> Result<web::Json<()>, ErrorResponse> {
    let delete_limit_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.delete_limit(&limit.into_inner().into()),
        Limiter::Async(limiter) => limiter.delete_limit(&limit.into_inner().into()).await,
    };

    match delete_limit_result {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn delete_limits(
    data: web::Data<Arc<Limiter>>,
    namespace: web::Path<String>,
) -> Result<web::Json<()>, ErrorResponse> {
    let delete_limits_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.delete_limits(namespace)
        }
        Limiter::Async(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.delete_limits(namespace).await
        }
    };

    match delete_limits_result {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn get_counters(
    data: web::Data<Arc<Limiter>>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Counter>>, ErrorResponse> {
    let get_counters_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.get_counters(namespace)
        }
        Limiter::Async(limiter) => {
            let namespace: Namespace = namespace.into_inner().into();
            limiter.get_counters(namespace).await
        }
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

#[api_v2_operation]
async fn check(
    state: web::Data<Arc<Limiter>>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
    } = request.into_inner();
    let is_rate_limited_result = match state.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.is_rate_limited(namespace, &values, delta),
        Limiter::Async(limiter) => limiter.is_rate_limited(namespace, &values, delta).await,
    };

    match is_rate_limited_result {
        Ok(rate_limited) => {
            if rate_limited {
                Err(ErrorResponse::TooManyRequests)
            } else {
                Ok(Json(()))
            }
        }
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn report(
    data: web::Data<Arc<Limiter>>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
    } = request.into_inner();
    let update_counters_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.update_counters(namespace, &values, delta),
        Limiter::Async(limiter) => limiter.update_counters(namespace, &values, delta).await,
    };

    match update_counters_result {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn check_and_report(
    data: web::Data<Arc<Limiter>>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let CheckAndReportInfo {
        namespace,
        values,
        delta,
    } = request.into_inner();
    let rate_limited_and_update_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            limiter.check_rate_limited_and_update(namespace, &values, delta)
        }
        Limiter::Async(limiter) => {
            limiter
                .check_rate_limited_and_update(namespace, &values, delta)
                .await
        }
    };

    match rate_limited_and_update_result {
        Ok(is_rate_limited) => {
            if is_rate_limited {
                Err(ErrorResponse::TooManyRequests)
            } else {
                Ok(Json(()))
            }
        }
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

pub async fn run_http_server(address: &str, rate_limiter: Arc<Limiter>) -> std::io::Result<()> {
    let data = web::Data::new(rate_limiter);

    // This uses the paperclip crate to generate an OpenAPI spec.
    // Ref: https://paperclip.waffles.space/actix-plugin.html

    HttpServer::new(move || {
        App::new()
            .wrap_api()
            .with_json_spec_at("/api/spec")
            .app_data(data.clone())
            .route("/status", web::get().to(status))
            .route("/metrics", web::get().to(metrics))
            .route("/limits", web::post().to(create_limit))
            .route("/limits", web::delete().to(delete_limit))
            .route("/limits/{namespace}", web::get().to(get_limits))
            .route("/limits/{namespace}", web::delete().to(delete_limits))
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
        let app = test::init_service(App::new().route("/status", web::get().to(status))).await;

        let req = test::TestRequest::with_uri("/status").to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_metrics() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
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
    async fn test_limits_create_read_delete() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits", web::post().to(create_limit))
                .route("/limits", web::delete().to(delete_limit))
                .route("/limits/{namespace}", web::get().to(get_limits)),
        )
        .await;

        let namespace = "test_namespace";

        // Create a limit
        let limit = Limit::from(&LimitadorLimit::new(
            namespace,
            10,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        ));
        let req = test::TestRequest::post()
            .uri("/limits")
            .data(data.clone())
            .set_json(&limit)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Read limit created
        let req = test::TestRequest::get()
            .uri(&format!("/limits/{}", namespace))
            .data(data.clone())
            .to_request();
        let resp_limits: Vec<Limit> = test::call_and_read_body_json(&app, req).await;
        assert_eq!(resp_limits.len(), 1);
        assert_eq!(*resp_limits.get(0).unwrap(), limit);

        // Delete limit
        let req = test::TestRequest::delete()
            .uri("/limits")
            .data(data.clone())
            .set_json(&limit)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Check that the list is now empty
        let req = test::TestRequest::get()
            .uri(&format!("/limits/{}", namespace))
            .data(data.clone())
            .to_request();
        let resp_limits: Vec<Limit> = test::call_and_read_body_json(&app, req).await;
        assert!(resp_limits.is_empty());
    }

    #[actix_rt::test]
    async fn test_create_limit_with_name() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits", web::post().to(create_limit))
                .route("/limits/{namespace}", web::get().to(get_limits)),
        )
        .await;

        let namespace = "test_namespace";

        // Create a limit
        let mut limitador_limit =
            LimitadorLimit::new(namespace, 10, 60, vec!["req.method == GET"], vec!["app_id"]);
        limitador_limit.set_name("Test Limit".into());

        let limit = Limit::from(&limitador_limit);

        let req = test::TestRequest::post()
            .uri("/limits")
            .data(data.clone())
            .set_json(&limit)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Read limit created
        let req = test::TestRequest::get()
            .uri(&format!("/limits/{}", namespace))
            .data(data.clone())
            .to_request();
        let resp_limits: Vec<Limit> = test::call_and_read_body_json(&app, req).await;
        assert_eq!(resp_limits.len(), 1);
        assert_eq!(*resp_limits.get(0).unwrap(), limit);
    }

    #[actix_rt::test]
    async fn test_delete_all_limits_of_namespace() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits", web::post().to(create_limit))
                .route("/limits/{namespace}", web::get().to(get_limits))
                .route("/limits/{namespace}", web::delete().to(delete_limits)),
        )
        .await;

        let namespace = "test_namespace";

        // Create several limits
        let limits = vec![
            Limit::from(&LimitadorLimit::new(
                namespace,
                10,
                60,
                vec!["req.method == GET"],
                vec!["app_id"],
            )),
            Limit::from(&LimitadorLimit::new(
                namespace,
                5,
                60,
                vec!["req.method == POST"],
                vec!["app_id"],
            )),
        ];

        for limit in limits {
            let req = test::TestRequest::post()
                .uri("/limits")
                .data(data.clone())
                .set_json(&limit)
                .to_request();
            let resp = test::call_service(&app, req).await;
            assert!(resp.status().is_success());
        }

        // Delete all the limits
        let req = test::TestRequest::delete()
            .uri(&format!("/limits/{}", namespace))
            .data(data.clone())
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Check that the list is now empty
        let req = test::TestRequest::get()
            .uri(&format!("/limits/{}", namespace))
            .data(data.clone())
            .to_request();
        let resp_limits: Vec<Limit> = test::call_and_read_body_json(&app, req).await;
        assert!(resp_limits.is_empty());
    }

    #[actix_rt::test]
    async fn test_check_and_report() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits", web::post().to(create_limit))
                .route("/check_and_report", web::post().to(check_and_report)),
        )
        .await;

        // Crate a limit with max == 1

        let namespace = "test_namespace";

        let limit = Limit::from(&LimitadorLimit::new(
            namespace,
            1,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        ));

        let req = test::TestRequest::post()
            .uri("/limits")
            .data(data.clone())
            .set_json(&limit)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app_id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
        };

        // The first request should be OK
        let req = test::TestRequest::post()
            .uri("/check_and_report")
            .data(data.clone())
            .set_json(&info)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

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
    async fn test_check_and_report_endpoints_separately() {
        let rate_limiter: Arc<Limiter> = Arc::new(Limiter::new().await.unwrap());
        let data = web::Data::new(rate_limiter);
        let app = test::init_service(
            App::new()
                .app_data(data.clone())
                .route("/limits", web::post().to(create_limit))
                .route("/check", web::post().to(check))
                .route("/report", web::post().to(report)),
        )
        .await;

        // Crate a limit with max == 1

        let namespace = "test_namespace";

        let limit = Limit::from(&LimitadorLimit::new(
            namespace,
            1,
            60,
            vec!["req.method == GET"],
            vec!["app_id"],
        ));

        let req = test::TestRequest::post()
            .uri("/limits")
            .data(data.clone())
            .set_json(&limit)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Prepare values to check
        let mut values = HashMap::new();
        values.insert("req.method".into(), "GET".into());
        values.insert("app_id".into(), "1".into());
        let info = CheckAndReportInfo {
            namespace: namespace.into(),
            values,
            delta: 1,
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
}
