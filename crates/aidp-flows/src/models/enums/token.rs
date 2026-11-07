#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Str(String),
    Num(f64),
    Ident(String),
    LParen,
    RParen,
    Bang,
    EqEq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    AndAnd,
    OrOr,
}
