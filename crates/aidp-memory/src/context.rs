//! `memory context`: the Markdown block of active memories and excerpts for a stage.

use crate::error::Result;
use crate::manager::{load, parse_id, row_to_memory, save, MemoryManager, MEMORY_COLUMNS};
use crate::models::constants::limits::{MAX_CONTEXT_MEMORIES, MEMORY_BUDGET_SHARE};
use crate::models::constants::schema::MEMORY_WEIGHTS;
use crate::models::context_block::ContextBlock;
use crate::models::context_request::ContextRequest;
use crate::models::enums::{MemoryStatus, Source};
use crate::models::memory::Memory;
use crate::models::search_query::SearchQuery;
use crate::pack::{pack, render_markdown};
use crate::retriever::Retriever;
use crate::text::{estimate_tokens, match_expression};
use regex::Regex;
use rusqlite::params;
use std::collections::HashMap;
use std::sync::LazyLock;

const HEADING_MEMORIES: &str = "## Memoire du projet";
const HEADING_EXCERPTS: &str = "## Extraits pertinents";
const EXCERPT_K: usize = 12;
const DEFAULT_SOURCES: [Source; 5] = [
    Source::Bible,
    Source::Doc,
    Source::Code,
    Source::Knowledge,
    Source::Run,
];

static PATH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:Assets|Docs|\.claude)/[\w./-]+").expect("path pattern"));

impl MemoryManager<'_> {
    /// Builds the context block and records the injected memories as used.
    pub fn context(
        &self,
        request: &ContextRequest,
        retriever: &dyn Retriever,
    ) -> Result<ContextBlock> {
        let query_text = [
            request.issue_text.as_deref(),
            request.stage.as_deref(),
            request.route.map(|route| route.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        let memory_budget = (request.budget_tokens as f64 * MEMORY_BUDGET_SHARE) as usize;
        let mut memories = self.relevant_memories(request, &query_text)?;
        let mut used = estimate_tokens(HEADING_MEMORIES);
        let mut kept: Vec<Memory> = Vec::new();
        for memory in memories.drain(..) {
            let cost = estimate_tokens(&memory_line(&memory));
            if used + cost > memory_budget {
                continue;
            }
            used += cost;
            kept.push(memory);
        }
        self.mark_used(&kept)?;
        let mut markdown = String::new();
        if !kept.is_empty() {
            markdown.push_str(&format!("{HEADING_MEMORIES} {}\n\n", request.project));
            for memory in &kept {
                markdown.push_str(&memory_line(memory));
            }
        } else {
            used = 0;
        }
        let mut excerpts = Vec::new();
        let remaining = request
            .budget_tokens
            .saturating_sub(used + estimate_tokens(HEADING_EXCERPTS));
        if request
            .issue_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
            || request.stage.is_some()
        {
            let mut query = SearchQuery::new(request.project.clone(), query_text.clone());
            query.k = EXCERPT_K;
            query.sources = match request.route {
                Some(route) => route.sources().to_vec(),
                None => DEFAULT_SOURCES.to_vec(),
            };
            let packed = pack(retriever.search(&query)?, remaining);
            if !packed.hits.is_empty() {
                if !markdown.is_empty() {
                    markdown.push('\n');
                }
                markdown.push_str(HEADING_EXCERPTS);
                markdown.push_str("\n\n");
                markdown.push_str(&render_markdown(&packed));
                used += packed.used_tokens + estimate_tokens(HEADING_EXCERPTS);
                excerpts = packed.hits;
            }
        }
        Ok(ContextBlock {
            markdown,
            memories: kept,
            excerpts,
            used_tokens: used,
            budget_tokens: request.budget_tokens,
        })
    }

    fn relevant_memories(&self, request: &ContextRequest, query_text: &str) -> Result<Vec<Memory>> {
        let scores = self.relevance(&request.project, query_text)?;
        let paths: Vec<String> = request
            .issue_text
            .as_deref()
            .map(extract_paths)
            .unwrap_or_default();
        let active = self.list(
            &request.project,
            &crate::models::memory_filter::MemoryFilter {
                status: Some(MemoryStatus::Active),
                ..Default::default()
            },
        )?;
        let mut scored: Vec<(f64, Memory)> = active
            .into_iter()
            .filter_map(|memory| {
                let score = scores.get(&memory.id).copied().unwrap_or(0.0);
                if let (Some(wanted), Some(offered)) = (memory.scope.route, request.route) {
                    if wanted != offered {
                        return None;
                    }
                }
                if !memory.scope.paths.is_empty() {
                    let applies = if paths.is_empty() {
                        score > 0.0
                    } else {
                        paths.iter().any(|path| {
                            memory.scope.paths.iter().any(|pattern| {
                                glob::Pattern::new(pattern).is_ok_and(|glob| glob.matches(path))
                            })
                        })
                    };
                    if !applies {
                        return None;
                    }
                }
                Some((score, memory))
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then(right.1.confidence.total_cmp(&left.1.confidence))
                .then(right.1.evidence.cmp(&left.1.evidence))
        });
        Ok(scored
            .into_iter()
            .take(MAX_CONTEXT_MEMORIES)
            .map(|(_, memory)| memory)
            .collect())
    }

    fn relevance(&self, project: &str, query_text: &str) -> Result<HashMap<String, f64>> {
        let Some(expression) = match_expression(query_text) else {
            return Ok(HashMap::new());
        };
        self.store.read(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {MEMORY_COLUMNS}, bm25(memories_fts, {MEMORY_WEIGHTS}) FROM memories_fts \
                 JOIN memories m ON m.num = memories_fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.project = ?2 AND m.status = 'active'"
            ))?;
            let rows = statement.query_map(params![expression, project], |row| {
                Ok((row_to_memory(row)?.id, -row.get::<_, f64>(15)?))
            })?;
            Ok(rows.collect::<std::result::Result<HashMap<_, _>, _>>()?)
        })
    }

    fn mark_used(&self, memories: &[Memory]) -> Result<()> {
        if memories.is_empty() {
            return Ok(());
        }
        let now = self.now();
        self.store.write(|connection| {
            for memory in memories {
                let mut current = load(connection, &memory.project, parse_id(&memory.id)?)?;
                current.uses += 1;
                current.last_used_at = Some(now.clone());
                save(connection, &current)?;
            }
            Ok(())
        })
    }
}

fn memory_line(memory: &Memory) -> String {
    format!(
        "- **{}** ({}) {}: {}\n",
        memory.id, memory.kind, memory.title, memory.text
    )
}

/// Project paths (`Assets/...`, `Docs/...`) mentioned in a text.
pub fn extract_paths(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = PATH_PATTERN
        .find_iter(text)
        .map(|found| found.as_str().trim_end_matches(['.', ',']).to_string())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
#[path = "../tests/unit/context_tests.rs"]
mod tests;
