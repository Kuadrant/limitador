use crate::limit::Namespace;
use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};
use std::sync::Once;

const NAMESPACE_LABEL: &str = "limitador_namespace";

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();

    static ref AUTHORIZED_CALLS: IntCounterVec = IntCounterVec::new(
        Opts::new("authorized_calls", "Authorized calls"),
        &[NAMESPACE_LABEL]
    )
    .unwrap();

    static ref LIMITED_CALLS: IntCounterVec = IntCounterVec::new(
        Opts::new("limited_calls", "Limited calls"),
        &[NAMESPACE_LABEL],
    )
    .unwrap();

    // This can be used as a simple health check
    static ref LIMITADOR_UP: IntGauge =
        IntGauge::new("limitador_up", "Limitador is running").unwrap();
}

static REGISTER_METRICS: Once = Once::new();

pub fn register_metrics() {
    REGISTER_METRICS.call_once(|| {
        REGISTRY
            .register(Box::new(AUTHORIZED_CALLS.clone()))
            .unwrap();

        REGISTRY.register(Box::new(LIMITED_CALLS.clone())).unwrap();
        REGISTRY.register(Box::new(LIMITADOR_UP.clone())).unwrap();

        LIMITADOR_UP.set(1);
    })
}

pub fn incr_authorized_calls(namespace: &Namespace) {
    AUTHORIZED_CALLS
        .with_label_values(&[namespace.as_ref()])
        .inc();
}

pub fn incr_limited_calls(namespace: &Namespace) {
    LIMITED_CALLS.with_label_values(&[namespace.as_ref()]).inc();
}

pub fn gather_metrics() -> String {
    let mut buffer = Vec::new();

    TextEncoder::new()
        .encode(&REGISTRY.gather(), &mut buffer)
        .unwrap();

    String::from_utf8(buffer).unwrap()
}
