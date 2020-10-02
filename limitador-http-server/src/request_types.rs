use limitador::counter::Counter as LimitadorCounter;
use limitador::limit::Limit as LimitadorLimit;
use paperclip::actix::Apiv2Schema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// We need to define the Limit and Counter types. They're basically the same as
// defined in the lib but with some modifications to be able to derive
// Apiv2Schema (needed to generate the OpenAPI specs).

#[derive(Deserialize, Apiv2Schema)]
pub struct CheckAndReportInfo {
    pub namespace: String,
    pub values: HashMap<String, String>,
    pub delta: i64,
}

#[derive(Serialize, Deserialize, Apiv2Schema)]
pub struct Limit {
    namespace: String,
    max_value: i64,
    seconds: u64,
    conditions: Vec<String>,
    variables: Vec<String>,
}

impl From<&LimitadorLimit> for Limit {
    fn from(ll: &LimitadorLimit) -> Self {
        Self {
            namespace: ll.namespace().as_ref().to_string(),
            max_value: ll.max_value(),
            seconds: ll.seconds(),
            conditions: ll.conditions().into_iter().collect(),
            variables: ll.variables().into_iter().collect(),
        }
    }
}

impl Into<LimitadorLimit> for Limit {
    fn into(self) -> LimitadorLimit {
        LimitadorLimit::new(
            self.namespace.as_str(),
            self.max_value,
            self.seconds,
            self.conditions,
            self.variables,
        )
    }
}

#[derive(Serialize, Apiv2Schema)]
pub struct Counter {
    limit: Limit,
    set_variables: HashMap<String, String>,
    remaining: Option<i64>,
    expires_in_seconds: Option<u64>,
}

impl From<&LimitadorCounter> for Counter {
    fn from(lc: &LimitadorCounter) -> Self {
        Self {
            limit: lc.limit().into(),
            set_variables: lc.set_variables().clone(),
            remaining: lc.remaining(),
            expires_in_seconds: lc.expires_in(),
        }
    }
}
