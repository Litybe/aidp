//! Text helpers: word splitting, identifier tokens, query building, token estimates.

use crate::models::constants::limits::{
    CHARS_PER_TOKEN, MAX_IDENT_TOKENS, MAX_QUERY_TERMS, PLURAL_MIN_CHARS, PREFIX_MIN_CHARS,
};
use crate::models::constants::words::STOPWORDS;
use std::collections::{HashMap, HashSet};

const TERM_JOINER: &str = " OR ";
const SHORTEST_WORD: usize = 2;
const FIRST_LINE_BONUS: f64 = 3.0;
const IDENTIFIER_BONUS: f64 = 2.0;

/// Estimated tokens of a text: characters divided by four, rounded up.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// Lowercases a character and drops the common French and Latin diacritics.
pub fn fold_char(character: char) -> char {
    match character {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'œ' => 'o',
        other => other,
    }
}

/// Lowercases and strips diacritics.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(fold_char)
        .collect()
}

/// Whether a folded word is a stopword.
pub fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Alphanumeric runs of a text, folded, without stopwords or one-letter words.
pub fn words(text: &str) -> Vec<String> {
    fold(text)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= SHORTEST_WORD && !is_stopword(word))
        .map(str::to_string)
        .collect()
}

/// Splits an identifier at underscores, case changes and digit boundaries, lowercased.
pub fn split_identifier(identifier: &str) -> Vec<String> {
    let characters: Vec<char> = identifier.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, &character) in characters.iter().enumerate() {
        if !character.is_alphanumeric() {
            flush(&mut current, &mut parts);
            continue;
        }
        if index > 0 && !current.is_empty() {
            let previous = characters[index - 1];
            let next = characters.get(index + 1).copied();
            let lower_to_upper = previous.is_lowercase() && character.is_uppercase();
            let acronym_end = previous.is_uppercase()
                && character.is_uppercase()
                && next.is_some_and(char::is_lowercase);
            let digit_edge = previous.is_alphabetic() != character.is_alphabetic();
            if lower_to_upper || acronym_end || digit_edge {
                flush(&mut current, &mut parts);
            }
        }
        current.extend(character.to_lowercase());
    }
    flush(&mut current, &mut parts);
    parts
}

fn flush(current: &mut String, parts: &mut Vec<String>) {
    if !current.is_empty() {
        parts.push(fold(current));
        current.clear();
    }
}

/// Space-separated tokens of the camelCase and snake_case identifiers of a text.
pub fn identifier_tokens(text: &str) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut tokens: Vec<String> = Vec::new();
    let mut push = |token: String, tokens: &mut Vec<String>| {
        if token.chars().count() >= SHORTEST_WORD && seen.insert(token.clone()) {
            tokens.push(token);
        }
    };
    for identifier in text
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|word| !word.is_empty())
    {
        let parts = split_identifier(identifier);
        if parts.len() > 1 {
            for part in &parts {
                push(part.clone(), &mut tokens);
            }
            push(parts.concat(), &mut tokens);
        }
        if tokens.len() >= MAX_IDENT_TOKENS {
            break;
        }
    }
    tokens.join(" ")
}

/// Query terms: folded words, camelCase parts included, stopwords dropped.
///
/// A long query keeps its most telling terms: frequent ones, identifiers and words of its first line.
pub fn query_terms(query: &str) -> Vec<String> {
    let first_line_end = query.find('\n').unwrap_or(query.len());
    let mut terms: Vec<(String, f64, usize)> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut offset = 0;
    for raw in query.split(|character: char| !character.is_alphanumeric()) {
        let start = offset;
        offset += raw.len() + 1;
        if raw.is_empty() {
            continue;
        }
        let bonus = if start < first_line_end {
            FIRST_LINE_BONUS
        } else {
            0.0
        } + if is_identifier(raw) {
            IDENTIFIER_BONUS
        } else {
            0.0
        };
        let mut candidates = vec![fold(raw)];
        let parts = split_identifier(raw);
        if parts.len() > 1 {
            candidates.extend(parts);
        }
        for candidate in candidates {
            if candidate.chars().count() < SHORTEST_WORD || is_stopword(&candidate) {
                continue;
            }
            match positions.get(&candidate) {
                Some(&index) => terms[index].1 += 1.0,
                None => {
                    positions.insert(candidate.clone(), terms.len());
                    terms.push((candidate, 1.0 + bonus, terms.len()));
                }
            }
        }
    }
    if terms.len() > MAX_QUERY_TERMS {
        terms.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.2.cmp(&right.2)));
        terms.truncate(MAX_QUERY_TERMS);
        terms.sort_by_key(|term| term.2);
    }
    terms.into_iter().map(|(term, _, _)| term).collect()
}

/// A word written as code: inner capital, underscore or digit after a letter.
fn is_identifier(raw: &str) -> bool {
    raw.chars().skip(1).any(char::is_uppercase) || raw.contains('_')
}

/// Drops a trailing plural mark from a long word.
pub fn singular(term: &str) -> String {
    if term.chars().count() >= PLURAL_MIN_CHARS && (term.ends_with('s') || term.ends_with('x')) {
        term[..term.len() - 1].to_string()
    } else {
        term.to_string()
    }
}

/// An FTS5 match expression: every term as a prefix, joined with `OR`.
pub fn match_expression(query: &str) -> Option<String> {
    let clauses: Vec<String> = query_terms(query)
        .iter()
        .map(|term| singular(term))
        .map(|term| {
            if term.chars().count() >= PREFIX_MIN_CHARS {
                format!("\"{term}\"*")
            } else {
                format!("\"{term}\"")
            }
        })
        .collect();
    if clauses.is_empty() {
        None
    } else {
        Some(clauses.join(TERM_JOINER))
    }
}

/// Set of folded words used by the similarity measures.
pub fn word_set(text: &str) -> HashSet<String> {
    words(text).into_iter().collect()
}

/// Jaccard similarity of two word sets; zero when both are empty.
pub fn jaccard(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(right).count() as f64;
    let total = (left.len() + right.len()) as f64 - shared;
    shared / total
}

/// Hex SHA-256 of a text, truncated to sixteen characters.
pub fn short_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Longest prefix of a text that fits a character count, cut on a character boundary.
pub fn truncate_chars(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

#[cfg(test)]
#[path = "../tests/unit/text_tests.rs"]
mod tests;
