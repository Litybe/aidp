//! Measures the retrieval on cases whose answers are known: recall at `k` and mean reciprocal rank.

use crate::error::{MemoryError, Result};
use crate::manager::MemoryManager;
use crate::models::constants::schema::MEMORY_PATH_PREFIX;
use crate::models::context_request::ContextRequest;
use crate::models::enums::Source;
use crate::models::eval_case::EvalCase;
use crate::models::eval_miss::EvalMiss;
use crate::models::eval_report::EvalReport;
use crate::models::search_query::SearchQuery;
use crate::retriever::Retriever;

/// Ranks searched per case to compute the reciprocal rank beyond `k`.
const EVAL_DEPTH: usize = 20;
/// Hits listed for a missed case.
const MISS_TOP: usize = 5;

/// Parses a YAML list of cases.
pub fn parse_cases(content: &str) -> Result<Vec<EvalCase>> {
    serde_yaml::from_str(content)
        .map_err(|error| MemoryError::Invalid(format!("invalid eval cases: {error}")))
}

/// Runs every case: a query against the retriever, an issue through `memory context`.
pub fn evaluate(
    manager: &MemoryManager<'_>,
    retriever: &dyn Retriever,
    project: &str,
    cases: &[EvalCase],
    k: usize,
) -> Result<EvalReport> {
    let mut answered = 0;
    let mut reciprocal = 0.0;
    let mut misses = Vec::new();
    for case in cases {
        let mut query = SearchQuery::new(project, case.query.clone());
        query.k = EVAL_DEPTH.max(k);
        query.sources = match case.sources.as_deref() {
            Some(list) => Source::parse_list(list)?,
            None => Vec::new(),
        };
        let paths: Vec<String> = if case.issue.is_some() {
            context_paths(manager, retriever, project, case)?
        } else {
            retriever
                .search(&query)?
                .into_iter()
                .map(|hit| hit.path)
                .collect()
        };
        let rank = paths.iter().position(|path| {
            case.expect
                .iter()
                .any(|prefix| path.starts_with(prefix.as_str()))
        });
        if let Some(position) = rank {
            reciprocal += 1.0 / (position + 1) as f64;
        }
        if rank.is_some_and(|position| position < k) {
            answered += 1;
        } else {
            misses.push(EvalMiss {
                query: case.issue.clone().unwrap_or_else(|| case.query.clone()),
                expect: case.expect.clone(),
                rank: rank.map(|position| position + 1),
                top: paths.into_iter().take(MISS_TOP).collect(),
            });
        }
    }
    let total = cases.len().max(1) as f64;
    Ok(EvalReport {
        cases: cases.len(),
        k,
        recall: answered as f64 / total,
        mrr: reciprocal / total,
        misses,
    })
}

/// The memories then the excerpts of the block a stage would receive for the issue.
fn context_paths(
    manager: &MemoryManager<'_>,
    retriever: &dyn Retriever,
    project: &str,
    case: &EvalCase,
) -> Result<Vec<String>> {
    let mut request = ContextRequest::new(project);
    request.issue = case.issue.clone();
    request.issue_text = Some(case.query.clone());
    request.route = case.issue_route;
    request.record_use = false;
    let block = manager.context(&request, retriever)?;
    Ok(block
        .memories
        .iter()
        .map(|memory| format!("{MEMORY_PATH_PREFIX}{}", memory.id))
        .chain(block.excerpts.into_iter().map(|hit| hit.path))
        .collect())
}

#[cfg(test)]
#[path = "../tests/unit/eval_tests.rs"]
mod tests;
