use crate::limit::Namespace;
use prometheus::{Encoder, IntCounterVec, Opts, Registry, TextEncoder};
use std::sync::Once;

lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
    static ref AUTHORIZED_CALLS: IntCounterVec = IntCounterVec::new(
        Opts::new("authorized_calls", "Authorized calls"),
        &["namespace"]
    )
    .unwrap();
    static ref LIMITED_CALLS: IntCounterVec =
        IntCounterVec::new(Opts::new("limited_calls", "Limited calls"), &["namespace"]).unwrap();
}

static REGISTER_METRICS: Once = Once::new();

pub fn register_metrics() {
    REGISTER_METRICS.call_once(|| {
        REGISTRY
            .register(Box::new(AUTHORIZED_CALLS.clone()))
            .unwrap();

        REGISTRY.register(Box::new(LIMITED_CALLS.clone())).unwrap();
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
