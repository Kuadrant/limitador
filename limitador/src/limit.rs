use crate::limit::conditions::{Literal, TokenType};
use serde::{Deserialize, Serialize, Serializer};
use std::cmp::Ordering;
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
    #[serde(skip)]
    conditions: HashSet<Condition>,
    #[serde(serialize_with = "ordered_set")]
    variables: HashSet<String>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug, Clone, Hash)]
#[serde(try_from = "&str", into = "String")]
pub struct Condition {
    var_name: String,
    predicate: Predicate,
    operand: String,
}

impl TryFrom<String> for Condition {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.as_str().try_into()
    }
}

impl TryFrom<&str> for Condition {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match conditions::Scanner::scan(value.to_owned()) {
            Ok(tokens) => match tokens.len().cmp(&(3_usize)) {
                Ordering::Equal => {
                    match (
                        &tokens[0].token_type,
                        &tokens[1].token_type,
                        &tokens[2].token_type,
                    ) {
                        (TokenType::Identifier, TokenType::EqualEqual, TokenType::String) => {
                            if let (
                                Some(Literal::Identifier(var_name)),
                                Some(Literal::String(operand)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate: Predicate::EQUAL,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {:?} returned from Scanner for: `{}`",
                                    tokens, value
                                )
                            }
                        }
                        (TokenType::String, TokenType::EqualEqual, TokenType::Identifier) => {
                            if let (
                                Some(Literal::String(operand)),
                                Some(Literal::Identifier(var_name)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate: Predicate::EQUAL,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {:?} returned from Scanner for: `{}`",
                                    tokens, value
                                )
                            }
                        }
                        // For backwards compatibility!
                        (TokenType::Identifier, TokenType::EqualEqual, TokenType::Identifier) => {
                            if let (
                                Some(Literal::Identifier(operand)),
                                Some(Literal::Identifier(var_name)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate: Predicate::EQUAL,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {:?} returned from Scanner for: `{}`",
                                    tokens, value
                                )
                            }
                        }
                        (_, _, _) => Err(format!(
                            "Unexpected token {:?} at {} for: {}",
                            tokens[0], tokens[0].pos, value
                        )),
                    }
                }
                Ordering::Less => Err(format!("Missing expected token for: `{}`", value)),
                Ordering::Greater => Err(format!(
                    "Unexpected token {:?} at {} for: {}",
                    tokens[3], tokens[3].pos, value
                )),
            },
            Err(pos) => Err(format!("Invalid character at {} for: `{}`", pos, value)),
        }
    }
}

impl From<Condition> for String {
    fn from(condition: Condition) -> Self {
        let p = &condition.predicate;
        let predicate: String = p.clone().into();
        format!(
            "{} {} \"{}\"",
            condition.var_name, predicate, condition.operand
        )
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash)]
pub enum Predicate {
    EQUAL,
}

impl Predicate {
    fn test(&self, lhs: &str, rhs: &str) -> bool {
        match self {
            Predicate::EQUAL => lhs == rhs,
        }
    }
}

impl From<Predicate> for String {
    fn from(op: Predicate) -> Self {
        match op {
            Predicate::EQUAL => "==".to_string(),
        }
    }
}

