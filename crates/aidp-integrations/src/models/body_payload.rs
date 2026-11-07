use serde::Serialize;

/// Request body carrying only a comment text.
#[derive(Debug, Serialize)]
pub struct BodyPayload<'a> {
    pub body: &'a str,
}
