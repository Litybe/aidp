//! Retrieval: the `Retriever` trait and its BM25 plus code-graph implementation.

use crate::error::Result;
use crate::graph::CodeGraph;
use crate::mmr;
use crate::models::constants::limits::{
    CANDIDATE_FACTOR, FILTERED_CANDIDATES, GRAPH_DAMPING, GRAPH_MAX_ADDED,
    GRAPH_NEIGHBOURS_PER_SEED, GRAPH_SEED_HITS, JOURNAL_SEGMENT, JOURNAL_WEIGHT, MIN_CANDIDATES,
    MMR_LAMBDA, SOURCE_WEIGHTS,
};
use crate::models::enums::Source;
use crate::models::hit::Hit;
use crate::models::search_query::SearchQuery;
use crate::store::MemoryStore;
use crate::text::{match_expression, query_terms};
use rusqlite::{Connection, Row};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) const VIA_BM25: &str = "bm25";
const VIA_GRAPH: &str = "graph";
const NAME_MATCH_SEEDS: usize = 3;
const NAME_MATCH_SHARE: f64 = 0.6;
pub(crate) const CHUNK_COLUMNS: &str =
    "c.id, c.source, c.path, c.line_start, c.line_end, c.title, c.text";

/// Anything that ranks indexed chunks for a query; a vector index can implement it later.
pub trait Retriever {
    /// Ranked, de-duplicated hits for the query, best first, at most `query.k`.
    fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>>;
}

/// Full-text BM25 retrieval over the store, widened by the graphify graph.
pub struct Bm25Retriever<'a> {
    store: &'a MemoryStore,
    project_root: Option<PathBuf>,
    graph: OnceLock<Option<CodeGraph>>,
}

impl<'a> Bm25Retriever<'a> {
    /// `project_root` locates `graphify-out/graph.json`; `None` disables the expansion.
    pub fn new(store: &'a MemoryStore, project_root: Option<PathBuf>) -> Self {
        Self {
            store,
            project_root,
            graph: OnceLock::new(),
        }
    }

    fn graph(&self) -> Option<&CodeGraph> {
        self.graph
            .get_or_init(|| self.project_root.as_deref().and_then(CodeGraph::load))
            .as_ref()
    }

