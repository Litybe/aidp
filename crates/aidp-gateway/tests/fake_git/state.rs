//! The in-memory repositories: commits as whole-tree snapshots, refs, worktrees on disk.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::constants::*;

pub type Files = BTreeMap<String, Vec<u8>>;

pub struct Commit {
    pub parents: Vec<String>,
    pub message: String,
    pub files: Files,
    pub sequence: u64,
}

#[derive(Clone)]
pub enum Head {
    Branch(String),
    Detached(String),
}

#[derive(Default)]
pub struct Repo {
    pub refs: BTreeMap<String, String>,
    pub remotes: BTreeMap<String, PathBuf>,
    pub remote_heads: BTreeMap<String, String>,
}

pub struct Tree {
    pub repo: PathBuf,
    pub head: Head,
    pub index: Files,
}

#[derive(Default)]
pub struct State {
    pub commits: HashMap<String, Commit>,
    pub repos: BTreeMap<PathBuf, Repo>,
    pub trees: BTreeMap<PathBuf, Tree>,
    pub next: u64,
}

pub struct Context {
    pub repo: PathBuf,
    pub tree: Option<PathBuf>,
}

pub fn key(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy();
    PathBuf::from(
        text.strip_prefix(UNC_PREFIX)
            .unwrap_or(&text)
            .replace('\\', "/"),
    )
}

pub fn scan(root: &Path) -> Files {
    let mut files = Files::new();
    scan_into(root, root, &mut files);
    files
}

fn scan_into(root: &Path, directory: &Path, files: &mut Files) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name() == GIT_MARKER {
            continue;
        }
        if path.is_dir() {
            scan_into(root, &path, files);
        } else if let (Ok(relative), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path)) {
            files.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
}

pub fn write_files(root: &Path, previous: &Files, next: &Files) {
    for path in previous.keys().filter(|path| !next.contains_key(*path)) {
        let _ = std::fs::remove_file(root.join(path));
    }
    for (path, bytes) in next {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(target, bytes);
    }
}

pub fn matches_paths(path: &str, filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| {
            let filter = filter.trim_end_matches('/');
            filter == "." || path == filter || path.starts_with(&format!("{filter}/"))
        })
}

impl State {
    pub fn locate(&self, cwd: &Path) -> Option<Context> {
        let wanted = key(cwd);
        let tree = self
            .trees
            .iter()
            .filter(|(path, _)| wanted.starts_with(path))
            .max_by_key(|(path, _)| path.as_os_str().len());
        if let Some((path, tree)) = tree {
            return Some(Context {
                repo: tree.repo.clone(),
                tree: Some(path.clone()),
            });
        }
        self.repos.contains_key(&wanted).then_some(Context {
            repo: wanted,
            tree: None,
        })
    }

    pub fn make_commit(&mut self, parents: Vec<String>, message: &str, files: Files) -> String {
        self.next += 1;
        let mut hasher = DefaultHasher::new();
        (self.next, &parents, message).hash(&mut hasher);
        let seed = hasher.finish();
        let mut id = String::new();
        let mut round = seed;
        while id.len() < ID_LENGTH {
            id.push_str(&format!("{round:016x}"));
            let mut next_hasher = DefaultHasher::new();
            round.hash(&mut next_hasher);
            round = next_hasher.finish();
        }
        id.truncate(ID_LENGTH);
        let commit = Commit {
            parents,
            message: message.to_string(),
            files,
            sequence: self.next,
        };
        self.commits.insert(id.clone(), commit);
        id
    }

    pub fn head_id(&self, tree: &Tree) -> Option<String> {
        match &tree.head {
            Head::Detached(id) => Some(id.clone()),
            Head::Branch(name) => self
                .repos
                .get(&tree.repo)?
                .refs
                .get(&format!("{HEADS_PREFIX}{name}"))
                .cloned(),
        }
    }

    pub fn resolve(&self, context: &Context, spec: &str) -> Option<String> {
        if let Some(base) = spec.strip_suffix(PARENT_SUFFIX) {
            let id = self.resolve(context, base)?;
            return self.commits.get(&id)?.parents.first().cloned();
        }
        if spec == HEAD_NAME {
            let tree = self.trees.get(context.tree.as_ref()?)?;
            return self.head_id(tree);
        }
        if self.commits.contains_key(spec) {
            return Some(spec.to_string());
        }
        let repo = self.repos.get(&context.repo)?;
        [
            spec.to_string(),
            format!("{HEADS_PREFIX}{spec}"),
            format!("{REMOTES_PREFIX}{spec}"),
        ]
        .iter()
        .find_map(|name| repo.refs.get(name).cloned())
    }

