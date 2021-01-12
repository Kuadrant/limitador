use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Error;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct Namespace(String);

impl FromStr for Namespace {
    type Err = Error;

    fn from_str(s: &str) -> Result<Namespace, Self::Err> {
        Ok(Namespace(s.into()))
    }
}

impl From<&str> for Namespace {
    fn from(s: &str) -> Namespace {
        s.parse().unwrap()
    }
}

impl AsRef<str> for Namespace {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<String> for Namespace {
    fn from(s: String) -> Namespace {
        Namespace(s)
    }
}

#[derive(Eq, Debug, Clone, Serialize, Deserialize)]
pub struct Limit {
    namespace: Namespace,
    max_value: i64,
    seconds: u64,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    #[serde(serialize_with = "ordered_set")]
    conditions: HashSet<String>,
    #[serde(serialize_with = "ordered_set")]
    variables: HashSet<String>,
}

fn ordered_set<S>(value: &HashSet<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeSet<_> = value.iter().collect();
    ordered.serialize(serializer)
}

impl Limit {
    pub fn new(
        namespace: impl Into<Namespace>,
        max_value: i64,
        seconds: u64,
        conditions: impl IntoIterator<Item = impl Into<String>>,
        variables: impl IntoIterator<Item = impl Into<String>>,
    ) -> Limit {
        Limit {
            namespace: namespace.into(),
            max_value,
            seconds,
            conditions: conditions.into_iter().map(|cond| cond.into()).collect(),
            variables: variables.into_iter().map(|var| var.into()).collect(),
        }
    }

    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub fn max_value(&self) -> i64 {
        self.max_value
    }

    pub fn seconds(&self) -> u64 {
        self.seconds
    }

    pub fn conditions(&self) -> HashSet<String> {
        self.conditions.iter().map(|cond| cond.into()).collect()
    }

    pub fn variables(&self) -> HashSet<String> {
        self.variables.iter().map(|var| var.into()).collect()
    }

    pub fn has_variable(&self, var: &str) -> bool {
        self.variables.contains(var)
    }

    pub fn applies(&self, values: &HashMap<String, String>) -> bool {
        let all_conditions_apply = self
            .conditions
            .iter()
            .all(|cond| Self::condition_applies(&cond, values));

        let all_vars_are_set = self.variables.iter().all(|var| values.contains_key(var));

        all_conditions_apply && all_vars_are_set
    }

    fn condition_applies(condition: &str, values: &HashMap<String, String>) -> bool {
        // TODO: for now assume that all the conditions have this format:
        // "left_operand == right_operand"

        let split: Vec<&str> = condition.split(" == ").collect();
        let left_operand = split[0];
        let right_operand = split[1];

        match values.get(left_operand) {
            Some(val) => val == right_operand,
            None => false,
        }
    }
}

impl Hash for Limit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.namespace.hash(state);
        self.max_value.hash(state);
        self.seconds.hash(state);

        let mut encoded_conditions = self
            .conditions
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>();

        encoded_conditions.sort();
        encoded_conditions.hash(state);

        let mut encoded_vars = self
            .variables
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>();

        encoded_vars.sort();
        encoded_vars.hash(state);
    }
}

impl PartialEq for Limit {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.max_value == other.max_value
            && self.seconds == other.seconds
            && self.conditions == other.conditions
            && self.variables == other.variables
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_applies() {
        let limit = Limit::new(
            Namespace::from("test_namespace"),
            10,
            60,
            vec!["x == 5"],
            vec!["y"],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "1".into());

        assert!(limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_is_false() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert_eq!(false, limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_var_is_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

        // Notice that "x" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("a".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert_eq!(false, limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_var_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

        // Notice that "y" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());

        assert_eq!(false, limit.applies(&values))
    }

    #[test]
    fn limit_applies_when_all_its_conditions_apply() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == 5", "y == 2"],
            vec!["z"],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_if_one_cond_doesnt() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == 5", "y == 2"],
            vec!["z"],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "3".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert_eq!(false, limit.applies(&values))
    }
}
