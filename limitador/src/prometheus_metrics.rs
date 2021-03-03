use crate::limit::Namespace;
use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};

const NAMESPACE_LABEL: &str = "limitador_namespace";

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
}

pub struct PrometheusMetrics {
    registry: Registry,
    authorized_calls: IntCounterVec,
    limited_calls: IntCounterVec,
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        let labels = vec![NAMESPACE_LABEL];

        let authorized_calls_counter = Self::authorized_calls_counter(&labels);
        let limited_calls_counter = Self::limited_calls_counter(&labels);
        let limitador_up_gauge = Self::limitador_up_gauge();

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

        limitador_up_gauge.set(1);

        Self {
            registry,
            authorized_calls: authorized_calls_counter,
            limited_calls: limited_calls_counter,
        }
    }

    pub fn incr_authorized_calls(&self, namespace: &Namespace) {
        self.authorized_calls
            .with_label_values(&[namespace.as_ref()])
            .inc();
    }

    pub fn incr_limited_calls(&self, namespace: &Namespace) {
        self.limited_calls
            .with_label_values(&[namespace.as_ref()])
            .inc();
    }

    pub fn gather_metrics(&self) -> String {
        let mut buffer = Vec::new();

        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buffer)
            .unwrap();

        String::from_utf8(buffer).unwrap()
    }

    fn authorized_calls_counter(labels: &[&str]) -> IntCounterVec {
        IntCounterVec::new(
            Opts::new(&AUTHORIZED_CALLS.name, &AUTHORIZED_CALLS.description),
            &labels,
        )
        .unwrap()
    }

    fn limited_calls_counter(labels: &[&str]) -> IntCounterVec {
        IntCounterVec::new(
            Opts::new(&LIMITED_CALLS.name, &LIMITED_CALLS.description),
            &labels,
        )
        .unwrap()
    }

    fn limitador_up_gauge() -> IntGauge {
        IntGauge::new(&LIMITADOR_UP.name, &LIMITADOR_UP.description).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_authorized_calls_by_namespace() {
        let prometheus_metrics = PrometheusMetrics::new();

        let namespaces_with_auth_counts = [
            (Namespace::from("some_namespace"), 2),
            (Namespace::from("another_namespace"), 3),
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
                assert!(metrics_output.contains(&formatted_counter(
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
            (Namespace::from("some_namespace"), 2),
            (Namespace::from("another_namespace"), 3),
        ];

        namespaces_with_limited_counts
            .iter()
            .for_each(|(namespace, limited_count)| {
                for _ in 0..*limited_count {
                    prometheus_metrics.incr_limited_calls(namespace)
                }
            });

        let metrics_output = prometheus_metrics.gather_metrics();

        namespaces_with_limited_counts
            .iter()
            .for_each(|(namespace, limited_count)| {
                assert!(metrics_output.contains(&formatted_counter(
                    &LIMITED_CALLS.name,
                    *limited_count,
                    namespace
                )));
            });
    }

    #[test]
    fn shows_limitador_up_set_to_1() {
        let metrics_output = PrometheusMetrics::new().gather_metrics();
        assert!(metrics_output.contains("limitador_up 1"))
    }

    fn formatted_counter(name: &str, count: i32, namespace: &Namespace) -> String {
        format!(
            "{}{{limitador_namespace=\"{}\"}} {}",
            name,
            namespace.as_ref(),
            count,
        )
    }
}
