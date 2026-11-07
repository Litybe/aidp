use crate::error::FlowError;
use crate::models::enums::Token;
pub use crate::models::enums::{BinaryOp, Expr};
pub use crate::models::Context;
use serde_json::Value;
use std::collections::BTreeSet;

/// Root segment of a path that reads a step.
const STEPS_ROOT: &str = "steps";
const KEYWORD_TRUE: &str = "true";
const KEYWORD_FALSE: &str = "false";
const KEYWORD_NULL: &str = "null";

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_part(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@')
}

fn lex_string(chars: &[char], start: usize) -> Result<(Token, usize), FlowError> {
    let quote = chars[start];
    let mut text = String::new();
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == quote {
            return Ok((Token::Str(text), index + 1));
        }
        text.push(chars[index]);
        index += 1;
    }
    Err(FlowError::Parse("unterminated string literal".into()))
}

fn lex_number(chars: &[char], start: usize) -> Result<(Token, usize), FlowError> {
    let mut index = start + 1;
    while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
        index += 1;
    }
    let text: String = chars[start..index].iter().collect();
    text.parse::<f64>()
        .map(|number| (Token::Num(number), index))
        .map_err(|_| FlowError::Parse(format!("invalid number `{text}`")))
}

fn lex_operator(chars: &[char], index: usize) -> Result<(Token, usize), FlowError> {
    let current = chars[index];
    let next = chars.get(index + 1).copied();
    let token = match (current, next) {
        ('=', Some('=')) => (Token::EqEq, 2),
        ('!', Some('=')) => (Token::NotEq, 2),
        ('<', Some('=')) => (Token::Le, 2),
        ('>', Some('=')) => (Token::Ge, 2),
        ('&', Some('&')) => (Token::AndAnd, 2),
        ('|', Some('|')) => (Token::OrOr, 2),
        ('!', _) => (Token::Bang, 1),
        ('<', _) => (Token::Lt, 1),
        ('>', _) => (Token::Gt, 1),
        ('(', _) => (Token::LParen, 1),
        (')', _) => (Token::RParen, 1),
        _ => {
            return Err(FlowError::Parse(format!(
                "unexpected character `{current}`"
            )))
        }
    };
    Ok((token.0, index + token.1))
}

