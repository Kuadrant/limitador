use criterion::{black_box, criterion_group, criterion_main, Bencher, Criterion};
use rand::seq::SliceRandom;

use limitador::limit::Limit;
use limitador::storage::in_memory::InMemoryStorage;
use limitador::storage::redis::RedisStorage;
use limitador::storage::CounterStorage;
use limitador::RateLimiter;
use std::collections::HashMap;

criterion_group!(
    benches,
    bench_is_rate_limited_in_mem,
    bench_is_rate_limited_redis,
    bench_update_counters_in_mem,
    bench_update_counters_redis,
    bench_check_rate_limited_and_update_in_mem,
    bench_check_rate_limited_and_update_redis,
);
criterion_main!(benches);

#[derive(Debug, Clone)]
struct TestScenario {
    n_namespaces: u32,
    n_limits_per_ns: u32,
    n_conds_per_limit: u32,
    n_vars_per_limit: u32,
}

const TEST_SCENARIOS: &'static [&'static TestScenario] = &[
    &TestScenario {
        n_namespaces: 1,
        n_limits_per_ns: 1,
        n_conds_per_limit: 1,
        n_vars_per_limit: 1,
    },
    &TestScenario {
        n_namespaces: 10,
        n_limits_per_ns: 10,
        n_conds_per_limit: 10,
        n_vars_per_limit: 10,
    },
    &TestScenario {
        n_namespaces: 10,
        n_limits_per_ns: 50,
        n_conds_per_limit: 10,
        n_vars_per_limit: 10,
    },
];

struct TestCallParams {
    namespace: String,
    values: HashMap<String, String>,
    delta: i64,
}

fn bench_is_rate_limited_in_mem(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("is_rate_limited (in mem)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(InMemoryStorage::default());
            bench_is_rate_limited(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_is_rate_limited_redis(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("is_rate_limited (redis)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(RedisStorage::default());
            bench_is_rate_limited(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_update_counters_in_mem(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("update_counters (in mem)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(InMemoryStorage::default());
            bench_update_counters(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_update_counters_redis(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("update_counters (redis)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(RedisStorage::default());
            bench_update_counters(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_check_rate_limited_and_update_in_mem(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("check_rate_limited_and_update (in mem)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(InMemoryStorage::default());
            bench_check_rate_limited_and_update(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_check_rate_limited_and_update_redis(c: &mut Criterion) {
    c.bench_function_over_inputs(
        &format!("check_rate_limited_and_update (redis)"),
        |b: &mut Bencher, test_scenario: &&TestScenario| {
            let storage = Box::new(RedisStorage::default());
            bench_check_rate_limited_and_update(b, test_scenario, storage);
        },
        TEST_SCENARIOS.to_vec(),
    );
}

fn bench_is_rate_limited(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();

    let (rate_limiter, call_params) = generate_test_data(&test_scenario, storage);

    b.iter(|| {
        let params = call_params.choose(&mut rand::thread_rng()).unwrap();

        black_box(
            rate_limiter
                .is_rate_limited(
                    &params.namespace.to_owned().into(),
                    &params.values,
                    params.delta,
                )
                .unwrap(),
        )
    })
}

fn bench_update_counters(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();
    let (rate_limiter, call_params) = generate_test_data(&test_scenario, storage);

    b.iter(|| {
        let params = call_params.choose(&mut rand::thread_rng()).unwrap();

        black_box(
            rate_limiter
                .update_counters(
                    &params.namespace.to_owned().into(),
                    &params.values,
                    params.delta,
                )
                .unwrap(),
        )
    })
}

fn bench_check_rate_limited_and_update(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();
    let (rate_limiter, call_params) = generate_test_data(&test_scenario, storage);

    b.iter(|| {
        let params = call_params.choose(&mut rand::thread_rng()).unwrap();

        black_box(
            rate_limiter
                .check_rate_limited_and_update(
                    &params.namespace.to_owned().into(),
                    &params.values,
                    params.delta,
                )
                .unwrap(),
        )
    })
}

// Notice that this function creates all the limits with the same conditions and
// variables. Also, all the conditions have the same format: "cond_x == 1".
// That's to simplify things, those are not the aspects that should have the
// greatest impact on performance.
// The limits generated are big enough to avoid being rate-limited during the
// benchmark.
// Note that with this test data each request only increases one counter, we can
// that as another variable in the future.
fn generate_test_data(
    scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) -> (RateLimiter, Vec<TestCallParams>) {
    let mut test_values: HashMap<String, String> = HashMap::new();

    let mut conditions = vec![];
    for idx_cond in 0..scenario.n_conds_per_limit {
        let cond_name = format!("cond_{}", idx_cond.to_string());
        conditions.push(format!("{} == 1", cond_name));
        test_values.insert(cond_name, "1".into());
    }

    let mut variables = vec![];
    for idx_var in 0..scenario.n_vars_per_limit {
        let var_name = format!("var_{}", idx_var.to_string());
        variables.push(var_name.clone());
        test_values.insert(var_name, "1".into());
    }

    let mut test_limits = vec![];
    let mut call_params: Vec<TestCallParams> = vec![];

    for idx_namespace in 0..scenario.n_namespaces {
        let namespace = idx_namespace.to_string();

        for _ in 0..scenario.n_limits_per_ns {
            test_limits.push(Limit::new(
                namespace.clone(),
                100_000_000_000,
                10,
                conditions.clone(),
                variables.clone(),
            ))
        }

        call_params.push(TestCallParams {
            namespace,
            values: test_values.clone(),
            delta: 1,
        });
    }

    let rate_limiter = RateLimiter::new_with_storage(storage);

    for limit in test_limits {
        rate_limiter.add_limit(limit);
    }

    (rate_limiter, call_params)
}
