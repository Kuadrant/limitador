use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::time::Instant;

use criterion::{black_box, criterion_group, criterion_main, Bencher, BenchmarkId, Criterion};
use rand::seq::SliceRandom;
use rand::SeedableRng;

use limitador::limit::{Context, Limit};
#[cfg(feature = "disk_storage")]
use limitador::storage::disk::{DiskStorage, OptimizeFor};
#[cfg(feature = "distributed_storage")]
use limitador::storage::distributed::CrInMemoryStorage;
use limitador::storage::in_memory::InMemoryStorage;
#[cfg(feature = "redis_storage")]
use limitador::storage::redis::CachedRedisStorageBuilder;
use limitador::storage::{AsyncCounterStorage, CounterStorage};
use limitador::{AsyncRateLimiter, RateLimiter};

const SEED: u64 = 42;

#[cfg(all(not(feature = "disk_storage"), not(feature = "redis_storage")))]
criterion_group!(benches, bench_in_mem);
#[cfg(all(feature = "disk_storage", not(feature = "redis_storage")))]
criterion_group!(benches, bench_in_mem, bench_disk);
#[cfg(all(not(feature = "disk_storage"), feature = "redis_storage"))]
criterion_group!(benches, bench_in_mem, bench_redis, bench_cached_redis);
#[cfg(all(
    feature = "disk_storage",
    feature = "redis_storage",
    not(feature = "distributed_storage")
))]
criterion_group!(
    benches,
    bench_in_mem,
    bench_disk,
    bench_redis,
    bench_cached_redis
);
#[cfg(all(
    feature = "disk_storage",
    feature = "redis_storage",
    feature = "distributed_storage"
))]
criterion_group!(
    benches,
    bench_in_mem,
    bench_disk,
    bench_redis,
    bench_cached_redis,
    bench_distributed,
);

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
        n_namespaces: 10,
        n_limits_per_ns: 50,
        n_conds_per_limit: 10,
        n_vars_per_limit: 0,
    },
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

struct TestCallParams<'a> {
    namespace: String,
    ctx: Context<'a>,
    delta: u64,
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
    let mut group = c.benchmark_group("Memory");
    for scenario in TEST_SCENARIOS {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<InMemoryStorage>::default();
                bench_is_rate_limited(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<InMemoryStorage>::default();
                bench_update_counters(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let storage = Box::<InMemoryStorage>::default();
                bench_check_rate_limited_and_update(b, test_scenario, storage);
            },
        );
    }
    group.finish();
}