    pub fn ancestors(&self, id: &str) -> HashSet<String> {
        let mut seen = HashSet::new();
        let mut pending = vec![id.to_string()];
        while let Some(current) = pending.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(commit) = self.commits.get(&current) {
                pending.extend(commit.parents.iter().cloned());
            }
        }
        seen
    }

    pub fn merge_base(&self, first: &str, second: &str) -> Option<String> {
        let left = self.ancestors(first);
        let right = self.ancestors(second);
        let common: Vec<&String> = left.intersection(&right).collect();
        common
            .iter()
            .filter(|candidate| {
                !common.iter().any(|other| {
                    other != *candidate && self.ancestors(other).contains(candidate.as_str())
                })
            })
            .max_by_key(|id| self.commits.get(id.as_str()).map_or(0, |c| c.sequence))
            .map(|id| (*id).clone())
    }

    pub fn range(&self, spec: &str, context: &Context, first_parent: bool) -> Option<Vec<String>> {
        let (excluded, tip) = match spec.split_once(RANGE_SEPARATOR) {
            Some((base, tip)) => (self.ancestors(&self.resolve(context, base)?), tip),
            None => (HashSet::new(), spec),
        };
        let tip = self.resolve(context, tip)?;
        let mut selected: Vec<String> = Vec::new();
        if first_parent {
            let mut current = Some(tip);
            while let Some(id) = current {
                if excluded.contains(&id) {
                    break;
                }
                current = self.commits.get(&id)?.parents.first().cloned();
                selected.push(id);
            }
        } else {
            selected = self
                .ancestors(&tip)
                .into_iter()
                .filter(|id| !excluded.contains(id))
                .collect();
            selected.sort_by_key(|id| std::cmp::Reverse(self.commits[id].sequence));
        }
        Some(selected)
    }

    pub fn files_of(&self, id: &str) -> Files {
        self.commits
            .get(id)
            .map(|commit| commit.files.clone())
            .unwrap_or_default()
    }

    pub fn blob(&self, context: &Context, spec: &str) -> Option<Vec<u8>> {
        let (revision, path) = spec.split_once(REVISION_PATH_SEPARATOR)?;
        let id = self.resolve(context, revision)?;
        self.commits.get(&id)?.files.get(path).cloned()
    }

    pub fn paths_changed(left: &Files, right: &Files, filters: &[String]) -> BTreeSet<String> {
        left.keys()
            .chain(right.keys())
            .filter(|path| matches_paths(path, filters) && left.get(*path) != right.get(*path))
            .cloned()
            .collect()
    }

    pub fn status_lines(&self, tree_path: &Path, filters: &[String], all: bool) -> Vec<String> {
        let Some(tree) = self.trees.get(tree_path) else {
            return Vec::new();
        };
        let committed = self
            .head_id(tree)
            .map(|id| self.files_of(&id))
            .unwrap_or_default();
        let disk = scan(tree_path);
        let mut paths: BTreeSet<&String> = BTreeSet::new();
        paths.extend(committed.keys());
        paths.extend(tree.index.keys());
        paths.extend(disk.keys());
        let mut lines = Vec::new();
        let mut collapsed = BTreeSet::new();
        for path in paths
            .into_iter()
            .filter(|path| matches_paths(path, filters))
        {
            let staged = match (committed.get(path), tree.index.get(path)) {
                (None, Some(_)) => STATUS_ADDED,
                (Some(_), None) => STATUS_DELETED,
                (Some(left), Some(right)) if left != right => STATUS_MODIFIED,
                _ => STATUS_CLEAN,
            };
            if !tree.index.contains_key(path) && disk.contains_key(path) {
                let shown = if all {
                    path.clone()
                } else {
                    untracked_display(path, &tree.index)
                };
                if collapsed.insert(shown.clone()) {
                    lines.push(format!("{STATUS_UNTRACKED} {shown}"));
                }
                continue;
            }
            let unstaged = match (tree.index.get(path), disk.get(path)) {
                (Some(_), None) => STATUS_DELETED,
                (Some(left), Some(right)) if left != right => STATUS_MODIFIED,
                _ => STATUS_CLEAN,
            };
            if staged != STATUS_CLEAN || unstaged != STATUS_CLEAN {
                lines.push(format!("{staged}{unstaged} {path}"));
            }
        }
        lines
    }
}

fn untracked_display(path: &str, index: &Files) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    for depth in 1..parts.len() {
        let directory = format!("{}/", parts[..depth].join("/"));
        if !index.keys().any(|tracked| tracked.starts_with(&directory)) {
            return directory;
        }
    }
    path.to_string()
}
