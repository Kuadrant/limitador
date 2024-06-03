use crate::limit::conditions::{ErrorType, Literal, SyntaxError, Token, TokenType};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
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

    pub fn deprecated_syntax_used() {
        DEPRECATED_SYNTAX.fetch_or(true, Ordering::SeqCst);
    }
}

#[cfg(feature = "lenient_conditions")]
pub use deprecated::check_deprecated_syntax_usages_and_reset;

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

#[derive(Eq, Debug, Clone, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Limit {
    namespace: Namespace,
    #[serde(skip_serializing, default)]
    max_value: u64,
    seconds: u64,
    #[serde(skip_serializing, default)]
    name: Option<String>,

    // Need to sort to generate the same object when using the JSON as a key or
    // value in Redis.
    conditions: BTreeSet<Condition>,
    variables: BTreeSet<String>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug, Clone, Hash, PartialOrd, Ord)]
#[serde(try_from = "String", into = "String")]
pub struct Condition {
    var_name: String,
    predicate: Predicate,
    operand: String,
}

#[derive(Debug)]
pub struct ConditionParsingError {
    error: SyntaxError,
    pub tokens: Vec<Token>,
    condition: String,
}

impl Display for ConditionParsingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} of condition \"{}\"", self.error, self.condition)
    }
}

impl Error for ConditionParsingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

impl TryFrom<&str> for Condition {
    type Error = ConditionParsingError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.to_owned().try_into()
    }
}

impl TryFrom<String> for Condition {
    type Error = ConditionParsingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match conditions::Scanner::scan(value.clone()) {
            Ok(tokens) => match tokens.len().cmp(&(3_usize)) {
                Ordering::Equal => {
                    match (
                        &tokens[0].token_type,
                        &tokens[1].token_type,
                        &tokens[2].token_type,
                    ) {
                        (
                            TokenType::Identifier,
                            TokenType::EqualEqual | TokenType::NotEqual,
                            TokenType::String,
                        ) => {
                            if let (
                                Some(Literal::Identifier(var_name)),
                                Some(Literal::String(operand)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                let predicate = match &tokens[1].token_type {
                                    TokenType::EqualEqual => Predicate::Equal,
                                    TokenType::NotEqual => Predicate::NotEqual,
                                    _ => unreachable!(),
                                };
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {tokens:?} returned from Scanner for: `{value}`"
                                )
                            }
                        }
                        (
                            TokenType::String,
                            TokenType::EqualEqual | TokenType::NotEqual,
                            TokenType::Identifier,
                        ) => {
                            if let (
                                Some(Literal::String(operand)),
                                Some(Literal::Identifier(var_name)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                let predicate = match &tokens[1].token_type {
                                    TokenType::EqualEqual => Predicate::Equal,
                                    TokenType::NotEqual => Predicate::NotEqual,
                                    _ => unreachable!(),
                                };
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {tokens:?} returned from Scanner for: `{value}`"
                                )
                            }
                        }
                        #[cfg(feature = "lenient_conditions")]
                        (TokenType::Identifier, TokenType::EqualEqual, TokenType::Identifier) => {
                            if let (
                                Some(Literal::Identifier(var_name)),
                                Some(Literal::Identifier(operand)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                deprecated::deprecated_syntax_used();
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate: Predicate::Equal,
                                    operand: operand.clone(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {tokens:?} returned from Scanner for: `{value}`"
                                )
                            }
                        }
                        #[cfg(feature = "lenient_conditions")]
                        (TokenType::Identifier, TokenType::EqualEqual, TokenType::Number) => {
                            if let (
                                Some(Literal::Identifier(var_name)),
                                Some(Literal::Number(operand)),
                            ) = (&tokens[0].literal, &tokens[2].literal)
                            {
                                deprecated::deprecated_syntax_used();
                                Ok(Condition {
                                    var_name: var_name.clone(),
                                    predicate: Predicate::Equal,
                                    operand: operand.to_string(),
                                })
                            } else {
                                panic!(
                                    "Unexpected state {tokens:?} returned from Scanner for: `{value}`"
                                )
                            }
                        }
                        (t1, t2, _) => {
                            let faulty = match (t1, t2) {
                                (
                                    TokenType::Identifier | TokenType::String,
                                    TokenType::EqualEqual | TokenType::NotEqual,
                                ) => 2,
                                (TokenType::Identifier | TokenType::String, _) => 1,
                                (_, _) => 0,
                            };
                            Err(ConditionParsingError {
                                error: SyntaxError {
                                    pos: tokens[faulty].pos,
                                    error: ErrorType::UnexpectedToken(tokens[faulty].clone()),
                                },
                                tokens,
                                condition: value,
                            })
                        }
                    }
                }
                Ordering::Less => Err(ConditionParsingError {
                    error: SyntaxError {
                        pos: value.len(),
                        error: ErrorType::MissingToken,
                    },
                    tokens,
                    condition: value,
                }),
                Ordering::Greater => Err(ConditionParsingError {
                    error: SyntaxError {
                        pos: tokens[3].pos,
                        error: ErrorType::UnexpectedToken(tokens[3].clone()),
                    },
                    tokens,
                    condition: value,
                }),
            },
            Err(err) => Err(ConditionParsingError {
                error: err,
                tokens: Vec::new(),
                condition: value,
            }),
        }
    }
}

