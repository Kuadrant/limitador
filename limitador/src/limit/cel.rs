use crate::limit::Limit;
use cel::objects::Key;
use cel::{ExecutionError, Value};
pub use errors::{EvaluationError, ParseError};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

static CEL_ENV: LazyLock<Arc<cel::Env>> = LazyLock::new(|| Arc::new(cel::Env::stdlib()));

pub(super) mod errors {
    use cel::ExecutionError;
    use std::error::Error;
    use std::fmt::{Display, Formatter};

    #[derive(Debug, PartialEq)]
    pub enum EvaluationError {
        UnexpectedValueType(String),
        ExecutionError(ExecutionError),
    }

    impl Display for EvaluationError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                EvaluationError::UnexpectedValueType(value) => {
                    write!(f, "unexpected value of type {value}")
                }
                EvaluationError::ExecutionError(error) => error.fmt(f),
            }
        }
    }

    impl Error for EvaluationError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                EvaluationError::UnexpectedValueType(_) => None,
                EvaluationError::ExecutionError(err) => Some(err),
            }
        }
    }

    #[derive(Debug)]
    pub struct ParseError {
        input: String,
        source: Box<dyn Error + 'static + Send + Sync>,
    }

    impl ParseError {
        pub fn from(source: cel::ParseErrors, input: String) -> Self {
            Self {
                input,
                source: Box::new(source),
            }
        }
    }

    impl Display for ParseError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "couldn't parse {}: {}", self.input, self.source)
        }
    }

    impl Error for ParseError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self.source.as_ref())
        }
    }

    impl From<ExecutionError> for EvaluationError {
        fn from(err: ExecutionError) -> Self {
            EvaluationError::ExecutionError(err)
        }
    }
}

pub struct Context<'a> {
    variables: HashSet<String>,
    ctx: cel::Context<'a, 'static>,
}

impl<'a> Context<'a> {
    pub(crate) fn new(root: String, values: HashMap<String, String>) -> Self {
        let mut ctx = cel::Context::with_env(Arc::clone(&CEL_ENV));
        let mut variables = HashSet::new();

        if root.is_empty() {
            for (binding, value) in values {
                ctx.add_variable_from_value(binding.clone(), value.clone());
                variables.insert(binding);
            }
        } else {
            let map = cel::objects::Map::from(values.clone());
            ctx.add_variable_from_value(root, Value::Map(map));
        }

        Self { variables, ctx }
    }

    pub fn list_binding(&mut self, name: String, value: Vec<HashMap<String, String>>) {
        let v = value
            .iter()
            .map(|values| {
                let map = cel::objects::Map::from(values.clone());
                Value::Map(map)
            })
            .collect::<Vec<_>>();
        self.variables.insert(name.clone());
        self.ctx
            .add_variable_from_value(name, Value::List(v.into()));
    }

    pub(crate) fn for_limit<'b>(&'b self, limit: &Limit) -> Self
    where
        'b: 'a,
    {
        let mut inner = self.ctx.new_inner_scope();
        let limit_data = cel::objects::Map::from(HashMap::from([
            (
                "name",
                limit
                    .name
                    .as_ref()
                    .map(|n| Value::String(Arc::new(n.to_string())))
                    .unwrap_or(Value::Null),
            ),
            (
                "id",
                limit
                    .id
                    .as_ref()
                    .map(|n| Value::String(Arc::new(n.to_string())))
                    .unwrap_or(Value::Null),
            ),
        ]));
        inner.add_variable_from_value("limit", Value::Map(limit_data));
        Self {
            variables: self.variables.clone(),
            ctx: inner,
        }
    }

    pub(crate) fn has_variables(&self, names: &[&str]) -> bool {
        names.iter().all(|name| self.variables.contains(*name))
    }
}

impl Default for Context<'_> {
    fn default() -> Self {
        Self::new(String::default(), HashMap::default())
    }
}

