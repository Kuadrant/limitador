use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use limitador::limit::Limit;
use limitador::storage::redis::RedisStorage;
use limitador::RateLimiter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::sync::Mutex;

struct State {
    limiter: Mutex<RateLimiter>,
}

#[derive(Serialize, Deserialize)]
struct AuthRepRequest {
    namespace: String,
    values: HashMap<String, String>,
    delta: i64,
}

async fn create_limit(state: web::Data<State>, limit: web::Json<Limit>) -> impl Responder {
    match state.limiter.lock().unwrap().add_limit(limit.into_inner()) {
        Ok(_) => HttpResponse::Ok(),
        Err(_) => HttpResponse::InternalServerError(),
    }
}

async fn get_limits(state: web::Data<State>, namespace: web::Path<String>) -> HttpResponse {
    match state
        .limiter
        .lock()
        .unwrap()
        .get_limits(namespace.into_inner().as_str())
    {
        Ok(limits) => HttpResponse::Ok().json(limits),
        Err(_) => HttpResponse::InternalServerError().await.unwrap(),
    }
}

async fn delete_limit(state: web::Data<State>, limit: web::Json<Limit>) -> impl Responder {
    match state.limiter.lock().unwrap().delete_limit(&limit) {
        Ok(_) => HttpResponse::Ok(),
        Err(_) => HttpResponse::InternalServerError(),
    }
}

async fn delete_limits(state: web::Data<State>, namespace: web::Path<String>) -> impl Responder {
    match state
        .limiter
        .lock()
        .unwrap()
        .delete_limits(namespace.into_inner().as_str())
    {
        Ok(_) => HttpResponse::Ok(),
        Err(_) => HttpResponse::InternalServerError(),
    }
}

async fn get_counters(state: web::Data<State>, namespace: web::Path<String>) -> HttpResponse {
    match state
        .limiter
        .lock()
        .unwrap()
        .get_counters(namespace.into_inner().as_str())
    {
        Ok(counters) => HttpResponse::Ok().json(counters),
        Err(_) => HttpResponse::InternalServerError().await.unwrap(),
    }
}

async fn check(state: web::Data<State>, request: web::Json<AuthRepRequest>) -> impl Responder {
    match state.limiter.lock().unwrap().is_rate_limited(
        &request.namespace,
        &request.values,
        request.delta,
    ) {
        Ok(rate_limited) => {
            if rate_limited {
                HttpResponse::TooManyRequests()
            } else {
                HttpResponse::Ok()
            }
        }
        Err(_) => HttpResponse::InternalServerError(),
    }
}

async fn report(state: web::Data<State>, request: web::Json<AuthRepRequest>) -> impl Responder {
    match state.limiter.lock().unwrap().update_counters(
        &request.namespace,
        &request.values,
        request.delta,
    ) {
        Ok(_) => HttpResponse::Ok(),
        Err(_) => HttpResponse::InternalServerError(),
    }
}

async fn check_and_report(
    state: web::Data<State>,
    request: web::Json<AuthRepRequest>,
) -> impl Responder {
    match state.limiter.lock().unwrap().is_rate_limited(
        &request.namespace,
        &request.values,
        request.delta,
    ) {
        Ok(rate_limited) => {
            if rate_limited {
                HttpResponse::TooManyRequests()
            } else {
                match state.limiter.lock().unwrap().update_counters(
                    &request.namespace,
                    &request.values,
                    request.delta,
                ) {
                    Ok(_) => HttpResponse::Ok(),
                    Err(_) => HttpResponse::InternalServerError(),
                }
            }
        }
        Err(_) => HttpResponse::InternalServerError(),
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
        limiter: Mutex::new(rate_limiter),
    });

    let host = env::var("HOST").unwrap_or_else(|_| String::from("0.0.0.0"));
    let port = env::var("PORT").unwrap_or_else(|_| String::from("8080"));
    let addr = format!("{}:{}", host, port);

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route("/limits", web::post().to(create_limit))
            .route("/limits", web::delete().to(delete_limit))
            .route("/limits/{namespace}", web::get().to(get_limits))
            .route("/limits/{namespace}", web::delete().to(delete_limits))
            .route("/counters/{namespace}", web::get().to(get_counters))
            .route("/check_and_report", web::post().to(check_and_report))
            .route("/check", web::get().to(check))
            .route("/report", web::post().to(report))
    })
    .bind(addr)?
    .run()
    .await
}