impl From<Condition> for String {
    fn from(condition: Condition) -> Self {
        let p = &condition.predicate;
        let predicate: String = p.clone().into();
        let quotes = if condition.operand.contains('"') {
            '\''
        } else {
            '"'
        };
        format!(
            "{} {} {}{}{}",
            condition.var_name, predicate, quotes, condition.operand, quotes
        )
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Hash)]
pub enum Predicate {
    Equal,
    NotEqual,
}

impl Predicate {
    fn test(&self, lhs: &str, rhs: &str) -> bool {
        match self {
            Predicate::Equal => lhs == rhs,
            Predicate::NotEqual => lhs != rhs,
        }
    }
}

impl From<Predicate> for String {
    fn from(op: Predicate) -> Self {
        match op {
            Predicate::Equal => "==".to_string(),
            Predicate::NotEqual => "!=".to_string(),
        }
    }
}

impl Limit {
    pub fn new<N: Into<Namespace>, T: TryInto<Condition>>(
        namespace: N,
        max_value: u64,
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

mod conditions {
    use std::error::Error;
    use std::fmt::{Debug, Display, Formatter};
    use std::num::IntErrorKind;

    #[derive(Debug)]
    pub struct SyntaxError {
        pub pos: usize,
        pub error: ErrorType,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub enum ErrorType {
        UnexpectedToken(Token),
        MissingToken,
        InvalidCharacter(char),
        InvalidNumber,
        UnclosedStringLiteral(char),
    }

    impl Display for SyntaxError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match &self.error {
                ErrorType::UnexpectedToken(token) => write!(
                    f,
                    "SyntaxError: Unexpected token `{}` at offset {}",
                    token, self.pos
                ),
                ErrorType::InvalidCharacter(char) => write!(
                    f,
                    "SyntaxError: Invalid character `{}` at offset {}",
                    char, self.pos
                ),
                ErrorType::InvalidNumber => {
                    write!(f, "SyntaxError: Invalid number at offset {}", self.pos)
                }
                ErrorType::MissingToken => {
                    write!(f, "SyntaxError: Expected token at offset {}", self.pos)
                }
                ErrorType::UnclosedStringLiteral(char) => {
                    write!(f, "SyntaxError: Missing closing `{}` for string literal starting at offset {}", char, self.pos)
                }
            }
        }
    }

    impl Error for SyntaxError {}

    #[derive(Clone, Eq, PartialEq, Debug)]
    pub enum TokenType {
        // Predicates
        EqualEqual,
        NotEqual,

        //Literals
        Identifier,
        String,
        Number,
    }

    #[derive(Clone, Eq, PartialEq, Debug)]
    pub enum Literal {
        Identifier(String),
        String(String),
        Number(i64),
    }

    impl Display for Literal {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                Literal::Identifier(id) => write!(f, "{id}"),
                Literal::String(string) => write!(f, "'{string}'"),
                Literal::Number(number) => write!(f, "{number}"),
            }
        }
    }

    #[derive(Clone, Eq, PartialEq, Debug)]
    pub struct Token {
        pub token_type: TokenType,
        pub literal: Option<Literal>,
        pub pos: usize,
    }

    impl Display for Token {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self.token_type {
                TokenType::EqualEqual => write!(f, "Equality (==)"),
                TokenType::NotEqual => write!(f, "Unequal (!=)"),
                TokenType::Identifier => {
                    write!(f, "Identifier: {}", self.literal.as_ref().unwrap())
                }
                TokenType::String => {
                    write!(f, "String literal: {}", self.literal.as_ref().unwrap())
                }
                TokenType::Number => {
                    write!(f, "Number literal: {}", self.literal.as_ref().unwrap())
                }
            }
        }
    }

    pub struct Scanner {
        input: Vec<char>,
        pos: usize,
    }

    impl Scanner {
        pub fn scan(condition: String) -> Result<Vec<Token>, SyntaxError> {
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

        fn next_token(&mut self) -> Result<Option<Token>, SyntaxError> {
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
                        Err(SyntaxError {
                            pos: self.pos,
                            error: ErrorType::InvalidCharacter(self.input[self.pos - 1]),
                        })
                    }
                }
                '!' => {
                    if self.next_matches('=') {
                        Ok(Some(Token {
                            token_type: TokenType::NotEqual,
                            literal: None,
                            pos: self.pos - 1,
                        }))
                    } else {
                        Err(SyntaxError {
                            pos: self.pos,
                            error: ErrorType::InvalidCharacter(self.input[self.pos - 1]),
                        })
                    }
                }
                '"' | '\'' => self.scan_string(character).map(Some),
                ' ' | '\n' | '\r' | '\t' => Ok(None),
                _ => {
                    if character.is_alphabetic() {
                        self.scan_identifier().map(Some)
                    } else if character.is_numeric() {
                        self.scan_number().map(Some)
                    } else {
                        Err(SyntaxError {
                            pos: self.pos,
                            error: ErrorType::InvalidCharacter(character),
                        })
                    }
                }
            }
        }

        fn scan_identifier(&mut self) -> Result<Token, SyntaxError> {
            let start = self.pos;
            while !self.done() && self.valid_id_char() {
                self.advance();
            }
            Ok(Token {
                token_type: TokenType::Identifier,
                literal: Some(Literal::Identifier(
                    self.input[start - 1..self.pos].iter().collect(),
                )),
                pos: start,
            })
        }

        fn valid_id_char(&mut self) -> bool {
            let char = self.input[self.pos];
            char.is_alphanumeric() || char == '.' || char == '_'
        }

        fn scan_string(&mut self, until: char) -> Result<Token, SyntaxError> {
            let start = self.pos;
            loop {
                if self.done() {
                    return Err(SyntaxError {
                        pos: start,
                        error: ErrorType::UnclosedStringLiteral(until),
                    });
                }
                if self.advance() == until {
                    return Ok(Token {
                        token_type: TokenType::String,
                        literal: Some(Literal::String(
                            self.input[start..self.pos - 1].iter().collect(),
                        )),
                        pos: start,
                    });
                }
            }
        }

        fn scan_number(&mut self) -> Result<Token, SyntaxError> {
            let start = self.pos;
            while !self.done() && self.input[self.pos].is_numeric() {
                self.advance();
            }
            let number_str = self.input[start - 1..self.pos].iter().collect::<String>();
            match number_str.parse::<i64>() {
                Ok(number) => Ok(Token {
                    token_type: TokenType::Number,
                    literal: Some(Literal::Number(number)),
                    pos: start,
                }),
                Err(err) => {
                    let syntax_error = match err.kind() {
                        IntErrorKind::Empty => {
                            unreachable!("This means a bug in the scanner!")
                        }
                        IntErrorKind::Zero => {
                            unreachable!("We're parsing Numbers as i64, so 0 should always work!")
                        }
                        _ => SyntaxError {
                            pos: start,
                            error: ErrorType::InvalidNumber,
                        },
                    };
                    Err(syntax_error)
                }
            }
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
        use crate::limit::conditions::{ErrorType, Literal, Scanner, Token, TokenType};

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
        fn test_number_literal() {
            let tokens = Scanner::scan("var == 42".to_owned()).expect("Should parse alright!");
            assert_eq!(tokens.len(), 3);
            assert_eq!(
                tokens[0],
                Token {
                    token_type: TokenType::Identifier,
                    literal: Some(Identifier("var".to_owned())),
                    pos: 1,
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
                    token_type: TokenType::Number,
                    literal: Some(Literal::Number(42)),
                    pos: 8,
                }
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

        #[test]
        fn unclosed_string_literal() {
            let error = Scanner::scan("foo == 'ba".to_owned()).expect_err("Should fail!");
            assert_eq!(error.pos, 8);
            assert_eq!(error.error, ErrorType::UnclosedStringLiteral('\''));
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
                var_name: "x".to_string(),
                predicate: Predicate::Equal,
                operand: "5".to_string(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar=='ok' ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "foobar".to_string(),
                predicate: Predicate::Equal,
                operand: "ok".to_string(),
            }
        );

        let result: Condition =
            serde_json::from_str(r#""  foobar  ==   'ok'  ""#).expect("Should deserialize");
        assert_eq!(
            result,
            Condition {
                var_name: "foobar".to_string(),
                predicate: Predicate::Equal,
                operand: "ok".to_string(),
            }
        );
    }

    #[test]
    #[cfg(not(feature = "lenient_conditions"))]
    fn invalid_deprecated_condition_parsing() {
        let _result = serde_json::from_str::<Condition>(r#""x == 5""#)
            .err()
            .expect("Should fail!");
    }

    #[test]
    fn invalid_condition_parsing() {
        let result = serde_json::from_str::<Condition>(r#""x != 5 && x > 12""#)
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
            var_name: "foobar".to_string(),
            predicate: Predicate::Equal,
            operand: "ok".to_string(),
        };
        let result = serde_json::to_string(&condition).expect("Should serialize");
        assert_eq!(result, r#""foobar == \"ok\"""#.to_string());
    }
}
