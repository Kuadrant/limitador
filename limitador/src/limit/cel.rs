use crate::limit::cel::errors::EvaluationError;
use cel_interpreter::{ExecutionError, Value};
pub use errors::ParseError;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub(super) mod errors {
    use cel_interpreter::ExecutionError;
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
                    write!(f, "unexpected value of type {}", value)
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
        source: Box<dyn Error + 'static>,
    }

    impl ParseError {
        pub fn from(source: cel_parser::ParseError, input: String) -> Self {
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

pub struct Context {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Expression {
    source: String,
    expression: cel_parser::Expression,
}

impl Expression {
    pub fn parse<T: ToString>(source: T) -> Result<Self, ParseError> {
        let source = source.to_string();
        match cel_parser::parse(&source) {
            Ok(expression) => Ok(Self { source, expression }),
            Err(err) => Err(ParseError::from(err, source)),
        }
    }

    pub fn eval(&self, ctx: &Context) -> Result<String, EvaluationError> {
        match self.resolve(ctx)? {
            Value::Int(i) => Ok(i.to_string()),
            Value::UInt(i) => Ok(i.to_string()),
            Value::Float(f) => Ok(f.to_string()),
            Value::String(s) => Ok(s.to_string()),
            Value::Null => Ok("null".to_owned()),
            Value::Bool(b) => Ok(b.to_string()),
            val => Err(err_on_value(val)),
        }
    }

    pub fn resolve(&self, _ctx: &Context) -> Result<Value, ExecutionError> {
        let ctx = cel_interpreter::Context::default();
        Value::resolve(&self.expression, &ctx)
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
    }
}

impl TryFrom<String> for Expression {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
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

#[derive(Clone, Debug, Serialize)]
pub struct Predicate(Expression);

impl Predicate {
    pub fn parse<T: ToString>(source: T) -> Result<Self, ParseError> {
        Expression::parse(source).map(Self)
    }

    pub fn test(&self, ctx: &Context) -> Result<bool, EvaluationError> {
        match self.0.resolve(ctx)? {
            Value::Bool(b) => Ok(b),
            v => Err(err_on_value(v)),
        }
    }
}

impl Eq for Predicate {}

impl PartialEq<Self> for Predicate {
    fn eq(&self, other: &Self) -> bool {
        self.0.source == other.0.source
    }
}

impl PartialOrd<Self> for Predicate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Predicate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Expression, Predicate};

    #[test]
    fn expression() {
        let exp = Expression::parse("100").expect("failed to parse");
        assert_eq!(exp.eval(&Context {}), Ok(String::from("100")));
    }

    #[test]
    fn expression_serialization() {
        let exp = Expression::parse("100").expect("failed to parse");
        let serialized = serde_json::to_string(&exp).expect("failed to serialize");
        let deserialized: Expression =
            serde_json::from_str(&serialized).expect("failed to deserialize");
        assert_eq!(exp.eval(&Context {}), deserialized.eval(&Context {}));
    }

    #[test]
    fn unexpected_value_type_expression() {
        let exp = Expression::parse("['100']").expect("failed to parse");
        assert_eq!(
            exp.eval(&Context {}).map_err(|e| format!("{e}")),
            Err("unexpected value of type list: `[String(\"100\")]`".to_string())
        );
    }

    #[test]
    fn predicate() {
        let pred = Predicate::parse("42 == uint('42')").expect("failed to parse");
        assert_eq!(pred.test(&Context {}), Ok(true));
    }

    #[test]
    fn unexpected_value_predicate() {
        let pred = Predicate::parse("42").expect("failed to parse");
        assert_eq!(
            pred.test(&Context {}).map_err(|e| format!("{e}")),
            Err("unexpected value of type integer: `42`".to_string())
        );
    }
}
