use crate::limit::Limit;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

#[derive(Eq, Clone, Debug, Serialize, Deserialize)]
pub struct Counter {
    limit: Limit,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    #[serde(serialize_with = "ordered_map")]
    set_variables: HashMap<String, String>,
}

fn ordered_map<S>(value: &HashMap<String, String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

impl Counter {
    pub fn new(limit: Limit, set_variables: HashMap<String, String>) -> Counter {
        // TODO: check that all the variables defined in the limit are set.

        let mut vars = set_variables;
        vars.retain(|var, _| limit.has_variable(var));

        Counter {
            limit,
            set_variables: vars,
        }
    }

    pub fn max_value(&self) -> i64 {
        self.limit.max_value()
    }

    pub fn seconds(&self) -> u64 {
        self.limit.seconds()
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
