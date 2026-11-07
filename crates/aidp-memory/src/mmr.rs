//! Maximal marginal relevance: keeps relevant hits that do not repeat each other.

use crate::models::hit::Hit;
use crate::text::{jaccard, word_set};
use std::collections::HashSet;

/// Picks up to `k` hits from `candidates` (sorted by score, best first).
///
/// `lambda` of 1.0 keeps pure relevance order; lower values penalize hits
/// whose words overlap an already selected hit.
pub fn select(candidates: Vec<Hit>, k: usize, lambda: f64) -> Vec<Hit> {
    let best = candidates
        .iter()
        .map(|hit| hit.score)
        .fold(f64::MIN_POSITIVE, f64::max);
    let sets: Vec<HashSet<String>> = candidates
        .iter()
        .map(|hit| word_set(&format!("{} {}", hit.title, hit.text)))
        .collect();
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();
    let mut chosen: Vec<usize> = Vec::new();
    while chosen.len() < k && !remaining.is_empty() {
        let mut best_position = 0usize;
        let mut best_value = f64::MIN;
        for (position, &candidate) in remaining.iter().enumerate() {
            let relevance = candidates[candidate].score / best;
            let redundancy = chosen
                .iter()
                .map(|&picked| jaccard(&sets[candidate], &sets[picked]))
                .fold(0.0, f64::max);
            let value = lambda * relevance - (1.0 - lambda) * redundancy;
            if value > best_value {
                best_value = value;
                best_position = position;
            }
        }
        chosen.push(remaining.remove(best_position));
    }
    let mut slots: Vec<Option<Hit>> = candidates.into_iter().map(Some).collect();
    chosen
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/mmr_tests.rs"]
mod tests;
