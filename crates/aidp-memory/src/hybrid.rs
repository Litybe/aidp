//! Hybrid retrieval: BM25 and graph candidates fused with cosine candidates by reciprocal rank.

use crate::embedder::Embedder;
use crate::error::Result;
use crate::mmr;
use crate::models::constants::embedding::{
    EMBED_MAX_CHARS, MIN_COSINE, RRF_K, VECTOR_CANDIDATE_FACTOR, VIA_HYBRID, VIA_VECTOR,
};
use crate::models::constants::limits::{FILTERED_CANDIDATES, MIN_CANDIDATES, MMR_LAMBDA};
use crate::models::hit::Hit;
use crate::models::search_query::SearchQuery;
use crate::retriever::{
    hit_from_row, path_pattern, source_weight, Bm25Retriever, Retriever, CHUNK_COLUMNS,
};
use crate::store::MemoryStore;
use crate::vectors::{cosine, decode_vector};
use std::collections::HashMap;
use std::path::PathBuf;

/// BM25 plus brute-force cosine over the stored vectors, fused by RRF, then weighted and diversified.
pub struct HybridRetriever<'a> {
    store: &'a MemoryStore,
    lexical: Bm25Retriever<'a>,
    embedder: Box<dyn Embedder>,
}

impl<'a> HybridRetriever<'a> {
    /// `project_root` locates the graphify graph, as for [`Bm25Retriever::new`].
    pub fn new(
        store: &'a MemoryStore,
        project_root: Option<PathBuf>,
        embedder: Box<dyn Embedder>,
    ) -> Self {
        Self {
            store,
            lexical: Bm25Retriever::new(store, project_root),
            embedder,
        }
    }

    fn vector_ranked(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        let text: String = query.text.chars().take(EMBED_MAX_CHARS).collect();
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let pattern = path_pattern(query)?;
        let limit = if pattern.is_some() {
            FILTERED_CANDIDATES
        } else {
            (query.k * VECTOR_CANDIDATE_FACTOR).max(MIN_CANDIDATES)
        };
        let query_vector = self
            .embedder
            .embed(&[text.as_str()])?
            .into_iter()
            .next()
            .unwrap_or_default();
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
            "SELECT v.chunk_id, v.vector FROM chunk_vectors v JOIN chunks c ON c.rowid = v.chunk_id \
             WHERE c.project = ?1 AND v.model = ?2 {filter}"
        );
        self.store.read(|connection| {
            let mut scored: Vec<(f64, i64)> = Vec::new();
            let mut statement = connection.prepare(&sql)?;
            let mut rows =
                statement.query(rusqlite::params![query.project, self.embedder.model_id()])?;
            while let Some(row) = rows.next()? {
                let stored = decode_vector(&row.get::<_, Vec<u8>>(1)?)?;
                let similarity = cosine(&query_vector, &stored);
                if similarity >= MIN_COSINE {
                    scored.push((similarity, row.get(0)?));
                    if scored.len() >= limit * 2 {
                        sort_best_first(&mut scored);
                        scored.truncate(limit);
                    }
                }
            }
            sort_best_first(&mut scored);
            scored.truncate(limit);
            let mut hits = Vec::with_capacity(scored.len());
            for (similarity, chunk_id) in scored {
                let hit = connection.query_row(
                    &format!("SELECT {CHUNK_COLUMNS} FROM chunks c WHERE c.rowid = ?1"),
                    [chunk_id],
                    |row| hit_from_row(row, VIA_VECTOR),
                );
                let Ok(mut hit) = hit else { continue };
                if pattern
                    .as_ref()
                    .is_some_and(|glob| !glob.matches(&hit.path))
                {
                    continue;
                }
                hit.score = similarity;
                hits.push(hit);
            }
            Ok(hits)
        })
    }
}

impl Retriever for HybridRetriever<'_> {
    fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        let lexical = self.lexical.ranked(query)?;
        let vector = match self.vector_ranked(query) {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!(%error, "vector search failed; using BM25 only");
                Vec::new()
            }
        };
        let mut fused = rrf_fuse(lexical, vector);
        for hit in &mut fused {
            hit.score *= source_weight(hit);
        }
        sort_hits(&mut fused);
        Ok(mmr::select(fused, query.k, MMR_LAMBDA))
    }
}

fn sort_best_first(scored: &mut [(f64, i64)]) {
    scored.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
}

fn sort_hits(hits: &mut [Hit]) {
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Reciprocal rank fusion of two ranked lists: each hit scores `1 / (RRF_K + rank)` per list it appears in.
pub fn rrf_fuse(lexical: Vec<Hit>, vector: Vec<Hit>) -> Vec<Hit> {
    let mut fused: HashMap<String, Hit> = HashMap::new();
    for (rank, mut hit) in lexical.into_iter().enumerate() {
        hit.score = 1.0 / (RRF_K + (rank + 1) as f64);
        fused.insert(hit.id.clone(), hit);
    }
    for (rank, hit) in vector.into_iter().enumerate() {
        let share = 1.0 / (RRF_K + (rank + 1) as f64);
        match fused.get_mut(&hit.id) {
            Some(existing) => {
                existing.score += share;
                existing.via = VIA_HYBRID.to_string();
            }
            None => {
                let mut hit = hit;
                hit.score = share;
                fused.insert(hit.id.clone(), hit);
            }
        }
    }
    let mut hits: Vec<Hit> = fused.into_values().collect();
    sort_hits(&mut hits);
    hits
}

#[cfg(test)]
#[path = "../tests/unit/hybrid_tests.rs"]
mod tests;