#[cfg(feature = "distributed_storage")]
fn bench_distributed(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("Distributed");
    for scenario in TEST_SCENARIOS {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                runtime.block_on(async move {
                    let storage = Box::new(CrInMemoryStorage::new(
                        "test_node".to_owned(),
                        10_000,
                        "127.0.0.1:0".to_owned(),
                        vec![],
                    ));
                    bench_is_rate_limited(b, test_scenario, storage);
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                runtime.block_on(async move {
                    let storage = Box::new(CrInMemoryStorage::new(
                        "test_node".to_owned(),
                        10_000,
                        "127.0.0.1:0".to_owned(),
                        vec![],
                    ));
                    bench_update_counters(b, test_scenario, storage);
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                runtime.block_on(async move {
                    let storage = Box::new(CrInMemoryStorage::new(
                        "test_node".to_owned(),
                        10_000,
                        "127.0.0.1:0".to_owned(),
                        vec![],
                    ));
                    bench_check_rate_limited_and_update(b, test_scenario, storage);
                })
            },
        );
    }
    group.finish();
}
#[cfg(feature = "disk_storage")]
fn bench_disk(c: &mut Criterion) {
    let mut group = c.benchmark_group("Disk");
    for scenario in TEST_SCENARIOS.iter() {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let tmp = tempfile::TempDir::new().expect("We should have a dir!");
                let storage =
                    Box::new(DiskStorage::open(tmp.path(), OptimizeFor::Throughput).unwrap());
                bench_is_rate_limited(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let tmp = tempfile::TempDir::new().expect("We should have a dir!");
                let storage =
                    Box::new(DiskStorage::open(tmp.path(), OptimizeFor::Throughput).unwrap());
                bench_update_counters(b, test_scenario, storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                let tmp = tempfile::TempDir::new().expect("We should have a dir!");
                let storage =
                    Box::new(DiskStorage::open(tmp.path(), OptimizeFor::Throughput).unwrap());
                bench_check_rate_limited_and_update(b, test_scenario, storage);
            },
        );
    }
    group.finish();
}

#[cfg(feature = "redis_storage")]
fn bench_cached_redis(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    async fn create_storage() -> Box<dyn AsyncCounterStorage> {
        let storage_builder = CachedRedisStorageBuilder::new("redis://127.0.0.1:6379");
        let storage = storage_builder
            .build()
            .await
            .expect("We need a Redis running locally");
        storage.clear().await.unwrap();
        Box::new(storage)
    }

    let mut group = c.benchmark_group("CachedRedis");
    for scenario in TEST_SCENARIOS {
        group.bench_with_input(
            BenchmarkId::new("is_rate_limited", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                async_bench_is_rate_limited(&runtime, b, test_scenario, create_storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("update_counters", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                async_bench_update_counters(&runtime, b, test_scenario, create_storage);
            },
        );
        group.bench_with_input(
            BenchmarkId::new("check_rate_limited_and_update", scenario),
            scenario,
            |b: &mut Bencher, test_scenario: &&TestScenario| {
                async_bench_check_rate_limited_and_update(
                    &runtime,
                    b,
                    test_scenario,
                    create_storage,
                );
            },
        );
    }
    group.finish();
}

#[cfg(feature = "redis_storage")]
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

    let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);
    b.iter(|| {
        let params = call_params.choose(rng).unwrap();

        black_box(
            rate_limiter
                .is_rate_limited(
                    &params.namespace.to_owned().into(),
                    &params.ctx,
                    params.delta,
                    false, // FIXME: ??
                )
                .unwrap(),
        )
    })
}

fn async_bench_is_rate_limited<F>(
    runtime: &tokio::runtime::Runtime,
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: fn() -> F,
) where
    F: Future<Output = Box<dyn AsyncCounterStorage>>,
{
    b.to_async(runtime).iter_custom(|iters| async move {
        let storage = storage().await;
        let (rate_limiter, call_params) = generate_async_test_data(test_scenario, storage);
        let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);

        let start = Instant::now();
        for _i in 0..iters {
            black_box({
                let params = call_params.choose(rng).unwrap();
                rate_limiter
                    .is_rate_limited(
                        &params.namespace.to_owned().into(),
                        &params.ctx,
                        params.delta,
                    )
                    .await
                    .unwrap()
            });
        }
        start.elapsed()
    })
}

fn bench_update_counters(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();
    let (rate_limiter, call_params) = generate_test_data(test_scenario, storage);

    let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);
    b.iter(|| {
        let params = call_params.choose(rng).unwrap();

        rate_limiter
            .update_counters(
                &params.namespace.to_owned().into(),
                &params.ctx,
                params.delta,
            )
            .unwrap();
    })
}

fn async_bench_update_counters<F>(
    runtime: &tokio::runtime::Runtime,
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: fn() -> F,
) where
    F: Future<Output = Box<dyn AsyncCounterStorage>>,
{
    b.to_async(runtime).iter_custom(|iters| async move {
        let storage = storage().await;
        let (rate_limiter, call_params) = generate_async_test_data(test_scenario, storage);
        let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);

        let start = Instant::now();
        for _i in 0..iters {
            black_box({
                let params = call_params.choose(rng).unwrap();
                rate_limiter
                    .update_counters(
                        &params.namespace.to_owned().into(),
                        &params.ctx,
                        params.delta,
                    )
                    .await
            })
            .unwrap();
        }
        start.elapsed()
    })
}

