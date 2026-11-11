//! Keeps `chunk_vectors` in step with `chunks`: embeds what has no vector for the current model.

use crate::embedder::Embedder;
use crate::error::{MemoryError, Result};
use crate::models::constants::embedding::{EMBED_BATCH, EMBED_MAX_CHARS};
use crate::models::enums::Source;
use crate::vectors::encode_vector;
use rusqlite::{params, Connection, Transaction};

const TITLE_SEPARATOR: char = '\n';

/// Text sent to the model for a chunk: its title then its text, cut to the model limit.
pub fn embedding_text(title: &str, text: &str) -> String {
    format!("{title}{TITLE_SEPARATOR}{text}")
        .chars()
        .take(EMBED_MAX_CHARS)
        .collect()
}

/// Embeds, by batches, the chunks of `project` (of one `source`, or all) whose vector is missing or comes from another model.
pub fn embed_pending(
    transaction: &Transaction,
    embedder: &dyn Embedder,
    project: &str,
    source: Option<Source>,
) -> Result<usize> {
    let source_filter = source.map(|source| source.as_str());
    let mut embedded = 0;
    loop {
        let batch = pending_batch(transaction, embedder, project, source_filter)?;
        if batch.is_empty() {
            return Ok(embedded);
        }
        let texts: Vec<&str> = batch.iter().map(|(_, text)| text.as_str()).collect();
        let vectors = embedder.embed(&texts)?;
        if vectors.len() != batch.len() {
            return Err(MemoryError::Embedding(format!(
                "model returned {} vectors for {} texts",
                vectors.len(),
                batch.len()
            )));
        }
        let mut statement = transaction.prepare_cached(
            "INSERT OR REPLACE INTO chunk_vectors (chunk_id, model, dim, vector) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for ((chunk_id, _), vector) in batch.iter().zip(&vectors) {
            statement.execute(params![
                chunk_id,
                embedder.model_id(),
                vector.len() as i64,
                encode_vector(vector)
            ])?;
        }
        embedded += batch.len();
    }
}

fn pending_batch(
    transaction: &Transaction,
    embedder: &dyn Embedder,
    project: &str,
    source: Option<&str>,
) -> Result<Vec<(i64, String)>> {
    let mut statement = transaction.prepare_cached(
        "SELECT c.rowid, c.title, c.text FROM chunks c \
         LEFT JOIN chunk_vectors v ON v.chunk_id = c.rowid \
         WHERE c.project = ?1 AND (?2 IS NULL OR c.source = ?2) \
         AND (v.chunk_id IS NULL OR v.model != ?3 OR v.dim != ?4) \
         ORDER BY c.rowid LIMIT ?5",
    )?;
    let rows = statement.query_map(
        params![
            project,
            source,
            embedder.model_id(),
            embedder.dim() as i64,
            EMBED_BATCH as i64
        ],
        |row| {
            let title: String = row.get(1)?;
            let text: String = row.get(2)?;
            Ok((row.get::<_, i64>(0)?, embedding_text(&title, &text)))
        },
    )?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Vectors stored for `model` and the model-wide chunk count, over the whole database.
pub fn vector_counts(connection: &Connection, model: &str) -> Result<(usize, usize, usize)> {
    let embedded: usize = connection.query_row(
        "SELECT COUNT(*) FROM chunk_vectors WHERE model = ?1",
        params![model],
        |row| row.get(0),
    )?;
    let other: usize = connection.query_row(
        "SELECT COUNT(*) FROM chunk_vectors WHERE model != ?1",
        params![model],
        |row| row.get(0),
    )?;
    let chunks: usize =
        connection.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
    Ok((embedded, other, chunks))
}

#[cfg(test)]
#[path = "../tests/unit/embedding_index_tests.rs"]
mod tests;
