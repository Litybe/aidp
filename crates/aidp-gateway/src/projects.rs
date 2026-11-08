//! The project registry: the repos the gateway may launch sessions in.

use crate::ids::new_id;
use crate::store::{read_json, write_json};
use crate::sync::lock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const PROJECTS_FILE: &str = "projects.json";
const PIPELINES_DIR: &str = "Pipelines";
const GIT_DIR: &str = ".git";
const GENERATED_PREFIX: char = '_';
const BACKSLASH: char = '\\';
const SLASH: char = '/';
const SLASH_STR: &str = "/";

/// Outcome of a discovery pass (roots + vault).
#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverReport {
    pub roots: Vec<String>,
    pub scanned: usize,
    pub added: Vec<Project>,
    pub already: usize,
    /// Repos the vault knows but whose working copy was not found locally.
    pub unresolved: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub runs_count: u32,
    /// Absolute path of the dedicated clone where `workspace: project` flows (the Unity test lane)
    /// work. Unset: they work in `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_checkout: Option<String>,
}

impl Project {
    /// Where test flows work and where the Unity editor is looked for: the test checkout, else `path`.
    pub fn checkout_path(&self) -> &str {
        self.test_checkout.as_deref().unwrap_or(&self.path)
    }
}

pub struct Projects {
    file: PathBuf,
    list: Mutex<Vec<Project>>,
}

/// Stored with forward slashes so UI, vault `workspace_root` and root scans agree.
fn to_forward_slashes(path: &str) -> String {
    path.replace(BACKSLASH, SLASH_STR)
}

/// A checkout path as stored: forward slashes, no trailing slash; `None` when blank.
fn normalized_checkout(path: &str) -> Option<String> {
    let norm = to_forward_slashes(path.trim());
    let norm = norm.trim_end_matches(SLASH);
    (!norm.is_empty()).then(|| norm.to_string())
}

