use crate::metrics::Timings;
use limitador::limit::{Context, Expression, Namespace};
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::collections::HashMap;
use std::string::ToString;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const NAMESPACE_LABEL: &str = "limitador_namespace";
const LIMIT_NAME_LABEL: &str = "limit_name";

pub struct PrometheusMetrics {
    prometheus_handle: Arc<PrometheusHandle>,
    use_limit_name_label: bool,
    custom_labels: RwLock<HashMap<String, Expression>>,
    default_labels: Option<Expression>,
}

impl Default for PrometheusMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        Self::new_with_options(false, None)
    }

    // Note: This is optional because for a small number of limits it should be
    // fine, but it could become a problem when defining lots of limits. See the
    // caution note in the Prometheus docs:
    // https://prometheus.io/docs/practices/naming/#labels
    pub fn new_with_counters_by_limit_name() -> Self {
        Self::new_with_options(true, None)
    }

    pub fn new_with_options(
        use_limit_name_label: bool,
        default_labels: Option<Expression>,
    ) -> Self {
        Self::new_with_handle_and_default_labels(
            use_limit_name_label,
            Arc::new(Self::init_handle()),
            default_labels,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_handle(
        use_limit_name_label: bool,
        prometheus_handle: Arc<PrometheusHandle>,
    ) -> Self {
        Self::new_with_handle_and_default_labels(use_limit_name_label, prometheus_handle, None)
    }

    pub(crate) fn new_with_handle_and_default_labels(
        use_limit_name_label: bool,
        prometheus_handle: Arc<PrometheusHandle>,
        default_labels: Option<Expression>,
    ) -> Self {
        describe_histogram!(
            "datastore_latency",
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
            custom_labels: RwLock::default(),
            default_labels,
        }
    }

    // Creates and installs the prometheus exporter as global recorder
    // Only one recorder can be registered for the lifetime of the application
    pub fn init_handle() -> PrometheusHandle {
        let prom_builder = PrometheusBuilder::new();
        prom_builder
            .install_recorder()
            .expect("failed to create prometheus metrics exporter")
    }

    pub fn incr_authorized_calls(&self, namespace: &Namespace, cel_ctx: &Context) {
        let mut labels: Vec<(String, String)> = self.labels(cel_ctx);
        labels.push((NAMESPACE_LABEL.to_string(), namespace.as_ref().to_string()));
        counter!("authorized_calls", &labels).increment(1);
    }

    pub fn incr_authorized_hits(&self, namespace: &Namespace, cel_ctx: &Context, hits_addend: u64) {
        let mut labels: Vec<(String, String)> = self.labels(cel_ctx);
        labels.push((NAMESPACE_LABEL.to_string(), namespace.as_ref().to_string()));
        counter!("authorized_hits", &labels).increment(hits_addend);
    }

    pub fn incr_limited_calls<'a, LN>(
        &self,
        namespace: &Namespace,
        limit_name: LN,
        cel_ctx: &Context,
    ) where
        LN: Into<Option<&'a str>>,
    {
        let mut labels: Vec<(String, String)> = self.labels(cel_ctx);
        labels.push((NAMESPACE_LABEL.to_string(), namespace.as_ref().to_string()));

        if self.use_limit_name_label {
            // If we have configured the metric to accept 2 labels we need to
            // set values for them.
            labels.push((
                LIMIT_NAME_LABEL.to_string(),
                limit_name.into().unwrap_or("").to_string(),
            ));
        }
        counter!("limited_calls", &labels).increment(1)
    }

    pub fn gather_metrics(&self) -> String {
        self.prometheus_handle.render()
    }

    pub fn record_datastore_latency(timings: Timings) {
        histogram!("datastore_latency").record(Duration::from(timings).as_secs_f64())
    }

    pub fn set_custom_labels(&self, new_labels: HashMap<String, Expression>) -> Result<(), String> {
        match self.custom_labels.write() {
            Ok(mut custom_labels) => {
                *custom_labels = new_labels;
                Ok(())
            }
            Err(err) => Err(err.to_string()),
        }
    }

    fn labels(&self, ctx: &Context) -> Vec<(String, String)> {
        let mut labels = if let Ok(custom_labels) = self.custom_labels.read() {
            custom_labels
                .iter()
                .filter_map(|(label, exp)| {
                    if let Ok(Some(val)) = exp.eval(ctx) {
                        return Some((label.to_string(), val));
                    }
                    None
                })
                .collect()
        } else {
            Vec::default()
        };
        if let Some(default) = &self.default_labels {
            if let Ok(value) = default.eval_map(ctx) {
                let mut default_labels: Vec<(String, String)> = value.into_iter().collect();
                default_labels.append(&mut labels);
                return default_labels;
            }
        }
        labels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limitador::limit::Context;
    use metrics::with_local_recorder;
    use metrics_exporter_prometheus::PrometheusHandle;

    #[test]
    fn shows_authorized_calls_by_namespace() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(false, handle.clone());
            let namespaces_with_auth_counts = [
                ("auth_calls_by_namespace".into(), 2),
                ("auth_calls_by_namespace_two".into(), 3),
            ];

            namespaces_with_auth_counts
                .iter()
                .for_each(|(namespace, auth_count)| {
                    for _ in 0..*auth_count {
                        prometheus_metrics.incr_authorized_calls(namespace, &Context::default());
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
        });
    }

    #[test]
    fn shows_authorized_hits_by_namespace() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(false, handle.clone());
            let namespaces_with_auth_counts = [
                ("auth_calls_by_namespace".into(), 2, 7u64),
                ("auth_calls_by_namespace_two".into(), 3, 8u64),
            ];

            namespaces_with_auth_counts
                .iter()
                .for_each(|(namespace, auth_count, hits_addend)| {
                    for _ in 0..*auth_count {
                        prometheus_metrics.incr_authorized_hits(
                            namespace,
                            &Context::default(),
                            *hits_addend,
                        );
                    }
                });

            let metrics_output = prometheus_metrics.gather_metrics();

            namespaces_with_auth_counts
                .iter()
                .for_each(|(namespace, auth_count, hits_addend)| {
                    assert!(metrics_output.contains(&formatted_counter_with_namespace(
                        "authorized_hits",
                        (*auth_count) * (*hits_addend as i32),
                        namespace
                    )));
                });
        });
    }

    #[test]
    fn shows_limited_calls_by_namespace() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(false, handle.clone());
            let namespaces_with_limited_counts = [
                ("limited_calls_by_namespace".into(), 2),
                ("limited_calls_by_namespace_two".into(), 3),
            ];

            namespaces_with_limited_counts
                .iter()
                .for_each(|(namespace, limited_count)| {
                    for _ in 0..*limited_count {
                        prometheus_metrics.incr_limited_calls(namespace, None, &Context::default())
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
        });
    }

    #[test]
    fn can_show_limited_calls_by_limit_name() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(true, handle.clone());
            let limits_with_counts = [
                ("limited_calls_by_limit_name".into(), "Some limit", 2),
                ("limited_calls_by_limit_name".into(), "Another limit", 3),
            ];

            limits_with_counts
                .iter()
                .for_each(|(namespace, limit_name, limited_count)| {
                    for _ in 0..*limited_count {
                        prometheus_metrics.incr_limited_calls(
                            namespace,
                            *limit_name,
                            &Context::default(),
                        )
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
        });
    }

    #[test]
    fn incr_limited_calls_uses_empty_string_when_no_name() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(true, handle.clone());
            let namespace = "limited_calls_empty_name".into();
            prometheus_metrics.incr_limited_calls(&namespace, None, &Context::default());

            let metrics_output = prometheus_metrics.gather_metrics();

            assert!(
                metrics_output.contains(&formatted_counter_with_namespace_and_limit(
                    "limited_calls",
                    1,
                    &namespace,
                    "",
                ))
            );
        });
    }

    #[test]
    fn incr_limited_calls_uses_custom_labels() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(true, handle.clone());
            let namespace = "limited_calls_empty_name".into();
            let mut ctx = Context::default();
            let values = HashMap::from([("foobar".to_string(), "1".to_string())]);
            ctx.list_binding("descriptors".to_string(), vec![HashMap::default(), values]);
            prometheus_metrics.incr_limited_calls(&namespace, None, &ctx);

            prometheus_metrics
                .set_custom_labels(HashMap::default())
                .expect("Failed to set custom labels");
            let metrics_output = prometheus_metrics.gather_metrics();
            assert!(!metrics_output.contains("myLabel=\"user 1\""));
            prometheus_metrics
                .set_custom_labels(HashMap::from([(
                    "myLabel".to_string(),
                    Expression::parse("'user ' + descriptors[1].foobar")
                        .expect("Invalid expression!"),
                )]))
                .expect("Failed to set custom labels");
            prometheus_metrics.incr_limited_calls(&namespace, None, &ctx);
            let metrics_output = prometheus_metrics.gather_metrics();
            assert!(metrics_output.contains("myLabel=\"user 1\""));
        });
    }

    #[test]
    fn incr_limited_calls_uses_default_labels() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle_and_default_labels(
                true,
                handle.clone(),
                Some(Expression::parse("descriptors[0]").expect("Invalid expression!")),
            );
            let namespace = "limited_calls_empty_name".into();
            let mut ctx = Context::default();
            let values = HashMap::from([("foobar".to_string(), "1".to_string())]);
            ctx.list_binding("descriptors".to_string(), vec![values]);
            prometheus_metrics.incr_limited_calls(&namespace, None, &ctx);
            let metrics_output = prometheus_metrics.gather_metrics();
            assert!(
                metrics_output.contains("foobar=\"1\""),
                "{}",
                metrics_output
            );
        });
    }

    #[test]
    fn shows_limitador_up_set_to_1() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle: Arc<PrometheusHandle> = recorder.handle().into();

        with_local_recorder(&recorder, || {
            let prometheus_metrics = PrometheusMetrics::new_with_handle(true, handle.clone());
            let metrics_output = prometheus_metrics.gather_metrics();
            assert!(metrics_output.contains("limitador_up 1"))
        });
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
