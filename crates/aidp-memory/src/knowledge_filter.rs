//! Domain and agent filters and the importance boost of knowledge chunks.

use crate::agent_roles::agent_name;
use crate::error::Result;
use crate::models::chunk_meta::ChunkMeta;
use crate::models::constants::knowledge::IMPORTANCE_BOOST;
use crate::models::enums::{Domain, Source};
use crate::models::hit::Hit;
use crate::models::search_query::SearchQuery;
use crate::store::MemoryStore;
use std::collections::HashMap;

/// Whether a knowledge chunk reaches this domain list and this agent type.
pub fn passes(meta: &ChunkMeta, domains: &[Domain], agent_type: Option<&str>) -> bool {
    let domain_ok = domains.is_empty()
        || meta
            .domain
            .as_deref()
            .is_some_and(|domain| domains.iter().any(|wanted| wanted.as_str() == domain));
    let agent_ok = match agent_type {
        None => true,
        Some(agent) => {
            let wanted = agent_name(agent);
            meta.agents.is_empty() || meta.agents.iter().any(|name| agent_name(name) == wanted)
        }
    };
    domain_ok && agent_ok
}

/// Score multiplier of a note: `1 + IMPORTANCE_BOOST * importance`.
pub fn importance_factor(importance: Option<f64>) -> f64 {
    1.0 + IMPORTANCE_BOOST * importance.unwrap_or(0.0).clamp(0.0, 1.0)
}

fn knowledge_meta(store: &MemoryStore, hits: &[Hit]) -> Result<HashMap<String, ChunkMeta>> {
    let ids: Vec<&str> = hits
        .iter()
        .filter(|hit| hit.source == Source::Knowledge)
        .map(|hit| hit.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let marks = vec!["?"; ids.len()].join(", ");
    store.read(|connection| {
        let mut statement = connection.prepare(&format!(
            "SELECT id, meta FROM chunks WHERE id IN ({marks})"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut found = HashMap::new();
        for row in rows {
            let (id, raw) = row?;
            found.insert(id, serde_json::from_str(&raw).unwrap_or_default());
        }
        Ok(found)
    })
}

/// Drops the knowledge hits outside the query's domains or agent type; other sources pass.
pub(crate) fn retain_allowed(
    store: &MemoryStore,
    query: &SearchQuery,
    hits: Vec<Hit>,
) -> Result<Vec<Hit>> {
    if !query.filters_knowledge() {
        return Ok(hits);
    }
    let metas = knowledge_meta(store, &hits)?;
    let agent = query.agent_type.as_deref();
    Ok(hits
        .into_iter()
        .filter(|hit| {
            hit.source != Source::Knowledge
                || metas
                    .get(&hit.id)
                    .is_some_and(|meta| passes(meta, &query.domains, agent))
        })
        .collect())
}

/// Multiplies the score of each knowledge hit by its importance factor and re-sorts.
pub(crate) fn boost_importance(store: &MemoryStore, hits: &mut [Hit]) -> Result<()> {
    let metas = knowledge_meta(store, hits)?;
    if metas.is_empty() {
        return Ok(());
    }
    for hit in hits.iter_mut() {
        if let Some(meta) = metas.get(&hit.id) {
            hit.score *= importance_factor(meta.importance);
        }
    }
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/knowledge_filter_tests.rs"]
mod tests;