fn bench_check_rate_limited_and_update(
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: Box<dyn CounterStorage>,
) {
    storage.clear().unwrap();
    let (rate_limiter, call_params) = generate_test_data(test_scenario, storage);

    let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);
    b.iter(|| {
        let params = call_params.choose(rng).unwrap();

        black_box(
            rate_limiter
                .check_rate_limited_and_update(
                    &params.namespace.to_owned().into(),
                    &params.ctx,
                    params.delta,
                    false,
                )
                .unwrap(),
        )
    })
}

fn async_bench_check_rate_limited_and_update<F>(
    runtime: &tokio::runtime::Runtime,
    b: &mut Bencher,
    test_scenario: &TestScenario,
    storage: fn() -> F,
) where
    F: Future<Output = Box<dyn AsyncCounterStorage>>,
{
    b.to_async(runtime).iter_custom(|iters| async move {
        let storage = storage().await;
        let (rate_limiter, call_params) = generate_async_test_data(test_scenario, storage);
        let rng = &mut rand::rngs::StdRng::seed_from_u64(SEED);

        let start = Instant::now();
        for _i in 0..iters {
            black_box({
                let params = call_params.choose(rng).unwrap();

                rate_limiter
                    .check_rate_limited_and_update(
                        &params.namespace.to_owned().into(),
                        &params.ctx,
                        params.delta,
                        false,
                    )
                    .await
                    .unwrap()
            });
        }
        start.elapsed()
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
) -> (RateLimiter, Vec<TestCallParams<'_>>) {
    let rate_limiter = RateLimiter::new_with_storage(storage);

    let (test_limits, call_params) = generate_test_limits(scenario);
    for limit in test_limits {
        rate_limiter.add_limit(limit);
    }

    (rate_limiter, call_params)
}

// Notice that this function creates all the limits with the same conditions and
// variables. Also, all the conditions have the same format: "cond_x == 1".
// That's to simplify things, those are not the aspects that should have the
// greatest impact on performance.
// The limits generated are big enough to avoid being rate-limited during the
// benchmark.
// Note that with this test data each request only increases one counter, we can
// that as another variable in the future.
fn generate_async_test_data(
    scenario: &TestScenario,
    storage: Box<dyn AsyncCounterStorage>,
) -> (AsyncRateLimiter, Vec<TestCallParams<'_>>) {
    let rate_limiter = AsyncRateLimiter::new_with_storage(storage);

    let (test_limits, call_params) = generate_test_limits(scenario);
    for limit in test_limits {
        rate_limiter.add_limit(limit);
    }

    (rate_limiter, call_params)
}

fn generate_test_limits(scenario: &TestScenario) -> (Vec<Limit>, Vec<TestCallParams<'_>>) {
    let mut test_values: HashMap<String, String> = HashMap::new();

    let mut conditions = vec![];
    for idx_cond in 0..scenario.n_conds_per_limit {
        let cond_name = format!("cond_{idx_cond}");
        conditions.push(
            format!("{cond_name} == '1'")
                .try_into()
                .expect("failed parsing!"),
        );
        test_values.insert(cond_name, "1".into());
    }

    let mut variables = vec![];
    for idx_var in 0..scenario.n_vars_per_limit {
        let var_name = format!("var_{idx_var}");
        variables.push(var_name.clone().try_into().expect("failed parsing!"));
        test_values.insert(var_name, "1".into());
    }

    let mut test_limits = vec![];
    let mut call_params: Vec<TestCallParams> = vec![];

    for idx_namespace in 0..scenario.n_namespaces {
        let namespace = idx_namespace.to_string();

        for limit_idx in 0..scenario.n_limits_per_ns {
            test_limits.push(Limit::new(
                namespace.clone(),
                u64::MAX,
                ((limit_idx * 60) + 10) as u64,
                conditions.clone(),
                variables.clone(),
            ))
        }

        call_params.push(TestCallParams {
            namespace,
            ctx: test_values.clone().into(),
            delta: 1,
        });
    }
    (test_limits, call_params)
}
