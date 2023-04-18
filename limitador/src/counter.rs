use crate::limit::{Limit, Namespace};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::time::Duration;

#[derive(Eq, Clone, Debug, Serialize, Deserialize)]
pub struct Counter {
    limit: Limit,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    #[serde(serialize_with = "ordered_map")]
    set_variables: HashMap<String, String>,

    remaining: Option<i64>,
    expires_in: Option<Duration>,
}

fn ordered_map<S>(value: &HashMap<String, String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

impl Counter {
    pub fn new(limit: Limit, set_variables: HashMap<String, String>) -> Self {
        // TODO: check that all the variables defined in the limit are set.

        let mut vars = set_variables;
        vars.retain(|var, _| limit.has_variable(var));

        Self {
            limit,
            set_variables: vars,
            remaining: None,
            expires_in: None,
        }
    }

    pub fn limit(&self) -> &Limit {
        &self.limit
    }

    pub fn max_value(&self) -> i64 {
        self.limit.max_value()
    }

    pub fn update_to_limit(&mut self, limit: &Limit) -> bool {
        if limit == &self.limit {
            self.limit.set_max_value(limit.max_value());
            if let Some(name) = limit.name() {
                self.limit.set_name(name.to_string());
            }
            return true;
        }
        false
    }

    pub fn seconds(&self) -> u64 {
        self.limit.seconds()
    }

    pub fn namespace(&self) -> &Namespace {
        self.limit.namespace()
    }

    pub fn set_variables(&self) -> &HashMap<String, String> {
        &self.set_variables
    }

    pub fn remaining(&self) -> Option<i64> {
        self.remaining
    }

    pub fn set_remaining(&mut self, remaining: i64) {
        self.remaining = Some(remaining)
    }

    pub fn expires_in(&self) -> Option<Duration> {
        self.expires_in
    }

    pub fn set_expires_in(&mut self, duration: Duration) {
        self.expires_in = Some(duration)
    }

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

        let mut encoded_vars = self
            .set_variables
            .iter()
            .map(|(k, v)| k.to_owned() + ":" + v)
            .collect::<Vec<String>>();

        encoded_vars.sort();
        encoded_vars.hash(state);
    }
}

impl PartialEq for Counter {
    fn eq(&self, other: &Self) -> bool {
        self.limit == other.limit && self.set_variables == other.set_variables
    }
}
