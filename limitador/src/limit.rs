use cel::Context;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

mod cel;

pub use cel::{EvaluationError, ParseError};
pub use cel::{Expression, Predicate};

#[derive(Debug, Hash, Eq, PartialEq, Clone, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Namespace(String);

impl From<&str> for Namespace {
    fn from(s: &str) -> Namespace {
        Self(s.into())
    }
}

impl AsRef<str> for Namespace {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<String> for Namespace {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Eq, Debug, Clone, Serialize, Deserialize)]
pub struct Limit {
    #[serde(skip_serializing, default)]
    id: Option<String>,
    namespace: Namespace,
    #[serde(skip_serializing, default)]
    max_value: u64,
    seconds: u64,
    #[serde(skip_serializing, default)]
    name: Option<String>,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    conditions: BTreeSet<Predicate>,
    variables: BTreeSet<String>,
}

impl Limit {
    pub fn new<N: Into<Namespace>, T: TryInto<Predicate>>(
        namespace: N,
        max_value: u64,
        seconds: u64,
        conditions: impl IntoIterator<Item = T>,
        variables: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, ParseError>
    where
        <N as TryInto<Namespace>>::Error: core::fmt::Debug,
        <T as TryInto<Predicate>>::Error: core::fmt::Debug,
        ParseError: From<<T as TryInto<Predicate>>::Error>,
    {
        // the above where-clause is needed in order to call unwrap().
        let conditions: Result<BTreeSet<_>, _> =
            conditions.into_iter().map(|cond| cond.try_into()).collect();
        match conditions {
            Ok(conditions) => Ok(Self {
                id: None,
                namespace: namespace.into(),
                max_value,
                seconds,
                name: None,
                conditions,
                variables: variables.into_iter().map(|var| var.into()).collect(),
            }),
            Err(err) => Err(err.into()),
        }
    }

    pub fn with_id<S: Into<String>, N: Into<Namespace>, T: TryInto<Predicate>>(
        id: S,
        namespace: N,
        max_value: u64,
        seconds: u64,
        conditions: impl IntoIterator<Item = T>,
        variables: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, ParseError>
    where
        ParseError: From<<T as TryInto<Predicate>>::Error>,
    {
        match conditions.into_iter().map(|cond| cond.try_into()).collect() {
            Ok(conditions) => Ok(Self {
                id: Some(id.into()),
                namespace: namespace.into(),
                max_value,
                seconds,
                name: None,
                conditions,
                variables: variables.into_iter().map(|var| var.into()).collect(),
            }),
            Err(err) => Err(err.into()),
        }
    }

    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn max_value(&self) -> u64 {
        self.max_value
    }

    pub fn seconds(&self) -> u64 {
        self.seconds
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = Some(name)
    }

    pub fn set_max_value(&mut self, value: u64) {
        self.max_value = value;
    }

    pub fn conditions(&self) -> HashSet<String> {
        self.conditions
            .iter()
            .map(|cond| cond.clone().into())
            .collect()
    }

    pub fn variables(&self) -> HashSet<String> {
        self.variables.iter().map(|var| var.into()).collect()
    }

    #[cfg(feature = "disk_storage")]
    pub(crate) fn variables_for_key(&self) -> Vec<&str> {
        let mut variables = Vec::with_capacity(self.variables.len());
        for var in &self.variables {
            variables.push(var.as_str());
        }
        variables.sort();
        variables
    }

    pub fn has_variable(&self, var: &str) -> bool {
        self.variables.contains(var)
    }

    pub fn applies(&self, values: &HashMap<String, String>) -> bool {
        let ctx = Context::new(self, String::default(), values);
        let all_conditions_apply = self
            .conditions
            .iter()
            .all(|predicate| predicate.test(&ctx).unwrap());

        let all_vars_are_set = self.variables.iter().all(|var| values.contains_key(var));

        all_conditions_apply && all_vars_are_set
    }
}

impl Hash for Limit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.namespace.hash(state);
        self.seconds.hash(state);
        self.conditions.iter().for_each(|e| e.hash(state));
        self.variables.iter().for_each(|e| e.hash(state));
    }
}

impl PartialOrd for Limit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Limit {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.namespace.cmp(&other.namespace) {
            Ordering::Equal => match self.seconds.cmp(&other.seconds) {
                Ordering::Equal => match self.conditions.cmp(&other.conditions) {
                    Ordering::Equal => self.variables.cmp(&other.variables),
                    cmp => cmp,
                },
                cmp => cmp,
            },
            cmp => cmp,
        }
    }
}

impl PartialEq for Limit {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.seconds == other.seconds
            && self.conditions == other.conditions
            && self.variables == other.variables
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::Equal;

    #[test]
    fn limit_can_have_an_optional_name() {
        let mut limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"])
            .expect("This must be a valid limit!");
        assert!(limit.name.is_none());

        let name = "Test Limit";
        limit.set_name(name.to_string());
        assert_eq!(name, limit.name.unwrap())
    }

    #[test]
    fn limit_applies() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"])
            .expect("This must be a valid limit!");

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "1".into());

        assert!(limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_is_false() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"])
            .expect("This must be a valid limit!");

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_var_is_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"])
            .expect("This must be a valid limit!");

        // Notice that "x" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("a".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_var_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"])
            .expect("This must be a valid limit!");

        // Notice that "y" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_applies_when_all_its_conditions_apply() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"", "y == \"2\""],
            vec!["z"],
        )
        .expect("This must be a valid limit!");

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
            vec!["x == \"5\"", "y == \"2\""],
            vec!["z"],
        )
        .expect("This must be a valid limit!");

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "3".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_id() {
        let limit = Limit::with_id(
            "test_id",
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'"],
            vec!["app_id"],
        )
        .expect("This must be a valid limit!");

        assert_eq!(limit.id(), Some("test_id"))
    }

    #[test]
    fn partial_equality() {
        let limit1 = Limit::with_id(
            "test_id",
            "test_namespace",
            42,
            60,
            vec!["req_method == 'GET'"],
            vec!["app_id"],
        )
        .expect("This must be a valid limit!");

        let mut limit2 = Limit::new(
            limit1.namespace.clone(),
            limit1.max_value + 10,
            limit1.seconds,
            vec!["req_method == 'GET'"],
            limit1.variables.clone(),
        )
        .expect("This must be a valid limit!");
        limit2.set_name("Who cares?".to_string());

        assert_eq!(limit1.partial_cmp(&limit2), Some(Equal));
        assert_eq!(limit1, limit2);
    }

    #[test]
    fn conditions_have_limit_info() {
        let mut limit = Limit::new(
            "ns",
            42,
            10,
            vec!["limit.name == 'named_limit'"],
            Vec::<String>::default(),
        )
        .expect("failed to create");
        assert!(!limit.applies(&HashMap::default()));

        limit.set_name("named_limit".to_string());
        assert!(limit.applies(&HashMap::default()));

        let limit = Limit::with_id(
            "my_id",
            "ns",
            42,
            10,
            vec!["limit.id == 'my_id'", "limit.name == null"],
            Vec::<String>::default(),
        )
        .expect("failed to create");
        assert!(limit.applies(&HashMap::default()));

        let limit = Limit::with_id(
            "my_id",
            "ns",
            42,
            10,
            vec!["limit.id == 'other_id'"],
            Vec::<String>::default(),
        )
        .expect("failed to create");
        assert!(!limit.applies(&HashMap::default()));
    }
}
