//! The body of a GitHub GraphQL call.

use serde::Serialize;

/// One GraphQL document and its variables.
#[derive(Debug, Clone, Serialize)]
pub struct GraphqlRequest<V: Serialize> {
    pub query: &'static str,
    pub variables: V,
}