impl From<HashMap<String, String>> for Context<'_> {
    fn from(value: HashMap<String, String>) -> Self {
        Self::new(String::default(), value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Expression {
    source: String,
    expression: cel::IdedExpr,
}

impl Expression {
    pub fn parse<T: ToString>(source: T) -> Result<Self, ParseError> {
        let source = source.to_string();
        let parser = CEL_ENV.parser();
        match parser.parse(&source) {
            Ok(expression) => Ok(Self { source, expression }),
            Err(err) => Err(ParseError::from(err, source)),
        }
    }

    pub fn eval(&self, ctx: &Context) -> Result<Option<String>, EvaluationError> {
        let result = self.resolve(ctx);
        match result {
            Ok(value) => match value {
                Value::Int(i) => Ok(i.to_string()),
                Value::UInt(i) => Ok(i.to_string()),
                Value::Float(f) => Ok(f.to_string()),
                Value::String(s) => Ok(s.to_string()),
                Value::Null => Ok("null".to_owned()),
                Value::Bool(b) => Ok(b.to_string()),
                val => Err(err_on_value(val)),
            }
            .map(Some),
            Err(ExecutionError::NoSuchKey(_)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn eval_map(&self, ctx: &Context) -> Result<HashMap<String, String>, EvaluationError> {
        match self.resolve(ctx)? {
            Value::Map(map) => Ok(map
                .map
                .iter()
                .filter_map(|(k, v)| {
                    if let (Key::String(k), Value::String(v)) = (k, v) {
                        Some((k.to_string(), v.to_string()))
                    } else {
                        None
                    }
                })
                .collect()),
            _ => Ok(HashMap::default()),
        }
    }

    pub(super) fn resolve(&self, ctx: &Context) -> Result<Value, ExecutionError> {
        Value::resolve(&self.expression, &ctx.ctx)
    }

    pub fn source(&self) -> &str {
        self.source.as_str()
    }

    pub fn variables(&self) -> Vec<String> {
        self.expression
            .references()
            .variables()
            .into_iter()
            .map(String::from)
            .collect()
    }
}

fn err_on_value(val: Value) -> EvaluationError {
    match val {
        Value::List(list) => EvaluationError::UnexpectedValueType(format!("list: `{:?}`", *list)),
        Value::Map(map) => EvaluationError::UnexpectedValueType(format!("map: `{:?}`", *map.map)),
        Value::Function(ident, _) => {
            EvaluationError::UnexpectedValueType(format!("function: `{}`", *ident))
        }
        Value::Bytes(b) => EvaluationError::UnexpectedValueType(format!("function: `{:?}`", *b)),
        Value::Duration(d) => EvaluationError::UnexpectedValueType(format!("duration: `{d}`")),
        Value::Timestamp(ts) => EvaluationError::UnexpectedValueType(format!("timestamp: `{ts}`")),
        Value::Int(i) => EvaluationError::UnexpectedValueType(format!("integer: `{i}`")),
        Value::UInt(u) => EvaluationError::UnexpectedValueType(format!("unsigned integer: `{u}`")),
        Value::Float(f) => EvaluationError::UnexpectedValueType(format!("float: `{f}`")),
        Value::String(s) => EvaluationError::UnexpectedValueType(format!("string: `{s}`")),
        Value::Bool(b) => EvaluationError::UnexpectedValueType(format!("bool: `{b}`")),
        Value::Null => EvaluationError::UnexpectedValueType("null".to_owned()),
        Value::Opaque(o) => {
            EvaluationError::UnexpectedValueType(format!("opaque: `{}`", o.runtime_type_name()))
        }
        Value::Struct(s) => EvaluationError::UnexpectedValueType(format!("struct: `{}`", s.name())),
    }
}

impl TryFrom<String> for Expression {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for Predicate {
    type Error = ParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<Expression> for String {
    fn from(value: Expression) -> Self {
        value.source
    }
}

impl PartialEq<Self> for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Expression {}

impl Hash for Expression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

impl PartialOrd<Self> for Expression {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Expression {
    fn cmp(&self, other: &Self) -> Ordering {
        self.source.cmp(&other.source)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Predicate {
    #[serde(skip_serializing, default)]
    variables: HashSet<String>,
    expression: Expression,
}

impl Predicate {
    pub fn parse<T: ToString>(source: T) -> Result<Self, ParseError> {
        Expression::parse(source).map(|e| Self {
            variables: e
                .expression
                .references()
                .variables()
                .into_iter()
                .map(String::from)
                .collect(),
            expression: e,
        })
    }

    pub fn test(&self, ctx: &Context) -> Result<bool, EvaluationError> {
        if !self
            .variables
            .iter()
            .filter(|binding| binding.as_str() != "limit")
            .all(|v| ctx.variables.contains(v))
        {
            return Ok(false);
        }

        match self.expression.resolve(ctx) {
            Ok(value) => match value {
                Value::Bool(b) => Ok(b),
                v => Err(err_on_value(v)),
            },
            Err(ExecutionError::NoSuchKey(_)) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }
}

impl Eq for Predicate {}

impl PartialEq<Self> for Predicate {
    fn eq(&self, other: &Self) -> bool {
        self.expression.source == other.expression.source
    }
}

impl PartialOrd<Self> for Predicate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Predicate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.expression.cmp(&other.expression)
    }
}

impl Hash for Predicate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.expression.source.hash(state);
    }
}

impl TryFrom<String> for Predicate {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for Expression {
    type Error = ParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<Predicate> for String {
    fn from(value: Predicate) -> Self {
        value.expression.source
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, EvaluationError, ExecutionError, Expression, Predicate};
    use std::collections::HashMap;

    #[test]
    fn expression() {
        let exp = Expression::parse("100").expect("failed to parse");
        assert_eq!(exp.eval(&ctx()), Ok(Some(String::from("100"))));
    }

    #[test]
    fn expression_serialization() {
        let exp = Expression::parse("100").expect("failed to parse");
        let serialized = serde_json::to_string(&exp).expect("failed to serialize");
        let deserialized: Expression =
            serde_json::from_str(&serialized).expect("failed to deserialize");
        assert_eq!(exp.eval(&ctx()), deserialized.eval(&ctx()));
    }

    #[test]
    fn expression_missing_map_key() {
        let exp = Expression::parse("{'present': 'value'}.missing").expect("failed to parse");
        assert_eq!(exp.eval(&ctx()), Ok(None));
    }

    #[test]
    fn expression_map_keeps_only_string_labels() {
        let exp = Expression::parse("{'label': 'value', 'number': 42, 1: 'ignored'}")
            .expect("failed to parse");
        assert_eq!(
            exp.eval_map(&ctx()),
            Ok(HashMap::from([("label".to_string(), "value".to_string())]))
        );
    }

    #[test]
    fn expression_scalar_values() {
        for (source, expected) in [
            ("42", "42"),
            ("42u", "42"),
            ("1.5", "1.5"),
            ("'value'", "value"),
            ("true", "true"),
            ("null", "null"),
        ] {
            let exp = Expression::parse(source).expect("failed to parse");
            assert_eq!(exp.eval(&ctx()), Ok(Some(expected.to_string())), "{source}");
        }
    }

    #[test]
    fn parse_error_preserves_source() {
        let err = Expression::parse("1 +").expect_err("invalid expression should not parse");
        assert!(err.to_string().starts_with("couldn't parse 1 +:"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn unexpected_value_type_expression() {
        let exp = Expression::parse("['100']").expect("failed to parse");
        assert_eq!(
            exp.eval(&ctx()).map_err(|e| format!("{e}")),
            Err("unexpected value of type list: `[String(\"100\")]`".to_string())
        );
    }

    #[test]
    fn predicate() {
        let pred = Predicate::parse("42 == uint('42')").expect("failed to parse");
        assert_eq!(pred.test(&ctx()), Ok(true));
    }

    #[test]
    fn predicate_no_var() {
        let pred = Predicate::parse("not_there == 42").expect("failed to parse");
        assert_eq!(pred.test(&ctx()), Ok(false));
    }

    #[test]
    fn predicate_no_key() {
        let pred = Predicate::parse("there[0].not == 42").expect("failed to parse");
        let mut ctx = Context::default();
        ctx.list_binding("there".to_string(), vec![HashMap::default()]);
        assert_eq!(pred.test(&ctx), Ok(false));
    }

    #[test]
    fn predicate_field_access_on_a_string_is_a_type_error() {
        let pred = Predicate::parse("there.not == 42").expect("failed to parse");
        assert!(matches!(
            pred.test(&HashMap::from([("there".to_string(), String::default())]).into()),
            Err(EvaluationError::ExecutionError(
                ExecutionError::NoSuchOverload(_)
            ))
        ));
    }

    #[test]
    fn unexpected_value_predicate() {
        let pred = Predicate::parse("42").expect("failed to parse");
        assert_eq!(
            pred.test(&ctx()).map_err(|e| format!("{e}")),
            Err("unexpected value of type integer: `42`".to_string())
        );
    }

    #[test]
    fn supports_list_bindings() {
        let pred = Predicate::parse("root[0].key == '1' && root[1]['key'] == '2'")
            .expect("failed to parse");
        let mut ctx = Context::default();
        ctx.list_binding(
            "root".to_string(),
            vec![
                HashMap::from([("key".to_string(), "1".to_string())]),
                HashMap::from([("key".to_string(), "2".to_string())]),
            ],
        );
        assert_eq!(pred.test(&ctx).map_err(|e| format!("{e}")), Ok(true));
    }

    fn ctx<'a>() -> Context<'a> {
        Context::default()
    }
}
