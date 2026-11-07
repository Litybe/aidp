//! The variables of a GraphQL mutation that takes a single `input`.

use serde::Serialize;

/// `{ "input": ... }`.
#[derive(Debug, Clone, Serialize)]
pub struct GraphqlInput<I: Serialize> {
    pub input: I,
}
