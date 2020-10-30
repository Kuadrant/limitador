use crate::http_api::request_types::{CheckAndReportInfo, Counter, Limit};
use crate::Limiter;
use actix_web::{http::StatusCode, ResponseError};
use actix_web::{App, HttpServer};
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
async fn status() -> web::Json<()> {
    Json(())
}

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
        Limiter::Blocking(limiter) => limiter.get_limits(namespace.into_inner().as_str()),
        Limiter::Async(limiter) => limiter.get_limits(namespace.into_inner().as_str()).await,
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
        Limiter::Blocking(limiter) => limiter.delete_limits(namespace.into_inner().as_str()),
        Limiter::Async(limiter) => limiter.delete_limits(namespace.into_inner().as_str()).await,
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
        Limiter::Blocking(limiter) => limiter.get_counters(namespace.into_inner().as_str()),
        Limiter::Async(limiter) => limiter.get_counters(namespace.into_inner().as_str()).await,
    };

    match get_counters_result {
        Ok(counters) => {
            let mut resp_counters: Vec<Counter> = vec![];
            counters.iter().for_each(|c| {
                resp_counters.push(c.into());
            });
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
    let is_rate_limited_result = match state.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            limiter.is_rate_limited(request.namespace.as_str(), &request.values, request.delta)
        }
        Limiter::Async(limiter) => {
            limiter
                .is_rate_limited(request.namespace.as_str(), &request.values, request.delta)
                .await
        }
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
    let update_counters_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => {
            limiter.update_counters(request.namespace.as_str(), &request.values, request.delta)
        }
        Limiter::Async(limiter) => {
            limiter
                .update_counters(request.namespace.as_str(), &request.values, request.delta)
                .await
        }
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
    let rate_limited_and_update_result = match data.get_ref().as_ref() {
        Limiter::Blocking(limiter) => limiter.check_rate_limited_and_update(
            request.namespace.as_str(),
            &request.values,
            request.delta,
        ),
        Limiter::Async(limiter) => {
            limiter
                .check_rate_limited_and_update(
                    request.namespace.as_str(),
                    &request.values,
                    request.delta,
                )
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
