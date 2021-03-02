use crate::limit::Namespace;
use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};

const NAMESPACE_LABEL: &str = "limitador_namespace";

pub struct PrometheusMetrics {
    registry: Registry,
    authorized_calls: IntCounterVec,
    limited_calls: IntCounterVec,
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        let labels = vec![NAMESPACE_LABEL];

        let authorized_calls =
            IntCounterVec::new(Opts::new("authorized_calls", "Authorized calls"), &labels).unwrap();

        let limited_calls =
            IntCounterVec::new(Opts::new("limited_calls", "Limited calls"), &labels).unwrap();

        // Can be used as a simple health check
        let limitador_up = IntGauge::new("limitador_up", "Limitador is running").unwrap();

        let registry = Registry::new();

        registry
            .register(Box::new(authorized_calls.clone()))
            .unwrap();

        registry.register(Box::new(limited_calls.clone())).unwrap();
        registry.register(Box::new(limitador_up.clone())).unwrap();

        limitador_up.set(1);

        Self {
            registry,
            authorized_calls,
            limited_calls,
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
}