fn ordered_condition_set<S>(value: &HashSet<Condition>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeSet<String> = value
        .iter()
        .map(|c| {
            let s: String = c.clone().into();
            s
        })
        .collect();
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
                .map(|cond| cond.try_into().expect("Duh!"))
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
        let left_operand = condition.var_name.as_str();
        let right_operand = condition.operand.as_str();

        match values.get(left_operand) {
            Some(val) => condition.predicate.test(val, right_operand),
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
            .map(|c| c.clone().into())
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

mod conditions {

    #[derive(Eq, PartialEq, Debug)]
    pub enum TokenType {
        // Predicates
        EqualEqual,

        //Literals
        Identifier,
        String,
    }

    #[derive(Eq, PartialEq, Debug)]
    pub enum Literal {
        Identifier(String),
        String(String),
    }

    #[derive(Eq, PartialEq, Debug)]
    pub struct Token {
        pub token_type: TokenType,
        pub literal: Option<Literal>,
        pub pos: usize,
    }

    pub struct Scanner {
        input: Vec<char>,
        pos: usize,
    }

    impl Scanner {
        pub fn scan(condition: String) -> Result<Vec<Token>, usize> {
            let mut tokens: Vec<Token> = Vec::with_capacity(3);
            let mut scanner = Scanner {
                input: condition.chars().collect(),
                pos: 0,
            };
            while !scanner.done() {
                match scanner.next_token() {
                    Ok(token) => {
                        if let Some(token) = token {
                            tokens.push(token)
                        }
                    }
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
            Ok(tokens)
        }

        fn next_token(&mut self) -> Result<Option<Token>, usize> {
            let character = self.advance();
            match character {
                '=' => {
                    if self.next_matches('=') {
                        Ok(Some(Token {
                            token_type: TokenType::EqualEqual,
                            literal: None,
                            pos: self.pos - 1,
                        }))
                    } else {
                        Err(self.pos)
                    }
                }
                '"' | '\'' => self.scan_string(character).map(Some),
                ' ' | '\n' | '\r' | '\t' => Ok(None),
                _ => {
                    if character.is_alphabetic() {
                        self.scan_identifier().map(Some)
                    } else {
                        Err(self.pos)
                    }
                }
            }
        }

        fn scan_identifier(&mut self) -> Result<Token, usize> {
            let start = self.pos - 1;
            while !self.done() && self.valid_id_char() {
                self.advance();
            }
            Ok(Token {
                token_type: TokenType::Identifier,
                literal: Some(Literal::Identifier(
                    self.input[start..self.pos].iter().collect(),
                )),
                pos: start + 1,
            })
        }

        fn valid_id_char(&mut self) -> bool {
            let char = self.input[self.pos];
            char.is_alphanumeric() || char == '.'
        }

        fn scan_string(&mut self, until: char) -> Result<Token, usize> {
            let start = self.pos;
            while !self.done() && self.advance() != until {}
            Ok(Token {
                token_type: TokenType::String,
                literal: Some(Literal::String(
                    self.input[start..self.pos - 1].iter().collect(),
                )),
                pos: start,
            })
        }

        fn advance(&mut self) -> char {
            let char = self.input[self.pos];
            self.pos += 1;
            char
        }

        fn next_matches(&mut self, c: char) -> bool {
            if self.done() || self.input[self.pos] != c {
                return false;
            }

            self.pos += 1;
            true
        }

        fn done(&self) -> bool {
            self.pos >= self.input.len()
        }
    }

    #[cfg(test)]
    mod tests {
        use crate::limit::conditions::Literal::Identifier;
        use crate::limit::conditions::{Literal, Scanner, Token, TokenType};

        #[test]
        fn test_scanner() {
            let mut tokens =
                Scanner::scan("foo=='bar '".to_owned()).expect("Should parse alright!");
            assert_eq!(tokens.len(), 3);
            assert_eq!(
                tokens[0],
                Token {
                    token_type: TokenType::Identifier,
                    literal: Some(Identifier("foo".to_owned())),
                    pos: 1,
                }
            );
            assert_eq!(
                tokens[1],
                Token {
                    token_type: TokenType::EqualEqual,
                    literal: None,
                    pos: 4,
                }
            );
            assert_eq!(
                tokens[2],
                Token {
                    token_type: TokenType::String,
                    literal: Some(Literal::String("bar ".to_owned())),
                    pos: 6,
                }
            );

            tokens[1].pos += 1;
            tokens[2].pos += 2;
            assert_eq!(
                tokens,
                Scanner::scan("foo == 'bar '".to_owned()).expect("Should parse alright!")
            );

            tokens[0].pos += 2;
            tokens[1].pos += 2;
            tokens[2].pos += 2;
            assert_eq!(
                tokens,
                Scanner::scan("  foo == 'bar ' ".to_owned()).expect("Should parse alright!")
            );

            tokens[1].pos += 2;
            tokens[2].pos += 4;
            assert_eq!(
                tokens,
                Scanner::scan("  foo   ==   'bar ' ".to_owned()).expect("Should parse alright!")
            );
        }

        #[test]
        fn test_charset() {
            let tokens =
                Scanner::scan(" 変数 == '  💖 '".to_owned()).expect("Should parse alright!");
            assert_eq!(tokens.len(), 3);
            assert_eq!(
                tokens[0],
                Token {
                    token_type: TokenType::Identifier,
                    literal: Some(Identifier("変数".to_owned())),
                    pos: 2,
                }
            );
            assert_eq!(
                tokens[1],
                Token {
                    token_type: TokenType::EqualEqual,
                    literal: None,
                    pos: 5,
                }
            );
            assert_eq!(
                tokens[2],
                Token {
                    token_type: TokenType::String,
                    literal: Some(Literal::String("  💖 ".to_owned())),
                    pos: 8,
                }
            );
        }
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
    fn limit_does_not_apply_when_cond_is_false_deprecated_style() {
        let limit = Limit::new("test_namespace", 10, 60, vec!["x == foobar"], vec!["y"]);

        let mut values: HashMap<String, String> = HashMap::new();
        values.insert("x".into(), "foobar".into());
        values.insert("y".into(), "1".into());

        assert!(!limit.applies(&values))
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
    fn valid_condition_positional_parsing() {
        let result: Condition = serde_json::from_str(r#""x == '5'""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "x".to_string(),
                predicate: Predicate::EQUAL,
                operand: "5".to_string(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar  ==   'ok'  ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "foobar".to_string(),
                predicate: Predicate::EQUAL,
                operand: "ok".to_string(),
            }
        );
    }

    #[test]
    fn valid_condition_literal_parsing() {
        let result: Condition = serde_json::from_str(r#""x == '5'""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "x".to_string(),
                predicate: Predicate::EQUAL,
                operand: "5".to_string(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar=='ok' ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "foobar".to_string(),
                predicate: Predicate::EQUAL,
                operand: "ok".to_string(),
            }
        );
    }

    #[test]
    fn invalid_predicate_condition_parsing() {
        let result = serde_json::from_str::<'static, Condition>(r#""x != 5""#)
            .err()
            .expect("should fail parsing");
        assert_eq!(
            result.to_string(),
            "Invalid character at 3 for: `x != 5`".to_string()
        );
    }

    #[test]
    fn invalid_condition_parsing() {
        let result = serde_json::from_str::<'static, Condition>(r#""x != 5 && x > 12""#)
            .err()
            .expect("should fail parsing");
        assert_eq!(
            result.to_string(),
            "Invalid character at 3 for: `x != 5 && x > 12`".to_string()
        );
    }

    #[test]
    fn condition_serialization() {
        let condition = Condition {
            var_name: "foobar".to_string(),
            predicate: Predicate::EQUAL,
            operand: "ok".to_string(),
        };
        let result = serde_json::to_string(&condition).expect("Should serialize");
        assert_eq!(result, r#""foobar == \"ok\"""#.to_string());
    }
}
