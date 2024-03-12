use lazy_static::lazy_static;
use limitador::limit::Namespace;
use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const NAMESPACE_LABEL: &str = "limitador_namespace";
const LIMIT_NAME_LABEL: &str = "limit_name";

struct Metric {
    name: String,
    description: String,
}

lazy_static! {
    static ref AUTHORIZED_CALLS: Metric = Metric {
        name: "authorized_calls".into(),
        description: "Authorized calls".into(),
    };
    static ref LIMITED_CALLS: Metric = Metric {
        name: "limited_calls".into(),
        description: "Limited calls".into(),
    };
    static ref LIMITADOR_UP: Metric = Metric { // Can be used as a simple health check
        name: "limitador_up".into(),
        description: "Limitador is running".into(),
    };
    static ref DATASTORE_LATENCY: Metric = Metric {
        name: "counter_latency".into(),
        description: "Latency to the underlying counter datastore".into(),
    };
}

pub struct PrometheusMetrics {
    registry: Registry,
    authorized_calls: IntCounterVec,
    limited_calls: IntCounterVec,
    counter_latency: Histogram,
    use_limit_name_label: AtomicBool,
}

impl Default for PrometheusMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        Self::new_with_options(false)
    }

    // Note: This is optional because for a small number of limits it should be
    // fine, but it could become a problem when defining lots of limits. See the
    // caution note in the Prometheus docs:
    // https://prometheus.io/docs/practices/naming/#labels
    pub fn new_with_counters_by_limit_name() -> Self {
        Self::new_with_options(true)
    }

    pub fn set_use_limit_name_in_label(&self, use_limit_name_in_label: bool) {
        self.use_limit_name_label
            .store(use_limit_name_in_label, Ordering::SeqCst)
    }

    pub fn incr_authorized_calls(&self, namespace: &Namespace) {
        self.authorized_calls
            .with_label_values(&[namespace.as_ref()])
            .inc();
    }

    pub fn incr_limited_calls<'a, LN>(&self, namespace: &Namespace, limit_name: LN)
    where
        LN: Into<Option<&'a str>>,
    {
        let mut labels = vec![namespace.as_ref()];

        if self.use_limit_name_label.load(Ordering::Relaxed) {
            // If we have configured the metric to accept 2 labels we need to
            // set values for them.
            labels.push(limit_name.into().unwrap_or(""));
        }

        self.limited_calls.with_label_values(&labels).inc();
    }

    pub fn counter_access(&self, duration: Duration) {
        self.counter_latency.observe(duration.as_secs_f64());
    }

    pub fn gather_metrics(&self) -> String {
        let mut buffer = Vec::new();

        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buffer)
            .unwrap();

        String::from_utf8(buffer).unwrap()
    }

    fn new_with_options(use_limit_name_label: bool) -> Self {
        let authorized_calls_counter = Self::authorized_calls_counter();
        let limited_calls_counter = Self::limited_calls_counter(use_limit_name_label);
        let limitador_up_gauge = Self::limitador_up_gauge();
        let counter_latency = Self::counter_latency();

        let registry = Registry::new();

        registry
            .register(Box::new(authorized_calls_counter.clone()))
            .unwrap();

        registry
            .register(Box::new(limited_calls_counter.clone()))
            .unwrap();

        registry
            .register(Box::new(limitador_up_gauge.clone()))
            .unwrap();

        registry
            .register(Box::new(counter_latency.clone()))
            .unwrap();

        limitador_up_gauge.set(1);

        Self {
            registry,
            authorized_calls: authorized_calls_counter,
            limited_calls: limited_calls_counter,
            counter_latency,
            use_limit_name_label: AtomicBool::new(use_limit_name_label),
        }
    }

    fn authorized_calls_counter() -> IntCounterVec {
        IntCounterVec::new(
            Opts::new(&AUTHORIZED_CALLS.name, &AUTHORIZED_CALLS.description),
            &[NAMESPACE_LABEL],
        )
        .unwrap()
    }

    fn limited_calls_counter(use_limit_name_label: bool) -> IntCounterVec {
        let mut labels = vec![NAMESPACE_LABEL];

        if use_limit_name_label {
            labels.push(LIMIT_NAME_LABEL);
        }

        IntCounterVec::new(
            Opts::new(&LIMITED_CALLS.name, &LIMITED_CALLS.description),
            &labels,
        )
        .unwrap()
    }

    fn limitador_up_gauge() -> IntGauge {
        IntGauge::new(&LIMITADOR_UP.name, &LIMITADOR_UP.description).unwrap()
    }

    fn counter_latency() -> Histogram {
        Histogram::with_opts(HistogramOpts::new(
            &DATASTORE_LATENCY.name,
            &DATASTORE_LATENCY.description,
        ))
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_authorized_calls_by_namespace() {
        let prometheus_metrics = PrometheusMetrics::new();

        let namespaces_with_auth_counts = [
            ("some_namespace".into(), 2),
            ("another_namespace".into(), 3),
        ];

        namespaces_with_auth_counts
            .iter()
            .for_each(|(namespace, auth_count)| {
                for _ in 0..*auth_count {
                    prometheus_metrics.incr_authorized_calls(namespace)
                }
            });

        let metrics_output = prometheus_metrics.gather_metrics();

        namespaces_with_auth_counts
            .iter()
            .for_each(|(namespace, auth_count)| {
                assert!(metrics_output.contains(&formatted_counter_with_namespace(
                    &AUTHORIZED_CALLS.name,
                    *auth_count,
                    namespace
                )));
            });
    }

    #[test]
    fn shows_limited_calls_by_namespace() {
        let prometheus_metrics = PrometheusMetrics::new();

        let namespaces_with_limited_counts = [
            ("some_namespace".into(), 2),
            ("another_namespace".into(), 3),
        ];

        namespaces_with_limited_counts
            .iter()
            .for_each(|(namespace, limited_count)| {
                for _ in 0..*limited_count {
                    prometheus_metrics.incr_limited_calls(namespace, None)
                }
            });

        let metrics_output = prometheus_metrics.gather_metrics();

        namespaces_with_limited_counts
            .iter()
            .for_each(|(namespace, limited_count)| {
                assert!(metrics_output.contains(&formatted_counter_with_namespace(
                    &LIMITED_CALLS.name,
                    *limited_count,
                    namespace
                )));
            });
    }

    #[test]
    fn can_show_limited_calls_by_limit_name() {
        let prometheus_metrics = PrometheusMetrics::new_with_counters_by_limit_name();

        let limits_with_counts = [
            ("some_namespace".into(), "Some limit", 2),
            ("some_namespace".into(), "Another limit", 3),
        ];

        limits_with_counts
            .iter()
            .for_each(|(namespace, limit_name, limited_count)| {
                for _ in 0..*limited_count {
                    prometheus_metrics.incr_limited_calls(namespace, *limit_name)
                }
            });

        let metrics_output = prometheus_metrics.gather_metrics();

        limits_with_counts
            .iter()
            .for_each(|(namespace, limit_name, limited_count)| {
                assert!(
                    metrics_output.contains(&formatted_counter_with_namespace_and_limit(
                        &LIMITED_CALLS.name,
                        *limited_count,
                        namespace,
                        limit_name,
                    ))
                );
            });
    }

    #[test]
    fn incr_limited_calls_uses_empty_string_when_no_name() {
        let prometheus_metrics = PrometheusMetrics::new_with_counters_by_limit_name();
        let namespace = "some namespace".into();
        prometheus_metrics.incr_limited_calls(&namespace, None);

        let metrics_output = prometheus_metrics.gather_metrics();

        assert!(
            metrics_output.contains(&formatted_counter_with_namespace_and_limit(
                &LIMITED_CALLS.name,
                1,
                &namespace,
                "",
            ))
        );
    }

    #[test]
    fn shows_limitador_up_set_to_1() {
        let metrics_output = PrometheusMetrics::new().gather_metrics();
        assert!(metrics_output.contains("limitador_up 1"))
    }

    fn formatted_counter_with_namespace(
        metric_name: &str,
        count: i32,
        namespace: &Namespace,
    ) -> String {
        format!(
            "{}{{limitador_namespace=\"{}\"}} {}",
            metric_name,
            namespace.as_ref(),
            count,
        )
    }

    fn formatted_counter_with_namespace_and_limit(
        metric_name: &str,
        count: i32,
        namespace: &Namespace,
        limit_name: &str,
    ) -> String {
        format!(
            "{}{{limit_name=\"{}\",limitador_namespace=\"{}\"}} {}",
            metric_name,
            limit_name,
            namespace.as_ref(),
            count,
        )
    }
}
