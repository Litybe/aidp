//! Reader of the graphify code graph (`graphify-out/graph.json`).

use crate::models::constants::paths::{BACKSLASH, GRAPH_FILE, SEPARATOR};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const FOLLOWED_RELATIONS: [&str; 4] = ["calls", "inherits", "implements", "references"];
const LINE_PREFIX: char = 'L';

#[derive(Deserialize)]
struct RawGraph {
    #[serde(default)]
    nodes: Vec<RawNode>,
    #[serde(default)]
    links: Vec<RawLink>,
}

#[derive(Deserialize)]
struct RawNode {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    source_file: Option<String>,
    #[serde(default)]
    source_location: Option<String>,
}

#[derive(Deserialize)]
struct RawLink {
    source: String,
    target: String,
    #[serde(default)]
    relation: String,
}

/// A symbol of the graph with its location in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub label: String,
    pub file: String,
    pub line: u32,
}

/// Symbols and their call, inheritance and reference neighbours.
#[derive(Debug, Default)]
pub struct CodeGraph {
    symbols: Vec<Option<Symbol>>,
    neighbours: Vec<Vec<usize>>,
    by_file: HashMap<String, Vec<usize>>,
    by_label: HashMap<String, Vec<usize>>,
}

impl CodeGraph {
    /// Loads the graph of a project, `None` when absent or unreadable.
    pub fn load(project_root: &Path) -> Option<CodeGraph> {
        let content = std::fs::read_to_string(project_root.join(GRAPH_FILE)).ok()?;
        Self::parse(&content)
    }

    /// Parses a graph document.
    pub fn parse(content: &str) -> Option<CodeGraph> {
        let raw: RawGraph = serde_json::from_str(content).ok()?;
        let mut graph = CodeGraph::default();
        let mut index_of: HashMap<&str, usize> = HashMap::new();
        for node in &raw.nodes {
            let position = graph.symbols.len();
            index_of.insert(node.id.as_str(), position);
            let located = node.source_file.as_ref().zip(node.source_location.as_ref());
            let symbol = located.and_then(|(file, location)| {
                let line = location
                    .trim_start_matches(LINE_PREFIX)
                    .parse::<u32>()
                    .ok()?;
                Some(Symbol {
                    label: node.label.clone(),
                    file: file.replace(BACKSLASH, &SEPARATOR.to_string()),
                    line,
                })
            });
            if let Some(symbol) = &symbol {
                graph
                    .by_file
                    .entry(symbol.file.clone())
                    .or_default()
                    .push(position);
                graph
                    .by_label
                    .entry(crate::text::fold(&symbol.label))
                    .or_default()
                    .push(position);
            }
            graph.symbols.push(symbol);
            graph.neighbours.push(Vec::new());
        }
        for link in &raw.links {
            if !FOLLOWED_RELATIONS.contains(&link.relation.as_str()) {
                continue;
            }
            if let (Some(&from), Some(&to)) = (
                index_of.get(link.source.as_str()),
                index_of.get(link.target.as_str()),
            ) {
                graph.neighbours[from].push(to);
                graph.neighbours[to].push(from);
            }
        }
        Some(graph)
    }

    /// Symbols defined in `file` within the inclusive line range.
    pub fn symbols_in(&self, file: &str, line_start: u32, line_end: u32) -> Vec<usize> {
        self.by_file.get(file).map_or_else(Vec::new, |positions| {
            positions
                .iter()
                .copied()
                .filter(|&position| {
                    self.symbol(position)
                        .is_some_and(|symbol| symbol.line >= line_start && symbol.line <= line_end)
                })
                .collect()
        })
    }

    /// Symbols whose folded label equals `term`.
    pub fn symbols_named(&self, term: &str) -> Vec<usize> {
        self.by_label.get(term).cloned().unwrap_or_default()
    }

    /// Located neighbours of a symbol.
    pub fn neighbours_of(&self, position: usize) -> Vec<usize> {
        self.neighbours
            .get(position)
            .map(|list| {
                list.iter()
                    .copied()
                    .filter(|&other| self.symbol(other).is_some())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The located symbol at a position.
    pub fn symbol(&self, position: usize) -> Option<&Symbol> {
        self.symbols.get(position).and_then(Option::as_ref)
    }
}

#[cfg(test)]
#[path = "../tests/unit/graph_tests.rs"]
mod tests;
