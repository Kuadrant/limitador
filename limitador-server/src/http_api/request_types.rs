use limitador::counter::Counter as LimitadorCounter;
use limitador::errors::LimitadorError;
use limitador::limit::Limit as LimitadorLimit;
use paperclip::actix::Apiv2Schema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
// We need to define the Limit and Counter types. They're basically the same as
// defined in the lib but with some modifications to be able to derive
// Apiv2Schema (needed to generate the OpenAPI specs).

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Apiv2Schema)]
pub struct CheckAndReportInfo {
    pub namespace: String,
    pub values: HashMap<String, String>,
    pub delta: u64,
    pub response_headers: Option<String>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Apiv2Schema)]
pub struct Limit {
    id: Option<String>,
    namespace: String,
    max_value: u64,
    seconds: u64,
    name: Option<String>,
    conditions: Vec<String>,
    variables: Vec<String>,
}

impl From<&LimitadorLimit> for Limit {
    fn from(ll: &LimitadorLimit) -> Self {
        Self {
            id: ll.id().map(|id| id.to_string()),
            namespace: ll.namespace().as_ref().to_string(),
            max_value: ll.max_value(),
            seconds: ll.seconds(),
            name: ll.name().map(|name| name.to_string()),
            conditions: ll.conditions().into_iter().collect(),
            variables: ll.variables().into_iter().collect(),
        }
    }
}

impl TryFrom<Limit> for LimitadorLimit {
    type Error = LimitadorError;

    fn try_from(limit: Limit) -> Result<Self, Self::Error> {
        let mut limitador_limit = if let Some(id) = limit.id {
            Self::with_id(
                id,
                limit.namespace,
                limit.max_value,
                limit.seconds,
                limit.conditions,
                limit.variables,
            )?
        } else {
            Self::new(
                limit.namespace,
                limit.max_value,
                limit.seconds,
                limit.conditions,
                limit.variables,
            )?
        };

        if let Some(name) = limit.name {
            limitador_limit.set_name(name)
        }

        Ok(limitador_limit)
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Apiv2Schema)]
pub struct Counter {
    limit: Limit,
    set_variables: BTreeMap<String, String>,
    remaining: Option<u64>,
    expires_in_seconds: Option<u64>,
}

impl From<&LimitadorCounter> for Counter {
    fn from(lc: &LimitadorCounter) -> Self {
        Self {
            limit: lc.limit().into(),
            set_variables: lc.set_variables().clone(),
            remaining: lc.remaining(),
            expires_in_seconds: lc.expires_in().map(|duration| duration.as_secs()),
        }
    }
}