fn tokenize(source: &str) -> Result<Vec<Token>, FlowError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        let starts_negative_number =
            ch == '-' && chars.get(index + 1).is_some_and(char::is_ascii_digit);
        let (token, next) = if ch.is_whitespace() {
            index += 1;
            continue;
        } else if ch == '\'' || ch == '"' {
            lex_string(&chars, index)?
        } else if ch.is_ascii_digit() || starts_negative_number {
            lex_number(&chars, index)?
        } else if is_ident_start(ch) {
            let mut end = index + 1;
            while end < chars.len() && is_ident_part(chars[end]) {
                end += 1;
            }
            (Token::Ident(chars[index..end].iter().collect()), end)
        } else {
            lex_operator(&chars, index)?
        };
        tokens.push(token);
        index = next;
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn eat(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> Result<Expr, FlowError> {
        let mut left = self.parse_and()?;
        while self.eat(&Token::OrOr) {
            let right = self.parse_and()?;
            left = Expr::Binary(BinaryOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, FlowError> {
        let mut left = self.parse_equality()?;
        while self.eat(&Token::AndAnd) {
            let right = self.parse_equality()?;
            left = Expr::Binary(BinaryOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, FlowError> {
        let mut left = self.parse_relational()?;
        loop {
            let operator = match self.peek() {
                Some(Token::EqEq) => BinaryOp::Eq,
                Some(Token::NotEq) => BinaryOp::Ne,
                _ => return Ok(left),
            };
            self.position += 1;
            let right = self.parse_relational()?;
            left = Expr::Binary(operator, Box::new(left), Box::new(right));
        }
    }

    fn parse_relational(&mut self) -> Result<Expr, FlowError> {
        let mut left = self.parse_unary()?;
        loop {
            let operator = match self.peek() {
                Some(Token::Lt) => BinaryOp::Lt,
                Some(Token::Gt) => BinaryOp::Gt,
                Some(Token::Le) => BinaryOp::Le,
                Some(Token::Ge) => BinaryOp::Ge,
                _ => return Ok(left),
            };
            self.position += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary(operator, Box::new(left), Box::new(right));
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, FlowError> {
        if self.eat(&Token::Bang) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, FlowError> {
        match self.advance() {
            Some(Token::Str(text)) => Ok(Expr::Literal(Value::String(text))),
            Some(Token::Num(number)) => Ok(Expr::Literal(number_value(number))),
            Some(Token::Ident(name)) => ident_expr(&name),
            Some(Token::LParen) => {
                let inner = self.parse_or()?;
                if self.eat(&Token::RParen) {
                    Ok(inner)
                } else {
                    Err(FlowError::Parse("missing closing parenthesis".into()))
                }
            }
            Some(other) => Err(FlowError::Parse(format!("unexpected token {other:?}"))),
            None => Err(FlowError::Parse("unexpected end of expression".into())),
        }
    }
}

fn number_value(number: f64) -> Value {
    if number.fract() == 0.0 && number.abs() < 9.0e15 {
        Value::from(number as i64)
    } else {
        Value::from(number)
    }
}

fn ident_expr(name: &str) -> Result<Expr, FlowError> {
    match name {
        KEYWORD_TRUE => return Ok(Expr::Literal(Value::Bool(true))),
        KEYWORD_FALSE => return Ok(Expr::Literal(Value::Bool(false))),
        KEYWORD_NULL => return Ok(Expr::Literal(Value::Null)),
        _ => {}
    }
    let segments: Vec<String> = name.split('.').map(str::to_string).collect();
    if segments.iter().any(String::is_empty) {
        return Err(FlowError::Parse(format!("empty path segment in `{name}`")));
    }
    Ok(Expr::Path(segments))
}

/// Parses an expression. The whole input must be consumed.
pub fn parse(source: &str) -> Result<Expr, FlowError> {
    let mut parser = Parser {
        tokens: tokenize(source)?,
        position: 0,
    };
    let expr = parser.parse_or()?;
    match parser.peek() {
        None => Ok(expr),
        Some(token) => Err(FlowError::Parse(format!("unexpected token {token:?}"))),
    }
}

/// Truthiness used by `!`, `&&`, `||` and conditions.
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

fn compare_ordering(operator: BinaryOp, left: &Value, right: &Value) -> Result<bool, FlowError> {
    if left.is_null() || right.is_null() {
        return Ok(false);
    }
    let ordering = match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64().partial_cmp(&b.as_f64()),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => return Err(FlowError::Eval(format!("cannot order {left} and {right}"))),
    };
    let Some(ordering) = ordering else {
        return Ok(false);
    };
    Ok(match operator {
        BinaryOp::Lt => ordering.is_lt(),
        BinaryOp::Gt => ordering.is_gt(),
        BinaryOp::Le => ordering.is_le(),
        _ => ordering.is_ge(),
    })
}

impl Expr {
    /// Evaluates the expression against `context`.
    pub fn eval(&self, context: &Context) -> Result<Value, FlowError> {
        match self {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::Path(segments) => Ok(context.lookup(segments)),
            Expr::Not(inner) => Ok(Value::Bool(!is_truthy(&inner.eval(context)?))),
            Expr::Binary(BinaryOp::And, left, right) => {
                let result = is_truthy(&left.eval(context)?) && is_truthy(&right.eval(context)?);
                Ok(Value::Bool(result))
            }
            Expr::Binary(BinaryOp::Or, left, right) => {
                let result = is_truthy(&left.eval(context)?) || is_truthy(&right.eval(context)?);
                Ok(Value::Bool(result))
            }
            Expr::Binary(operator, left, right) => {
                let (left, right) = (left.eval(context)?, right.eval(context)?);
                let result = match operator {
                    BinaryOp::Eq => values_equal(&left, &right),
                    BinaryOp::Ne => !values_equal(&left, &right),
                    ordering => compare_ordering(*ordering, &left, &right)?,
                };
                Ok(Value::Bool(result))
            }
        }
    }

    /// Evaluates the expression and reduces the result to a boolean.
    pub fn eval_bool(&self, context: &Context) -> Result<bool, FlowError> {
        Ok(is_truthy(&self.eval(context)?))
    }

    /// Ids of the steps this expression reads through `steps.<id>...`.
    pub fn step_refs(&self) -> Vec<String> {
        let mut refs = BTreeSet::new();
        self.collect_step_refs(&mut refs);
        refs.into_iter().collect()
    }

    fn collect_step_refs(&self, refs: &mut BTreeSet<String>) {
        match self {
            Expr::Literal(_) => {}
            Expr::Path(segments) => {
                if segments.len() >= 2 && segments[0] == STEPS_ROOT {
                    refs.insert(segments[1].clone());
                }
            }
            Expr::Not(inner) => inner.collect_step_refs(refs),
            Expr::Binary(_, left, right) => {
                left.collect_step_refs(refs);
                right.collect_step_refs(refs);
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/expr_tests.rs"]
mod tests;
