use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::Arc;

use limitador::limit::Namespace;

const NAMESPACE_LABEL: &str = "limitador_namespace";
const LIMIT_NAME_LABEL: &str = "limit_name";

pub struct PrometheusMetrics {
    prometheus_handle: Arc<PrometheusHandle>,
    use_limit_name_label: bool,
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

    pub fn new_with_options(use_limit_name_label: bool) -> Self {
        Self::new_with_handle(use_limit_name_label, Arc::new(Self::init_handle()))
    }

    pub(crate) fn new_with_handle(
        use_limit_name_label: bool,
        prometheus_handle: Arc<PrometheusHandle>,
    ) -> Self {
        describe_histogram!(
            "counter_latency",
            "Latency to the underlying counter datastore"
        );
        describe_counter!("authorized_calls", "Authorized calls");
        describe_counter!("limited_calls", "Limited calls");
        describe_gauge!("limitador_up", "Limitador is running");
        gauge!("limitador_up").set(1);
        describe_gauge!(
            "datastore_partitioned",
            "Limitador is partitioned from backing datastore"
        );
        gauge!("datastore_partitioned").set(0);
        Self {
            use_limit_name_label,
            prometheus_handle,
        }
    }

    // Creates and installs the prometheus exporter as global recorder
    // Only one recorder can be registered for the lifetime of the application
    fn init_handle() -> PrometheusHandle {
        let prom_builder = PrometheusBuilder::new();
        prom_builder
            .install_recorder()
            .expect("failed to create prometheus metrics exporter")
    }

    pub fn incr_authorized_calls(&self, namespace: &Namespace) {
        counter!("authorized_calls", NAMESPACE_LABEL => namespace.as_ref().to_string()).increment(1)
    }

    pub fn incr_limited_calls<'a, LN>(&self, namespace: &Namespace, limit_name: LN)
    where
        LN: Into<Option<&'a str>>,
    {
        let mut labels = vec![(NAMESPACE_LABEL, namespace.as_ref().to_string())];

        if self.use_limit_name_label {
            // If we have configured the metric to accept 2 labels we need to
            // set values for them.
            labels.push((
                LIMIT_NAME_LABEL,
                limit_name.into().unwrap_or("").to_string(),
            ));
        }
        counter!("limited_calls", &labels).increment(1)
    }

    pub fn gather_metrics(&self) -> String {
        self.prometheus_handle.render()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use lazy_static::lazy_static;
    use metrics_exporter_prometheus::PrometheusHandle;

    // Setting recorder once for all test cases
    lazy_static! {
        pub static ref TEST_PROMETHEUS_HANDLE: Arc<PrometheusHandle> =
            Arc::new(PrometheusMetrics::init_handle());
    }

    #[test]
    fn shows_authorized_calls_by_namespace() {
        let prometheus_metrics =
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone());

        let namespaces_with_auth_counts = [
            ("auth_calls_by_namespace".into(), 2),
            ("auth_calls_by_namespace_two".into(), 3),
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
                    "authorized_calls",
                    *auth_count,
                    namespace
                )));
            });
    }

    #[test]
    fn shows_limited_calls_by_namespace() {
        let prometheus_metrics =
            PrometheusMetrics::new_with_handle(false, TEST_PROMETHEUS_HANDLE.clone());

        let namespaces_with_limited_counts = [
            ("limited_calls_by_namespace".into(), 2),
            ("limited_calls_by_namespace_two".into(), 3),
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
                    "limited_calls",
                    *limited_count,
                    namespace
                )));
            });
    }

    #[test]
    fn can_show_limited_calls_by_limit_name() {
        let prometheus_metrics =
            PrometheusMetrics::new_with_handle(true, TEST_PROMETHEUS_HANDLE.clone());

        let limits_with_counts = [
            ("limited_calls_by_limit_name".into(), "Some limit", 2),
            ("limited_calls_by_limit_name".into(), "Another limit", 3),
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
                        "limited_calls",
                        *limited_count,
                        namespace,
                        limit_name,
                    ))
                );
            });
    }

    #[test]
    fn incr_limited_calls_uses_empty_string_when_no_name() {
        let prometheus_metrics =
            PrometheusMetrics::new_with_handle(true, TEST_PROMETHEUS_HANDLE.clone());
        let namespace = "limited_calls_empty_name".into();
        prometheus_metrics.incr_limited_calls(&namespace, None);

        let metrics_output = prometheus_metrics.gather_metrics();

        assert!(
            metrics_output.contains(&formatted_counter_with_namespace_and_limit(
                "limited_calls",
                1,
                &namespace,
                "",
            ))
        );
    }

    #[test]
    fn shows_limitador_up_set_to_1() {
        let metrics_output =
            PrometheusMetrics::new_with_handle(true, TEST_PROMETHEUS_HANDLE.clone())
                .gather_metrics();
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
            "{}{{limitador_namespace=\"{}\",limit_name=\"{}\"}} {}",
            metric_name,
            namespace.as_ref(),
            limit_name,
            count,
        )
    }
}
