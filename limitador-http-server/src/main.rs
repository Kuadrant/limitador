use actix_web::{http::StatusCode, ResponseError};
use actix_web::{App, HttpServer};
use limitador::counter::Counter;
use limitador::limit::Limit;
use limitador::storage::redis::RedisStorage;
use limitador::RateLimiter;
use paperclip::actix::{
    api_v2_errors,
    api_v2_operation,
    // use this instead of actix_web::web
    web::{self, Json},
    Apiv2Schema,
    // extension trait for actix_web::App and proc-macro attributes
    OpenApiExt,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;

struct State {
    limiter: RateLimiter,
}

#[derive(Serialize, Deserialize, Apiv2Schema)]
struct CheckAndReportInfo {
    namespace: String,
    values: HashMap<String, String>,
    delta: i64,
}

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

#[api_v2_operation]
async fn create_limit(
    state: web::Data<State>,
    limit: web::Json<Limit>,
) -> Result<web::Json<()>, ErrorResponse> {
    match state.limiter.add_limit(&limit.into_inner()) {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn get_limits(
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<HashSet<Limit>>, ErrorResponse> {
    match state.limiter.get_limits(namespace.into_inner().as_str()) {
        Ok(limits) => Ok(Json(limits)),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn delete_limit(
    state: web::Data<State>,
    limit: web::Json<Limit>,
) -> Result<web::Json<()>, ErrorResponse> {
    match state.limiter.delete_limit(&limit) {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn delete_limits(
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<()>, ErrorResponse> {
    match state.limiter.delete_limits(namespace.into_inner().as_str()) {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn get_counters(
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<HashSet<Counter>>, ErrorResponse> {
    match state.limiter.get_counters(namespace.into_inner().as_str()) {
        Ok(counters) => Ok(Json(counters)),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn check(
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    match state
        .limiter
        .is_rate_limited(&request.namespace, &request.values, request.delta)
    {
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
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    match state
        .limiter
        .update_counters(&request.namespace, &request.values, request.delta)
    {
        Ok(_) => Ok(Json(())),
        Err(_) => Err(ErrorResponse::InternalServerError),
    }
}

#[api_v2_operation]
async fn check_and_report(
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let is_rate_limited = state.limiter.check_rate_limited_and_update(
        &request.namespace,
        &request.values,
        request.delta,
    );
    match is_rate_limited {
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

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    let rate_limiter = match env::var("REDIS_URL") {
        Ok(redis_url) => RateLimiter::new_with_storage(Box::new(RedisStorage::new(&redis_url))),
        Err(_) => RateLimiter::default(),
    };

    // Internally this uses Arc.
    // Ref: https://docs.rs/actix-web/2.0.0/actix_web/web/struct.Data.html
    let state = web::Data::new(State {
        limiter: rate_limiter,
    });

    let host = env::var("HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let port = env::var("PORT").unwrap_or_else(|_| String::from("8080"));
    let addr = format!("{}:{}", host, port);

    // This uses the paperclip crate to generate an OpenAPI spec.
    // Ref: https://paperclip.waffles.space/actix-plugin.html

    HttpServer::new(move || {
        App::new()
            .wrap_api()
            .with_json_spec_at("/api/spec")
            .app_data(state.clone())
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
    .bind(addr)?
    .run()
    .await
}
