use crate::request_types::{CheckAndReportInfo, Counter, Limit};
use actix_web::{http::StatusCode, ResponseError};
use actix_web::{App, HttpServer};
use limitador::storage::redis::AsyncRedisStorage;
use limitador::{AsyncRateLimiter, RateLimiter};
use paperclip::actix::{
    api_v2_errors,
    api_v2_operation,
    // use this instead of actix_web::web
    web::{self, Json},
    // extension trait for actix_web::App and proc-macro attributes
    OpenApiExt,
};
use std::env;
use std::fmt;

mod request_types;

pub enum Limiter {
    Blocking(RateLimiter),
    Async(AsyncRateLimiter),
}

struct State {
    limiter: Limiter,
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
    let add_result = match &state.limiter {
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
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Limit>>, ErrorResponse> {
    let get_limits_result = match &state.limiter {
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
    state: web::Data<State>,
    limit: web::Json<Limit>,
) -> Result<web::Json<()>, ErrorResponse> {
    let delete_limit_result = match &state.limiter {
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
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<()>, ErrorResponse> {
    let delete_limits_result = match &state.limiter {
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
    state: web::Data<State>,
    namespace: web::Path<String>,
) -> Result<web::Json<Vec<Counter>>, ErrorResponse> {
    let get_counters_result = match &state.limiter {
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
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let is_rate_limited_result = match &state.limiter {
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
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let update_counters_result = match &state.limiter {
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
    state: web::Data<State>,
    request: web::Json<CheckAndReportInfo>,
) -> Result<web::Json<()>, ErrorResponse> {
    let rate_limited_and_update_result = match &state.limiter {
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

fn new_limiter() -> Limiter {
    match env::var("REDIS_URL") {
        // Let's use the async impl. This could be configurable if needed.
        Ok(redis_url) => {
            let async_limiter =
                AsyncRateLimiter::new_with_storage(Box::new(AsyncRedisStorage::new(&redis_url)));
            Limiter::Async(async_limiter)
        }
        Err(_) => Limiter::Blocking(RateLimiter::default()),
    }
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    let host = env::var("HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let port = env::var("PORT").unwrap_or_else(|_| String::from("8080"));
    let addr = format!("{}:{}", host, port);

    // Internally, web::Data this uses Arc.
    // Ref: https://docs.rs/actix-web/2.0.0/actix_web/web/struct.Data.html
    let state = web::Data::new(State {
        limiter: new_limiter(),
    });

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
