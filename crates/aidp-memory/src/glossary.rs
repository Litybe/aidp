//! The synonym glossary: cached by file and modification time, it widens BM25 queries.

use crate::error::{MemoryError, Result};
use crate::knowledge_note::split_frontmatter;
use crate::models::constants::knowledge::{BLOCK_FENCE, GLOSSARY_BLOCK_OPEN, GLOSSARY_FILE};
use crate::models::glossary::Glossary;
use crate::text::{fold, is_stopword, singular};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;

type Cache = HashMap<PathBuf, (SystemTime, Arc<Glossary>)>;

static ACTIVE: RwLock<Option<PathBuf>> = RwLock::new(None);

fn cache() -> &'static RwLock<Cache> {
    static CACHE: OnceLock<RwLock<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The glossary of the last installed folder, reloaded when its file changed.
pub fn current() -> Option<Arc<Glossary>> {
    let path = ACTIVE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()?;
    cached(&path)
}

/// Makes `<knowledge_dir>/_glossary.md` the glossary of this process; the last call wins.
pub fn install_from_dir(knowledge_dir: &Path) {
    let path = knowledge_dir.join(GLOSSARY_FILE);
    *ACTIVE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
}

/// The glossary at `path`, parsed again only when its modification time changed.
pub fn cached(path: &Path) -> Option<Arc<Glossary>> {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()?;
    if let Some((stamp, glossary)) = cache()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(path)
    {
        if *stamp == modified {
            return Some(Arc::clone(glossary));
        }
    }
    let loaded = Glossary::load(path).map(Arc::new);
    let mut entries = cache()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match &loaded {
        Some(glossary) => {
            entries.insert(path.to_path_buf(), (modified, Arc::clone(glossary)));
        }
        None => {
            entries.remove(path);
        }
    }
    loaded
}

impl Glossary {
    /// Reads the YAML block of a glossary note.
    pub fn parse(content: &str) -> Result<Glossary> {
        let body = split_frontmatter(content).map_or_else(|| content.to_string(), |(_, body)| body);
        let mut lines = body.lines();
        lines
            .by_ref()
            .find(|line| line.trim() == GLOSSARY_BLOCK_OPEN)
            .ok_or_else(|| MemoryError::Invalid("the glossary has no yaml block".to_string()))?;
        let block: Vec<&str> = lines
            .take_while(|line| line.trim() != BLOCK_FENCE)
            .collect();
        serde_yaml::from_str(&block.join("\n"))
            .map_err(|error| MemoryError::Invalid(format!("invalid glossary: {error}")))
    }

    /// The glossary at `path`, or `None` when absent or unreadable.
    pub fn load(path: &Path) -> Option<Glossary> {
        let content = std::fs::read_to_string(path).ok()?;
        match Glossary::parse(&content) {
            Ok(glossary) => Some(glossary),
            Err(error) => {
                tracing::warn!(%error, "glossary ignored");
                None
            }
        }
    }

    /// The significant words of every group member that is a synonym of something in the query.
    ///
    /// A group matches when one of its members appears in the query as a word or a phrase.
    /// The other members come back in group order, each as its list of words.
    pub fn expansions(&self, query: &str) -> Vec<Vec<String>> {
        let query_words = ordered_words(query);
        let mut added: Vec<Vec<String>> = Vec::new();
        for group in &self.groups {
            let members: Vec<Vec<String>> =
                group.iter().map(|member| ordered_words(member)).collect();
            let matched: Vec<bool> = members
                .iter()
                .map(|member| contains_sequence(&query_words, member))
                .collect();
            if !matched.iter().any(|found| *found) {
                continue;
            }
            for (member, found) in group.iter().zip(&matched) {
                if *found {
                    continue;
                }
                let words = crate::text::words(member);
                if !words.is_empty() && !added.contains(&words) {
                    added.push(words);
                }
            }
        }
        added
    }
}

fn ordered_words(text: &str) -> Vec<String> {
    fold(text)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(singular)
        .collect()
}

fn contains_sequence(haystack: &[String], needle: &[String]) -> bool {
    let significant = needle.iter().any(|word| !is_stopword(word));
    significant
        && !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
#[path = "../tests/unit/glossary_tests.rs"]
mod tests;
