#![feature(test)]

extern crate test;

#[cfg(test)]
mod tests {
    use limitador::limit::Limit;
    use limitador::RateLimiter;
    use std::collections::HashMap;
    use test::{black_box, Bencher};

    #[bench]
    fn bench_is_rate_limited(b: &mut Bencher) {
        let rate_limiter = test_rate_limiter();
        let values = test_values();

        b.iter(|| {
            black_box(rate_limiter.is_rate_limited(&values, 1).unwrap());
        });
    }

    #[bench]
    fn bench_update_counters(b: &mut Bencher) {
        let mut rate_limiter = test_rate_limiter();
        let values = test_values();

        b.iter(|| {
            black_box(rate_limiter.update_counters(&values, 1).unwrap());
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
        values.insert("namespace".to_string(), "test_namespace".to_string());
        values.insert("req.method".to_string(), "GET".to_string());
        values.insert("app_id".to_string(), "test_app_id".to_string());
        values
    }
}
