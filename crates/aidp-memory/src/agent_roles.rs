//! Which knowledge domains an agent type reads.

use crate::error::Result;
use crate::models::agent_role::AgentRole;
use crate::models::constants::knowledge::{AGENT_PLUGIN_SEPARATOR, AGENT_ROLES};
use crate::models::enums::{Domain, Route};

/// The agent name without its plugin prefix (`plugin:name`), lowercased.
pub fn agent_name(agent_type: &str) -> String {
    agent_type
        .rsplit(AGENT_PLUGIN_SEPARATOR)
        .next()
        .unwrap_or(agent_type)
        .trim()
        .to_lowercase()
}

/// The role of a known agent type; `None` for an agent that sees every domain.
pub fn role_of(agent_type: &str) -> Option<&'static AgentRole> {
    let name = agent_name(agent_type);
    AGENT_ROLES
        .iter()
        .find(|role| role.names.contains(&name.as_str()))
}

/// The domains an agent reads; empty means every domain.
pub fn domains_for(agent_type: &str) -> Vec<Domain> {
    role_of(agent_type).map_or_else(Vec::new, |role| role.domains.to_vec())
}

/// The domains of an agent type; a generic or unknown agent gets the domains of `route` when known.
pub fn domains_for_route(agent_type: &str, route: Option<Route>) -> Vec<Domain> {
    let domains = domains_for(agent_type);
    if !domains.is_empty() {
        return domains;
    }
    route.map_or_else(Vec::new, |route| route.domains().to_vec())
}

/// `domains` unchanged when set or without an agent type, else the route domains of a generic agent.
pub fn narrow_by_route(
    domains: Vec<Domain>,
    agent_type: Option<&str>,
    route: Option<Route>,
) -> Vec<Domain> {
    match agent_type {
        Some(agent) if domains.is_empty() => domains_for_route(agent, route),
        _ => domains,
    }
}

/// The domains of an explicit list (names as spelled in `--domain`), else those of the agent type; empty means every domain.
pub fn parse_domains(raw: &[String], agent_type: Option<&str>) -> Result<Vec<Domain>> {
    let explicit = raw
        .iter()
        .map(|domain| Domain::parse(domain))
        .collect::<Result<Vec<_>>>()?;
    Ok(effective_domains(&explicit, agent_type))
}

/// Explicit domains win; otherwise the domains of the agent type, if one is given.
pub fn effective_domains(explicit: &[Domain], agent_type: Option<&str>) -> Vec<Domain> {
    if !explicit.is_empty() {
        return explicit.to_vec();
    }
    agent_type.map_or_else(Vec::new, domains_for)
}

#[cfg(test)]
#[path = "../tests/unit/agent_roles_tests.rs"]
mod tests;