fn basename(p: &str) -> String {
    Path::new(p.trim_end_matches([SLASH, BACKSLASH]))
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

impl Projects {
    pub fn load(dir: &Path) -> Self {
        let file = dir.join(PROJECTS_FILE);
        let mut list: Vec<Project> = read_json(&file, Vec::new());
        for p in &mut list {
            p.path = to_forward_slashes(&p.path)
                .trim_end_matches(SLASH)
                .to_string();
            p.test_checkout = p.test_checkout.as_deref().and_then(normalized_checkout);
        }
        Projects {
            file,
            list: Mutex::new(list),
        }
    }

    fn save(&self, list: &[Project]) {
        if let Err(e) = write_json(&self.file, &list) {
            tracing::warn!(error = %e, "cannot save projects");
        }
    }

    pub fn list(&self) -> Vec<Project> {
        lock(&self.list).clone()
    }

    /// By id first, then by name (for the CLI's `--project cheat-alerts-service`).
    pub fn get(&self, id_or_name: &str) -> Option<Project> {
        let l = lock(&self.list);
        l.iter()
            .find(|p| p.id == id_or_name)
            .or_else(|| l.iter().find(|p| p.name == id_or_name))
            .cloned()
    }

    pub fn add(&self, path: &str) -> Project {
        let norm = to_forward_slashes(path);
        let mut l = lock(&self.list);
        if let Some(p) = l.iter().find(|p| to_forward_slashes(&p.path) == norm) {
            return p.clone();
        }
        let p = Project {
            id: new_id(),
            name: basename(path),
            path: norm.trim_end_matches(SLASH).to_string(),
            runs_count: 0,
            test_checkout: None,
        };
        l.push(p.clone());
        self.save(&l);
        p
    }

    /// Keep the caller's id when importing (the viewer's config.json ids stay valid).
    pub fn import(&self, incoming: Vec<Project>) -> usize {
        let mut l = lock(&self.list);
        let mut n = 0;
        for p in incoming {
            let norm = to_forward_slashes(&p.path);
            if l.iter()
                .any(|x| x.id == p.id || to_forward_slashes(&x.path) == norm)
            {
                continue;
            }
            l.push(Project {
                id: if p.id.is_empty() { new_id() } else { p.id },
                name: p.name,
                path: p.path,
                runs_count: 0,
                test_checkout: p.test_checkout.as_deref().and_then(normalized_checkout),
            });
            n += 1;
        }
        self.save(&l);
        n
    }

    /// Sets (`Some(absolute path)`) or clears (`None` or blank) the test checkout of a project.
    /// `Ok(None)` for an unknown project; `Err` for a relative path.
    pub fn set_test_checkout(
        &self,
        id_or_name: &str,
        path: Option<&str>,
    ) -> Result<Option<Project>, String> {
        let norm = path.and_then(normalized_checkout);
        if let Some(value) = &norm {
            if !Path::new(value).is_absolute() {
                return Err(format!("testCheckout must be an absolute path: `{value}`"));
            }
        }
        let mut l = lock(&self.list);
        let index = match l.iter().position(|p| p.id == id_or_name) {
            Some(index) => index,
            None => match l.iter().position(|p| p.name == id_or_name) {
                Some(index) => index,
                None => return Ok(None),
            },
        };
        l[index].test_checkout = norm;
        let updated = l[index].clone();
        self.save(&l);
        Ok(Some(updated))
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut l = lock(&self.list);
        let before = l.len();
        l.retain(|p| p.id != id);
        let removed = l.len() != before;
        if removed {
            self.save(&l);
        }
        removed
    }

    /// The folders holding the registered checkouts: where sibling repos are looked for.
    pub fn parent_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = lock(&self.list)
            .iter()
            .filter_map(|project| Path::new(&project.path).parent().map(Path::to_path_buf))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Registers git repos under `roots`, then vault repos whose working copy is found.
    /// Idempotent: existing registrations (by path) are kept as they are.
    pub fn discover(&self, roots: &[PathBuf], vault: Option<&Path>) -> DiscoverReport {
        let mut rep = DiscoverReport {
            roots: roots.iter().map(|r| r.display().to_string()).collect(),
            ..Default::default()
        };
        let consider = |path: PathBuf, rep: &mut DiscoverReport| {
            rep.scanned += 1;
            let before = lock(&self.list).len();
            let p = self.add(&path.to_string_lossy());
            if lock(&self.list).len() > before {
                rep.added.push(p);
            } else {
                rep.already += 1;
            }
        };
        for root in roots {
            let Ok(rd) = std::fs::read_dir(root) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && p.join(GIT_DIR).exists() {
                    consider(p, &mut rep);
                }
            }
        }
        if let Some(v) = vault {
            if let Ok(rd) = std::fs::read_dir(v.join(PIPELINES_DIR)) {
                for e in rd.flatten() {
                    let dir = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if !dir.is_dir() || name.starts_with(GENERATED_PREFIX) {
                        continue;
                    }
                    if self.get(&name).is_some() {
                        rep.already += 1;
                        continue;
                    }
                    match Self::locate_repo(&dir, &name, roots) {
                        Some(p) => consider(p, &mut rep),
                        None => rep.unresolved.push(name),
                    }
                }
            }
        }
        rep
    }

    /// Where is the working copy of a vault repo? Runs record `workspace_root` (a ticket
    /// worktree never counts); failing that, a folder with the repo's name under a root.
    fn locate_repo(vault_repo_dir: &Path, name: &str, roots: &[PathBuf]) -> Option<PathBuf> {
        if let Ok(features) = std::fs::read_dir(vault_repo_dir) {
            for f in features.flatten().filter(|f| f.path().is_dir()) {
                let Ok(runs) = std::fs::read_dir(f.path()) else {
                    continue;
                };
                for r in runs.flatten() {
                    let Some(state) = crate::runs::read_run_state(&r.path()) else {
                        continue;
                    };
                    let in_worktree = state
                        .workspace_root
                        .components()
                        .any(|part| part.as_os_str() == crate::worktrees::WORKTREE_DIR);
                    if state.workspace_root.is_dir() && !in_worktree {
                        return Some(state.workspace_root);
                    }
                }
            }
        }
        roots.iter().map(|r| r.join(name)).find(|p| p.is_dir())
    }
}

#[cfg(test)]
#[path = "../tests/unit/projects_tests.rs"]
mod tests;
