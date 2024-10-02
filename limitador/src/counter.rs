use crate::limit::{Limit, Namespace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

#[derive(Eq, Clone, Debug, Serialize, Deserialize)]
pub struct Counter {
    limit: Arc<Limit>,

    set_variables: BTreeMap<String, String>,

    remaining: Option<u64>,
    expires_in: Option<Duration>,
}

impl Counter {
    pub fn new<L: Into<Arc<Limit>>>(limit: L, set_variables: HashMap<String, String>) -> Self {
        // TODO: check that all the variables defined in the limit are set.

        let limit = limit.into();
        let mut vars = set_variables;
        vars.retain(|var, _| limit.has_variable(var));

        Self {
            limit,
            set_variables: vars.into_iter().collect(),
            remaining: None,
            expires_in: None,
        }
    }

    #[cfg(any(feature = "redis_storage", feature = "disk_storage"))]
    pub(crate) fn key(&self) -> Self {
        Self {
            limit: Arc::clone(&self.limit),
            set_variables: self.set_variables.clone(),
            remaining: None,
            expires_in: None,
        }
    }

    pub fn limit(&self) -> &Limit {
        &self.limit
    }

    pub fn max_value(&self) -> u64 {
        self.limit.max_value()
    }

    pub fn update_to_limit(&mut self, limit: Arc<Limit>) -> bool {
        if limit == self.limit {
            self.limit = limit;
            return true;
        }
        false
    }

    pub fn window(&self) -> Duration {
        Duration::from_secs(self.limit.seconds())
    }

    pub fn id(&self) -> Option<&str> {
        self.limit.id()
    }

    pub fn namespace(&self) -> &Namespace {
        self.limit.namespace()
    }

    pub fn set_variables(&self) -> &BTreeMap<String, String> {
        &self.set_variables
    }

    pub fn remaining(&self) -> Option<u64> {
        self.remaining
    }

    pub fn set_remaining(&mut self, remaining: u64) {
        self.remaining = Some(remaining)
    }

    pub fn expires_in(&self) -> Option<Duration> {
        self.expires_in
    }

    pub fn set_expires_in(&mut self, duration: Duration) {
        self.expires_in = Some(duration)
    }

    pub fn is_qualified(&self) -> bool {
        !self.set_variables.is_empty()
    }

    #[cfg(feature = "disk_storage")]
    pub(crate) fn variables_for_key(&self) -> Vec<(&str, &str)> {
        let mut variables = Vec::with_capacity(self.set_variables.len());
        for (var, value) in &self.set_variables {
            variables.push((var.as_str(), value.as_str()));
        }
        variables.sort_by(|(key1, _), (key2, _)| key1.cmp(key2));
        variables
    }
}

impl Hash for Counter {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.limit.hash(state);

        self.set_variables.iter().for_each(|(k, v)| {
            k.hash(state);
            v.hash(state);
        });
    }
}

impl PartialEq for Counter {
    fn eq(&self, other: &Self) -> bool {
        self.limit == other.limit && self.set_variables == other.set_variables
    }
}
