use crate::models::enums::{Domain, Route};

/// The knowledge domains and the kind of work of a group of agent types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRole {
    pub names: &'static [&'static str],
    /// Domains the agents read; empty means every domain.
    pub domains: &'static [Domain],
    pub route: Option<Route>,
}
