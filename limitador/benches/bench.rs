use criterion::{black_box, criterion_group, criterion_main, Bencher, BenchmarkId, Criterion};
use rand::seq::SliceRandom;

use limitador::limit::Limit;
use limitador::storage::CounterStorage;
use limitador::RateLimiter;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

#[cfg(not(feature = "redis"))]
criterion_group!(benches, bench_in_mem);
#[cfg(feature = "redis")]
criterion_group!(benches, bench_in_mem, bench_redis);

criterion_main!(benches);

#[derive(Debug, Clone)]
struct TestScenario {
    n_namespaces: u32,
    n_limits_per_ns: u32,
    n_conds_per_limit: u32,
    n_vars_per_limit: u32,
}

const TEST_SCENARIOS: &[&TestScenario] = &[
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

impl Display for TestScenario {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} namespaces with {} limits each with {} conditions and {} variables",
            self.n_namespaces, self.n_limits_per_ns, self.n_conds_per_limit, self.n_vars_per_limit
        )
    }
}

fn bench_in_mem(c: &mut Criterion) {
    let mut group = c.benchmark_group("In memory");
    for scenario in TEST_SCENARIOS {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::in_memory::InMemoryStorage>::default();
                bench_is_rate_limited(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::in_memory::InMemoryStorage>::default();
                bench_update_counters(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::in_memory::InMemoryStorage>::default();
                bench_check_rate_limited_and_update(b, test_scenario, storage);
            },
        );
    }
    group.finish();
}

#[cfg(feature = "redis")]
fn bench_redis(c: &mut Criterion) {
    let mut group = c.benchmark_group("Redis");
    for scenario in TEST_SCENARIOS {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::redis::RedisStorage>::default();
                bench_is_rate_limited(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::redis::RedisStorage>::default();
                bench_update_counters(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<limitador::storage::redis::RedisStorage>::default();
                bench_check_rate_limited_and_update(b, test_scenario, storage);
            },
        );
    }
    group.finish();
}

fn bench_is_rate_limited(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();

    let (rate_limiter, call_params) = generate_test_data(test_scenario, storage);

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
    let (rate_limiter, call_params) = generate_test_data(test_scenario, storage);

    b.iter(|| {
        let params = call_params.choose(&mut rand::thread_rng()).unwrap();

        rate_limiter
            .update_counters(
                &params.namespace.to_owned().into(),
                &params.values,
                params.delta,
            )
            .unwrap();
        black_box(())
    })
}

fn bench_check_rate_limited_and_update(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();
    let (rate_limiter, call_params) = generate_test_data(test_scenario, storage);

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
        let cond_name = format!("cond_{idx_cond}");
        conditions.push(format!("{cond_name} == '1'"));
        test_values.insert(cond_name, "1".into());
    }

    let mut variables = vec![];
    for idx_var in 0..scenario.n_vars_per_limit {
        let var_name = format!("var_{idx_var}");
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
                i64::MAX,
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
