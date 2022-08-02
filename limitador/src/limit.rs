use serde::{Deserialize, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

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
    #[serde(serialize_with = "ordered_set")]
    conditions: HashSet<String>,
    #[serde(serialize_with = "ordered_set")]
    variables: HashSet<String>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug, Clone)]
#[serde(try_from = "&str", into = "String")]
pub struct Condition {
    var_name: String,
    operator: Operator,
    operand: String,
}

impl TryFrom<&str> for Condition {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let split: Vec<&str> = value.split_whitespace().collect();
        if split.len() != 3 {
            Err(format!("Invalid condition format for: {}", value))
        } else {
            let var_name = split[0].to_string();
            let operator = split[1].try_into()?;
            let operand = split[2].to_string();
            Ok(Condition {
                var_name,
                operator,
                operand,
            })
        }
    }
}

impl Into<String> for Condition {
    fn into(self) -> String {
        let operator: String = self.operator.into();
        format!("{} {} {}", self.var_name, operator, self.operand)
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug, Clone)]
#[serde(try_from = "&str", into = "String")]
pub enum Operator {
    EQUAL,
}

impl TryFrom<&str> for Operator {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "==" => Ok(Operator::EQUAL),
            _ => Err(format!("Invalid operator: {}", value)),
        }
    }
}

impl Into<String> for Operator {
    fn into(self) -> String {
        match self {
            Operator::EQUAL => "==".to_string(),
        }
    }
}

fn ordered_condition_set<S>(value: &HashSet<Condition>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeSet<String> = value.iter().map(|c| {
        let s: String = c.clone().into();
        s
    }).collect();
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
    pub fn new<N: Into<Namespace>>(
        namespace: N,
        max_value: i64,
        seconds: u64,
        conditions: impl IntoIterator<Item = impl Into<String>>,
        variables: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self
    where
        <N as TryInto<Namespace>>::Error: core::fmt::Debug,
    {
        // the above where-clause is needed in order to call unwrap().
        Self {
            namespace: namespace.into(),
            max_value,
            seconds,
            name: None,
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
            .all(|cond| Self::condition_applies(cond, values));

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
        let mut limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);
        assert!(limit.name.is_none());

        let name = "Test Limit";
        limit.set_name(name.to_string());
        assert_eq!(name, limit.name.unwrap())
    }

    #[test]
    fn limit_applies() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

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

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_cond_var_is_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

        // Notice that "x" is not set
        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("a".into(), "1".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
    }

    #[test]
    fn limit_does_not_apply_when_var_not_set() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == 5"], vec!["y"]);

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

        assert!(!limit.applies(&values))
    }

    #[test]
    fn valid_condition_parsing() {
        let result: Condition = serde_json::from_str(r#""x == 5""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "x".to_string(),
                operator: Operator::EQUAL,
                operand: "5".to_string(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar  ==   ok  ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "foobar".to_string(),
                operator: Operator::EQUAL,
                operand: "ok".to_string(),
            }
        );
    }

    #[test]
    fn invalid_operator_condition_parsing() {
        let result = serde_json::from_str::<'static, Condition>(r#""x != 5""#)
            .err()
            .expect("should fail parsing");
        assert_eq!(result.to_string(), "Invalid operator: !=".to_string());
    }

    #[test]
    fn invalid_condition_parsing() {
        let result = serde_json::from_str::<'static, Condition>(r#""x != 5 && x > 12""#)
            .err()
            .expect("should fail parsing");
        assert_eq!(
            result.to_string(),
            "Invalid condition format for: x != 5 && x > 12".to_string()
        );
    }

    #[test]
    fn condition_serilization() {
        let condition = Condition {
            var_name: "foobar".to_string(),
            operator: Operator::EQUAL,
            operand: "ok".to_string(),
        };
        let result = serde_json::to_string(&condition).expect("Should serialize");
        assert_eq!(result, r#""foobar == ok""#.to_string());
    }
}
