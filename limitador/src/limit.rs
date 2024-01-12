use cel_interpreter::{Context, Expression, Value};
use cel_parser::parse;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};

#[cfg(feature = "lenient_conditions")]
mod deprecated {
    use std::sync::atomic::{AtomicBool, Ordering};

    static DEPRECATED_SYNTAX: AtomicBool = AtomicBool::new(false);

    pub fn check_deprecated_syntax_usages_and_reset() -> bool {
        match DEPRECATED_SYNTAX.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(previous) => previous,
            Err(previous) => previous,
        }
    }
}

#[cfg(feature = "lenient_conditions")]
pub use deprecated::check_deprecated_syntax_usages_and_reset;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Serialize, Deserialize)]
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
    namespace: Namespace,
    #[serde(skip_serializing, default)]
    max_value: i64,
    seconds: u64,
    #[serde(skip_serializing, default)]
    name: Option<String>,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    #[serde(serialize_with = "ordered_condition_set")]
    conditions: HashSet<Condition>,
    #[serde(serialize_with = "ordered_set")]
    variables: HashSet<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(try_from = "String", into = "String")]
pub struct Condition {
    source: String,
    expression: Expression,
}

impl PartialEq for Condition {
    fn eq(&self, other: &Self) -> bool {
        self.expression == other.expression
    }
}

impl Eq for Condition {}

impl Hash for Condition {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source.hash(state)
    }
}

#[derive(Debug)]
pub struct ConditionParsingError {
    msg: String,
}

impl Display for ConditionParsingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for ConditionParsingError {}

impl TryFrom<&str> for Condition {
    type Error = ConditionParsingError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.to_owned().try_into()
    }
}

impl TryFrom<String> for Condition {
    type Error = ConditionParsingError;

    fn try_from(source: String) -> Result<Self, Self::Error> {
        match parse(&source) {
            Ok(expression) => Ok(Condition { source, expression }),
            Err(err) => Err(ConditionParsingError {
                msg: err.to_string(),
            }),
        }
    }
}

impl From<Condition> for String {
    fn from(condition: Condition) -> Self {
        condition.source.clone()
    }
}

fn ordered_condition_set<S>(value: &HashSet<Condition>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeSet<String> = value.iter().map(|c| c.clone().into()).collect();
    ordered.serialize(serializer)
}

fn ordered_set<S>(value: &HashSet<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeSet<_> = value.iter().collect();
    ordered.serialize(serializer)
}

impl Limit {
    pub fn new<N: Into<Namespace>, T: TryInto<Condition>>(
        namespace: N,
        max_value: i64,
        seconds: u64,
        conditions: impl IntoIterator<Item = T>,
        variables: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self
    where
        <N as TryInto<Namespace>>::Error: core::fmt::Debug,
        <T as TryInto<Condition>>::Error: core::fmt::Debug,
    {
        // the above where-clause is needed in order to call unwrap().
        Self {
            namespace: namespace.into(),
            max_value,
            seconds,
            name: None,
            conditions: conditions
                .into_iter()
                .map(|cond| cond.try_into().expect("Invalid condition"))
                .collect(),
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

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn set_name(&mut self, name: String) {
        self.name = Some(name)
    }

    pub fn set_max_value(&mut self, value: i64) {
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
        let all_conditions_apply = self
            .conditions
            .iter()
            .all(|cond| Self::condition_applies(cond, values));

        let all_vars_are_set = self.variables.iter().all(|var| values.contains_key(var));

        all_conditions_apply && all_vars_are_set
    }

    fn condition_applies(condition: &Condition, values: &HashMap<String, String>) -> bool {
        let mut context = Context::default();
        for (key, val) in values {
            context.add_variable(key.clone(), val.to_owned());
        }

        match Value::resolve(&condition.expression, &context) {
            Ok(val) => val == true.into(),
            Err(_) => false,
        }
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

    #[test]
    fn limit_can_have_an_optional_name() {
        let mut limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"]);
        assert!(limit.name.is_none());

        let name = "Test Limit";
        limit.set_name(name.to_string());
        assert_eq!(name, limit.name.unwrap())
    }

    #[test]
    fn limit_applies() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "1".into());

        assert!(limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_is_false() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    #[cfg(feature = "lenient_conditions")]
    fn limit_does_not_apply_when_cond_is_false_deprecated_style() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values));
        assert!(check_deprecated_syntax_usages_and_reset());
        assert!(!check_deprecated_syntax_usages_and_reset());

        let limit = Limit::new("test_namespace", 10, 60, vec!["x == foobar"], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "foobar".into());
        values.insert("y".into(), "1".into());

        assert!(limit.applies(&values));
        assert!(check_deprecated_syntax_usages_and_reset());
        assert!(!check_deprecated_syntax_usages_and_reset());
    }

    #[test]
    fn limit_does_not_apply_when_cond_var_is_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"]);

        // Notice that "x" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("a".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_var_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == \"5\""], vec!["y"]);

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
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "5".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(limit.applies(&values))
    }

    #[test]
    fn limit_applies_when_all_its_conditions_apply_with_subexpression() {
        let limit = Limit::new(
            "test_namespace",
            10,
            60,
            vec!["x == string((11 - 1) / 2)", "y == \"2\""],
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
            vec!["x == \"5\"", "y == \"2\""],
            vec!["z"],
        );

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "3".into());
        values.insert("y".into(), "2".into());
        values.insert("z".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn valid_condition_literal_parsing() {
        let result: Condition = serde_json::from_str(r#""x == '5'""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                source: "x == '5'".to_string(),
                expression: parse("x == '5'").unwrap(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar=='ok' ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                source: "  foobar=='ok' ".to_string(),
                expression: parse("foobar == 'ok'").unwrap(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar  ==   'ok'  ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                source: "  foobar  ==   'ok'  ".to_string(),
                expression: parse("  foobar  ==   'ok'  ").unwrap(),
            }
        );
    }

    #[ignore]
    #[test]
    #[cfg(not(feature = "lenient_conditions"))]
    fn invalid_deprecated_condition_parsing() {
        let _result = serde_json::from_str::<Condition>(r#""x == 5""#)
            .err()
            .expect("Should fail!");
    }

    #[ignore]
    #[test]
    fn invalid_condition_parsing() {
        let result = serde_json::from_str::<Condition>(r#""x != 5 ` x > 12""#)
            .expect_err("should fail parsing");
        assert_eq!(
            result.to_string(),
            "SyntaxError: Invalid character `&` at offset 8 of condition \"x != 5 && x > 12\""
                .to_string()
        );
    }

    #[test]
    fn condition_serialization() {
        let condition = Condition {
            source: "foobar == \"ok\"".to_string(),
            expression: parse("foobar == ok").unwrap(),
        };
        let result = serde_json::to_string(&condition).expect("Should serialize");
        assert_eq!(result, r#""foobar == \"ok\"""#.to_string());
    }
}
