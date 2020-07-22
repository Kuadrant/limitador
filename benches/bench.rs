use criterion::{black_box, criterion_group, criterion_main, Criterion};

use limitador::limit::Limit;
use limitador::RateLimiter;
use std::collections::HashMap;

fn bench_is_rate_limited(c: &mut Criterion) {
    let rate_limiter = test_rate_limiter();
    let values = test_values();

    c.bench_function("is rate limited", |b| {
        b.iter(|| {
            black_box(
                rate_limiter
                    .is_rate_limited("test_namespace", &values, 1)
                    .unwrap(),
            )
        })
    });
}

fn bench_update_counters(c: &mut Criterion) {
    let mut rate_limiter = test_rate_limiter();
    let values = test_values();

    c.bench_function("update counters", |b| {
        b.iter(|| {
            black_box(
                rate_limiter
                    .update_counters("test_namespace", &values, 1)
                    .unwrap(),
            )
        });
    });
}

fn test_rate_limiter() -> RateLimiter {
    let limit = Limit::new(
        "test_namespace",
        100_000_000_000,
        60,
        vec!["req.method == GET"],
        vec!["req.method", "app_id"],
    );

    let mut rate_limiter = RateLimiter::default();
    rate_limiter.add_limit(limit).unwrap();

    rate_limiter
}

fn test_values() -> HashMap<String, String> {
    let mut values: HashMap<String, String> = HashMap::new();
    values.insert("req.method".to_string(), "GET".to_string());
    values.insert("app_id".to_string(), "test_app_id".to_string());
    values
}

criterion_group!(benches, bench_is_rate_limited, bench_update_counters);
criterion_main!(benches);