    fn candidates(&self, query: &SearchQuery, expression: &str, limit: usize) -> Result<Vec<Hit>> {
        let filter = if query.sources.is_empty() {
            String::new()
        } else {
            let list = query
                .sources
                .iter()
                .map(|source| format!("'{}'", source.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("AND c.source IN ({list}) ")
        };
        let sql = format!(
            "SELECT {CHUNK_COLUMNS}, bm25(chunks_fts, {weights}) AS rank \
             FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid \
             WHERE chunks_fts MATCH ?1 AND c.project = ?2 {filter}\
             ORDER BY rank LIMIT ?3",
            weights = crate::models::constants::schema::CHUNK_WEIGHTS
        );
        self.store.read(|connection| {
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(
                rusqlite::params![expression, query.project, limit as i64],
                |row| {
                    let mut hit = hit_from_row(row, VIA_BM25)?;
                    hit.score = -row.get::<_, f64>(7)? * source_weight(&hit);
                    Ok(hit)
                },
            )?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    fn chunk_at(
        &self,
        connection: &Connection,
        project: &str,
        file: &str,
        line: u32,
    ) -> Option<Hit> {
        connection
            .query_row(
                &format!(
                    "SELECT {CHUNK_COLUMNS} FROM chunks c WHERE c.project = ?1 AND c.path = ?2 \
                     AND c.source = 'code' AND c.line_start <= ?3 AND c.line_end >= ?3 \
                     ORDER BY c.line_end - c.line_start LIMIT 1"
                ),
                rusqlite::params![project, file, line],
                |row| hit_from_row(row, VIA_GRAPH),
            )
            .ok()
    }

    fn expand(&self, query: &SearchQuery, hits: &[Hit]) -> Result<Vec<Hit>> {
        let wants_code = query.sources.is_empty() || query.sources.contains(&Source::Code);
        let Some(graph) = self.graph().filter(|_| wants_code) else {
            return Ok(Vec::new());
        };
        let best = hits.iter().map(|hit| hit.score).fold(0.0, f64::max);
        let mut seeds: Vec<(usize, f64)> = Vec::new();
        for hit in hits
            .iter()
            .filter(|hit| hit.source == Source::Code)
            .take(GRAPH_SEED_HITS)
        {
            for position in graph.symbols_in(&hit.path, hit.line_start, hit.line_end) {
                seeds.push((position, hit.score));
            }
        }
        let mut added: HashMap<String, Hit> = HashMap::new();
        self.store.read(|connection| {
            for term in query_terms(&query.text) {
                for position in graph
                    .symbols_named(&term)
                    .into_iter()
                    .take(NAME_MATCH_SEEDS)
                {
                    seeds.push((position, best));
                    if let Some(symbol) = graph.symbol(position) {
                        if let Some(mut hit) =
                            self.chunk_at(connection, &query.project, &symbol.file, symbol.line)
                        {
                            hit.score = best * NAME_MATCH_SHARE;
                            keep_best(&mut added, hit);
                        }
                    }
                }
            }
            for (position, score) in &seeds {
                for neighbour in graph
                    .neighbours_of(*position)
                    .into_iter()
                    .take(GRAPH_NEIGHBOURS_PER_SEED)
                {
                    let Some(symbol) = graph.symbol(neighbour) else {
                        continue;
                    };
                    if let Some(mut hit) =
                        self.chunk_at(connection, &query.project, &symbol.file, symbol.line)
                    {
                        hit.score = score * GRAPH_DAMPING;
                        keep_best(&mut added, hit);
                    }
                }
            }
            Ok(())
        })?;
        let mut extra: Vec<Hit> = added
            .into_values()
            .filter(|hit| !hits.iter().any(|existing| existing.id == hit.id))
            .collect();
        extra.sort_by(|left, right| right.score.total_cmp(&left.score));
        extra.truncate(GRAPH_MAX_ADDED);
        Ok(extra)
    }
}

impl Bm25Retriever<'_> {
    /// Every BM25 and graph candidate matching the filters, best first, before the MMR pass.
    pub(crate) fn ranked(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        let Some(expression) = match_expression(&query.text) else {
            return Ok(Vec::new());
        };
        let pattern = path_pattern(query)?;
        let limit = if pattern.is_some() {
            FILTERED_CANDIDATES
        } else {
            (query.k * CANDIDATE_FACTOR).max(MIN_CANDIDATES)
        };
        let mut hits = self.candidates(query, &expression, limit)?;
        hits.extend(self.expand(query, &hits)?);
        if let Some(pattern) = &pattern {
            hits.retain(|hit| pattern.matches(&hit.path));
        }
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        Ok(hits)
    }
}

impl Retriever for Bm25Retriever<'_> {
    fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        Ok(mmr::select(self.ranked(query)?, query.k, MMR_LAMBDA))
    }
}

/// The compiled `--path` glob of a query, if any.
pub(crate) fn path_pattern(query: &SearchQuery) -> Result<Option<glob::Pattern>> {
    match &query.path_glob {
        Some(glob) => Ok(Some(glob::Pattern::new(glob).map_err(|error| {
            crate::error::MemoryError::Invalid(format!("invalid --path glob: {error}"))
        })?)),
        None => Ok(None),
    }
}

/// How much a hit of this source and path counts against the others.
pub(crate) fn source_weight(hit: &Hit) -> f64 {
    let weight = SOURCE_WEIGHTS
        .iter()
        .find(|(source, _)| *source == hit.source)
        .map_or(1.0, |(_, weight)| *weight);
    if hit.path.contains(JOURNAL_SEGMENT) {
        weight * JOURNAL_WEIGHT
    } else {
        weight
    }
}

fn keep_best(map: &mut HashMap<String, Hit>, hit: Hit) {
    match map.get(&hit.id) {
        Some(existing) if existing.score >= hit.score => {}
        _ => {
            map.insert(hit.id.clone(), hit);
        }
    }
}

pub(crate) fn hit_from_row(row: &Row, via: &str) -> rusqlite::Result<Hit> {
    let source: String = row.get(1)?;
    Ok(Hit {
        id: row.get(0)?,
        source: Source::parse(&source).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        path: row.get(2)?,
        line_start: row.get(3)?,
        line_end: row.get(4)?,
        title: row.get(5)?,
        text: row.get(6)?,
        score: 0.0,
        via: via.to_string(),
    })
}

#[cfg(test)]
#[path = "../tests/unit/retriever_tests.rs"]
mod tests;
