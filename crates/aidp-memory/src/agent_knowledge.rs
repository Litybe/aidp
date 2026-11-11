//! The knowledge block a sub-agent receives when it starts.

use crate::agent_roles::{agent_name, domains_for};
use crate::error::Result;
use crate::models::constants::knowledge::{
    DOMAIN_WORD_SEPARATOR, HOOK_K, HOOK_QUERY_MAX_CHARS, KNOWLEDGE_HEADING,
};
use crate::models::enums::{Domain, Source};
use crate::models::hit::Hit;
use crate::models::search_query::SearchQuery;
use crate::retriever::Retriever;
use crate::text::truncate_chars;

/// The search words of an agent type that has no prompt: its domains, else its own name.
pub fn role_query(agent_type: &str, domains: &[Domain]) -> String {
    if domains.is_empty() {
        return agent_name(agent_type).replace(DOMAIN_WORD_SEPARATOR, " ");
    }
    domains
        .iter()
        .map(|domain| domain.as_str().replace(DOMAIN_WORD_SEPARATOR, " "))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The knowledge block for an agent type, at most `max_chars` characters; empty when nothing matches.
///
/// The query is the prompt when there is one, else the words of the agent's domains.
pub fn agent_knowledge(
    retriever: &dyn Retriever,
    project: &str,
    agent_type: &str,
    prompt: Option<&str>,
    max_chars: usize,
) -> Result<String> {
    let domains = domains_for(agent_type);
    let text = match prompt.map(str::trim).filter(|prompt| !prompt.is_empty()) {
        Some(prompt) => truncate_chars(prompt, HOOK_QUERY_MAX_CHARS).to_string(),
        None => role_query(agent_type, &domains),
    };
    let mut query = SearchQuery::new(project, text);
    query.sources = vec![Source::Knowledge];
    query.k = HOOK_K;
    query.domains = domains;
    query.agent_type = Some(agent_type.to_string());
    let hits = retriever.search(&query)?;
    Ok(render(&hits, agent_type, max_chars))
}

fn entry(hit: &Hit) -> String {
    format!(
        "### {} (`{}`)\n{}\n\n",
        hit.title,
        hit.citation(),
        hit.text.trim()
    )
}

/// The heading, the lookup hint and as many entries as fit `max_chars`; empty without entry.
pub fn render(hits: &[Hit], agent_type: &str, max_chars: usize) -> String {
    let header = format!(
        "{KNOWLEDGE_HEADING}\n\nNotes for `{}`. More: `aidp rag search \"<query>\" --agent-type {}`.\n\n",
        agent_name(agent_type),
        agent_name(agent_type)
    );
    let mut used = header.chars().count();
    let mut body = String::new();
    for hit in hits {
        let mut text = entry(hit);
        let room = max_chars.saturating_sub(used);
        if text.chars().count() > room {
            if !body.is_empty() {
                continue;
            }
            text = truncate_chars(&text, room).to_string();
        }
        used += text.chars().count();
        body.push_str(&text);
    }
    if body.trim().is_empty() {
        return String::new();
    }
    format!("{header}{}", body.trim_end())
}

#[cfg(test)]
#[path = "../tests/unit/agent_knowledge_tests.rs"]
mod tests;
