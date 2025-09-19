use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

mod cel;

pub use cel::{Context, Expression, Predicate};
pub use cel::{EvaluationError, ParseError};

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
    variables: BTreeSet<Expression>,
}

impl Limit {
    pub fn new<N: Into<Namespace>>(
        namespace: N,
        max_value: u64,
        seconds: u64,
        conditions: impl IntoIterator<Item = Predicate>,
        variables: impl IntoIterator<Item = Expression>,
    ) -> Self
    where
        <N as TryInto<Namespace>>::Error: Debug,
    {
        Self {
            id: None,
            namespace: namespace.into(),
            max_value,
            seconds,
            name: None,
            conditions: conditions.into_iter().collect(),
            variables: variables.into_iter().collect(),
        }
    }

    pub fn with_id<S: Into<String>, N: Into<Namespace>>(
        id: S,
        namespace: N,
        max_value: u64,
        seconds: u64,
        conditions: impl IntoIterator<Item = Predicate>,
        variables: impl IntoIterator<Item = Expression>,
    ) -> Self {
        Self {
            id: Some(id.into()),
            namespace: namespace.into(),
            max_value,
            seconds,
            name: None,
            conditions: conditions.into_iter().collect(),
            variables: variables.into_iter().collect(),
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
        self.variables
            .iter()
            .map(|var| var.source().into())
            .collect()
    }

    pub fn resolve_variables(
        &self,
        ctx: &Context,
    ) -> Result<Option<BTreeMap<String, String>>, EvaluationError> {
        let mut map = BTreeMap::new();
        for variable in &self.variables {
            let name = variable.source().into();
            match variable.eval(ctx)? {
                None => return Ok(None),
                Some(value) => {
                    map.insert(name, value);
                }
            }
        }
        Ok(Some(map))
    }

    pub fn has_variable(&self, var: &str) -> bool {
        self.variables
            .iter()
            .flat_map(|v| v.variables())
            .any(|v| v.as_str() == var)
    }

    pub fn applies(&self, ctx: &Context) -> bool {
        let ctx = ctx.for_limit(self);
        let all_conditions_apply = self
            .conditions
            .iter()
            .all(|predicate| predicate.test(&ctx.for_limit(self)).unwrap());

        let all_vars_are_set = self.variables.iter().all(|var| {
            ctx.has_variables(
                &var.variables()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            )
        });

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
    use crate::counter::Counter;
    use std::cmp::Ordering::Equal;
    use std::collections::HashMap;

    #[test]
    fn limit_can_have_an_optional_name() {
        let mut limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"".try_into().expect("failed parsing!")],
            vec!["y".try_into().expect("failed parsing!")],
        );
        assert!(limit.name.is_none());

        let name = "Test Limit";
        limit.set_name(name.to_string());
        assert_eq!(name, limit.name.unwrap())
    }

    #[test]
    fn limit_applies() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"".try_into().expect("failed parsing!")],
            vec!["y".try_into().expect("failed parsing!")],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "1".into());

        assert!(limit.applies(&values.into()))
    }

    #[test]
    fn limit_does_not_apply_when_cond_is_false() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"".try_into().expect("failed parsing!")],
            vec!["y".try_into().expect("failed parsing!")],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values.into()))
    }

    #[test]
    fn limit_does_not_apply_when_cond_var_is_not_set() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"".try_into().expect("failed parsing!")],
            vec!["y".try_into().expect("failed parsing!")],
        );

        // Notice that "x" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("a".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values.into()))
    }

    #[test]
    fn limit_does_not_apply_when_var_not_set() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == \"5\"".try_into().expect("failed parsing!")],
            vec!["y".try_into().expect("failed parsing!")],
        );

        // Notice that "y" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());

        assert!(!limit.applies(&values.into()))
    }

    #[test]
    fn limit_applies_when_all_its_conditions_apply() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec![
                "x == \"5\"".try_into().expect("failed parsing!"),
                "y == \"2\"".try_into().expect("failed parsing!"),
            ],
            vec!["z".try_into().expect("failed parsing!")],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(limit.applies(&values.into()))
    }

    #[test]
    fn limit_does_not_apply_if_one_cond_doesnt() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec![
                "x == \"5\"".try_into().expect("failed parsing!"),
                "y == \"2\"".try_into().expect("failed parsing!"),
            ],
            vec!["z".try_into().expect("failed parsing!")],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "3".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(!limit.applies(&values.into()))
    }

    #[test]
    fn limit_id() {
        let limit = Limit::with_id(
            "test_id",
            "test_namespace",
            10,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        assert_eq!(limit.id(), Some("test_id"))
    }

    #[test]
    fn partial_equality() {
        let limit1 = Limit::with_id(
            "test_id",
            "test_namespace",
            42,
            60,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            vec!["app_id".try_into().expect("failed parsing!")],
        );

        let mut limit2 = Limit::new(
            limit1.namespace.clone(),
            limit1.max_value + 10,
            limit1.seconds,
            vec!["req_method == 'GET'".try_into().expect("failed parsing!")],
            limit1.variables.clone(),
        );
        limit2.set_name("Who cares?".to_string());

        assert_eq!(limit1.partial_cmp(&limit2), Some(Equal));
        assert_eq!(limit1, limit2);
    }

    #[test]
    fn resolves_variables() {
        let limit = Limit::new(
            "",
            10,
            60,
            Vec::default(),
            ["int(x) * 3".try_into().expect("failed parsing!")],
        );
        assert!(limit.has_variable("x"));
    }

    #[test]
    fn conditions_have_limit_info() {
        let mut limit = Limit::new(
            "ns",
            42,
            10,
            vec!["limit.name == 'named_limit'"
                .try_into()
                .expect("failed parsing!")],
            Vec::default(),
        );
        assert!(!limit.applies(&Context::default()));

        limit.set_name("named_limit".to_string());
        assert!(limit.applies(&Context::default()));

        let limit = Limit::with_id(
            "my_id",
            "ns",
            42,
            10,
            vec![
                "limit.id == 'my_id'".try_into().expect("failed parsing!"),
                "limit.name == null".try_into().expect("failed parsing!"),
            ],
            Vec::default(),
        );
        assert!(limit.applies(&Context::default()));

        let limit = Limit::with_id(
            "my_id",
            "ns",
            42,
            10,
            vec!["limit.id == 'other_id'"
                .try_into()
                .expect("failed parsing!")],
            Vec::default(),
        );
        assert!(!limit.applies(&Context::default()));
    }

    #[test]
    fn cel_limit_applies() {
        let limit = Limit::new(
            "ns",
            42,
            10,
            vec!["foo.contains('bar')".try_into().expect("failed parsing!")],
            vec!["bar.endsWith('baz')".try_into().expect("failed parsing!")],
        );
        let map = HashMap::from([
            ("foo".to_string(), "nice bar!".to_string()),
            ("bar".to_string(), "foo,baz".to_string()),
        ]);
        let ctx = map.into();
        assert!(limit.applies(&ctx));
        assert_eq!(
            Counter::new(limit, &ctx)
                .expect("failed")
                .unwrap()
                .set_variables()
                .get("bar.endsWith('baz')"),
            Some(&"true".to_string())
        );
    }
}
