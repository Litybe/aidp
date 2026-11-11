//! Vector helpers: the stored blob format and cosine similarity.

use crate::error::{MemoryError, Result};
use crate::models::constants::embedding::BYTES_PER_COMPONENT;

/// Little-endian f32 bytes of a vector, as stored in `chunk_vectors.vector`.
pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * BYTES_PER_COMPONENT);
    for component in vector {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    bytes
}

/// Inverse of [`encode_vector`]; a blob that is not a whole number of f32 is refused.
pub fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(BYTES_PER_COMPONENT) {
        return Err(MemoryError::Invalid(format!(
            "stored vector of {} bytes is not a list of f32",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(BYTES_PER_COMPONENT)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

/// Cosine similarity of two vectors; 0 when their sizes differ or one is null.
pub fn cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (a, b) in left.iter().zip(right) {
        dot += f64::from(*a) * f64::from(*b);
        left_norm += f64::from(*a) * f64::from(*a);
        right_norm += f64::from(*b) * f64::from(*b);
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

#[cfg(test)]
#[path = "../tests/unit/vectors_tests.rs"]
mod tests;
