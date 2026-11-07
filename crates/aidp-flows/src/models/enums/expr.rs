use crate::models::enums::BinaryOp;
use serde_json::Value;

/// A parsed condition expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Path(Vec<String>),
    Not(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}
