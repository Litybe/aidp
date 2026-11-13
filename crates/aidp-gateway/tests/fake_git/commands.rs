//! The git commands the gateway and its tests run, answered from the in-memory state.

use std::path::{Path, PathBuf};

use super::constants::*;
use super::merge;
use super::state::{
    key, matches_paths, scan, write_files, Context, Files, Head, Repo, State, Tree,
};

pub struct Reply {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn ok(stdout: impl Into<String>) -> Reply {
    Reply {
        code: EXIT_OK,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

fn fail(code: i32, message: impl Into<String>) -> Reply {
    Reply {
        code,
        stdout: String::new(),
        stderr: message.into(),
    }
}

fn lines(items: impl IntoIterator<Item = String>) -> Reply {
    let text: Vec<String> = items.into_iter().collect();
    ok(if text.is_empty() {
        String::new()
    } else {
        format!("{}\n", text.join("\n"))
    })
}

struct Call<'a> {
    cwd: PathBuf,
    args: Vec<&'a str>,
}

impl Call<'_> {
    fn has(&self, flag: &str) -> bool {
        self.args.contains(&flag)
    }

    fn positional(&self) -> Vec<&str> {
        let end = self
            .args
            .iter()
            .position(|arg| *arg == PATH_ARGUMENT_SEPARATOR)
            .unwrap_or(self.args.len());
        self.args[1..end]
            .iter()
            .copied()
            .filter(|arg| !arg.starts_with('-'))
            .collect()
    }

    fn paths(&self) -> Vec<String> {
        match self
            .args
            .iter()
            .position(|arg| *arg == PATH_ARGUMENT_SEPARATOR)
        {
            Some(index) => self.args[index + 1..]
                .iter()
                .map(|path| path.to_string())
                .collect(),
            None => Vec::new(),
        }
    }

    fn after(&self, flag: &str) -> Option<&str> {
        let index = self.args.iter().position(|arg| *arg == flag)?;
        self.args.get(index + 1).copied()
    }
}

impl State {
    pub fn run(&mut self, cwd: &Path, arguments: &[String]) -> Reply {
        let mut directory = cwd.to_path_buf();
        let mut rest: &[String] = arguments;
        while let Some(flag) = rest.first() {
            match (flag.as_str(), rest.get(1)) {
                (FLAG_DIRECTORY, Some(path)) => directory = PathBuf::from(path),
                (FLAG_CONFIG, Some(_)) => {}
                _ => break,
            }
            rest = &rest[2..];
        }
        let Some(command) = rest.first() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let call = Call {
            cwd: directory,
            args: rest.iter().map(String::as_str).collect(),
        };
        match command.as_str() {
            CMD_CONFIG | CMD_MAINTENANCE => ok(""),
            CMD_REMOTE => self.remote(&call),
            other => match self.locate(&call.cwd) {
                None => fail(EXIT_FATAL, NOT_A_REPOSITORY),
                Some(context) => self.dispatch(other, &call, &context),
            },
        }
    }

    fn dispatch(&mut self, command: &str, call: &Call<'_>, context: &Context) -> Reply {
        match command {
            CMD_REV_PARSE => self.rev_parse(call, context),
            CMD_STATUS => self.status(call, context),
            CMD_DIFF => self.diff(call, context),
            CMD_CAT_FILE => self.cat_file(call, context),
            CMD_LOG => self.log(call, context),
            CMD_MERGE_BASE => self.merge_base_command(call, context),
            CMD_REV_LIST => self.rev_list(call, context),
            CMD_MERGE_TREE => self.merge_tree(call, context),
            CMD_FETCH => self.fetch(call, context),
            CMD_WORKTREE => self.worktree(call, context),
            CMD_SHOW_REF => self.show_ref(call, context),
            CMD_SYMBOLIC_REF => self.symbolic_ref(call, context),
            CMD_BRANCH => self.branch(call, context),
            CMD_RESET => self.reset(call, context),
            CMD_CHECKOUT => self.checkout(call, context),
            CMD_ADD => self.add(call, context),
            CMD_REMOVE => self.remove_from_index(call, context),
            CMD_COMMIT => self.commit(call, context),
            CMD_MERGE => self.merge_command(call, context),
            CMD_PUSH => self.push(call, context),
            CMD_APPLY => self.apply(call, context),
            CMD_SHOW => self.show(call, context),
            _ => fail(
                EXIT_FATAL,
                format!("{UNSUPPORTED}: {}", call.args.join(" ")),
            ),
        }
    }

    fn remote(&mut self, call: &Call<'_>) -> Reply {
        let Some(context) = self.locate(&call.cwd) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let positional = call.positional();
        let repo = self.repos.get_mut(&context.repo);
        match (
            positional.first().copied(),
            positional.get(1),
            positional.get(2),
            repo,
        ) {
            (Some(SUB_SET_URL | SUB_ADD), Some(name), Some(url), Some(repo)) => {
                repo.remotes.insert(name.to_string(), key(Path::new(url)));
                ok("")
            }
            (Some(SUB_GET_URL), Some(name), _, Some(repo)) => match repo.remotes.get(*name) {
                Some(url) => ok(format!("{}\n", url.display())),
                None => fail(EXIT_FATAL, format!("{NO_SUCH_REMOTE}: {name}")),
            },
            _ => fail(
                EXIT_FATAL,
                format!("{UNSUPPORTED}: {}", call.args.join(" ")),
            ),
        }
    }

    fn rev_parse(&self, call: &Call<'_>, context: &Context) -> Reply {
        let tree = context.tree.as_ref().and_then(|path| self.trees.get(path));
        if call.has(FLAG_INSIDE_WORK_TREE) {
            return if tree.is_some() {
                ok("true\n")
            } else {
                ok("false\n")
            };
        }
        if call.has(FLAG_TOPLEVEL) {
            return match &context.tree {
                Some(path) => ok(format!("{}\n", path.display())),
                None => fail(EXIT_FATAL, NOT_A_REPOSITORY),
            };
        }
        if call.has(FLAG_ABBREV_REF) {
            return match tree.map(|tree| &tree.head) {
                Some(Head::Branch(name)) => ok(format!("{name}\n")),
                Some(Head::Detached(_)) => ok(format!("{HEAD_NAME}\n")),
                None => fail(EXIT_FATAL, NOT_A_REPOSITORY),
            };
        }
        let quiet = call.has(FLAG_QUIET);
        let Some(spec) = call.positional().last().copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        match self.resolve(context, spec) {
            Some(id) => ok(format!("{id}\n")),
            None if quiet => fail(EXIT_FAILED, ""),
            None => fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}")),
        }
    }

    fn status(&self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree) = &context.tree else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let entries = self.status_lines(tree, &call.paths(), call.has(FLAG_UNTRACKED_ALL));
        if call.has(FLAG_NUL) {
            return ok(entries
                .iter()
                .map(|entry| format!("{entry}\0"))
                .collect::<String>());
        }
        lines(entries)
    }

    fn diff(&self, call: &Call<'_>, context: &Context) -> Reply {
        let revisions = call.positional();
        let filters = call.paths();
        let resolve = |spec: &str| self.resolve(context, spec).map(|id| self.files_of(&id));
        let (left, right) = match (revisions.first(), revisions.get(1)) {
            (Some(first), Some(second)) => (resolve(first), resolve(second)),
            (Some(first), None) => {
                let disk = context.tree.as_ref().map(|path| {
                    let tracked = self.trees.get(path).map(|tree| tree.index.clone());
                    let mut files = scan(path);
                    files.retain(|name, _| tracked.as_ref().is_some_and(|t| t.contains_key(name)));
                    files
                });
                (resolve(first), disk)
            }
            _ => return fail(EXIT_FATAL, UNSUPPORTED),
        };
        let (Some(left), Some(right)) = (left, right) else {
            return fail(EXIT_FATAL, UNKNOWN_REVISION);
        };
        let changed = State::paths_changed(&left, &right, &filters);
        if call.has(FLAG_NAME_ONLY) {
            return lines(changed);
        }
        if call.has(FLAG_NAME_STATUS) {
            return lines(changed.into_iter().map(|path| {
                let code = match (left.contains_key(&path), right.contains_key(&path)) {
                    (false, _) => STATUS_ADDED,
                    (_, false) => STATUS_DELETED,
                    _ => STATUS_MODIFIED,
                };
                format!("{code}\t{path}")
            }));
        }
        if call.has(FLAG_NUMSTAT) {
            return lines(changed.into_iter().map(|path| {
                let (added, removed) = line_delta(left.get(&path), right.get(&path));
                format!("{added}\t{removed}\t{path}")
            }));
        }
        if call.has(FLAG_STAT) {
            return lines(changed.into_iter().map(|path| {
                let (added, removed) = line_delta(left.get(&path), right.get(&path));
                format!(" {path} | {}", added + removed)
            }));
        }
        fail(
            EXIT_FATAL,
            format!("{UNSUPPORTED}: {}", call.args.join(" ")),
        )
    }

    fn cat_file(&self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(spec) = call.positional().last().copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        match self.blob(context, spec) {
            None => fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}")),
            Some(bytes) if call.has(FLAG_SIZE) => ok(format!("{}\n", bytes.len())),
            Some(_) if call.has(FLAG_EXISTS) => ok(""),
            Some(bytes) => ok(String::from_utf8_lossy(&bytes).into_owned()),
        }
    }

    fn log(&self, call: &Call<'_>, context: &Context) -> Reply {
        let format = call
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix(FORMAT_PREFIX))
            .unwrap_or("%H %s");
        let limit = call
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix(LIMIT_PREFIX))
            .and_then(|count| count.parse::<usize>().ok())
            .or_else(|| call.has("-1").then_some(1));
        let spec = call
            .positional()
            .into_iter()
            .find(|arg| !arg.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| HEAD_NAME.to_string());
        let Some(mut ids) = self.range(&spec, context, call.has(FLAG_FIRST_PARENT)) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}"));
        };
        let filters = call.paths();
        if !filters.is_empty() {
            ids.retain(|id| {
                let commit = &self.commits[id];
                let before = commit
                    .parents
                    .first()
                    .map(|parent| self.files_of(parent))
                    .unwrap_or_default();
                !State::paths_changed(&before, &commit.files, &filters).is_empty()
            });
        }
        if let Some(limit) = limit {
            ids.truncate(limit);
        }
        if call.has(FLAG_REVERSE) {
            ids.reverse();
        }
        lines(ids.into_iter().map(|id| {
            format
                .replace("%H", &id)
                .replace("%h", &id[..ABBREVIATION_LENGTH])
                .replace(
                    "%s",
                    self.commits[&id].message.lines().next().unwrap_or_default(),
                )
                .replace("%cI", COMMIT_TIMESTAMP)
                .replace("%aI", COMMIT_TIMESTAMP)
                .replace(FIELD_SEPARATOR_TOKEN, FIELD_SEPARATOR)
        }))
    }

    fn merge_base_command(&self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        let (Some(first), Some(second)) = (positional.first(), positional.get(1)) else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let ids = (self.resolve(context, first), self.resolve(context, second));
        match ids {
            (Some(left), Some(right)) => match self.merge_base(&left, &right) {
                Some(base) => ok(format!("{base}\n")),
                None => fail(EXIT_FAILED, ""),
            },
            _ => fail(EXIT_FATAL, UNKNOWN_REVISION),
        }
    }

    fn rev_list(&self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(spec) = call.positional().last().copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let Some(mut ids) = self.range(spec, context, false) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}"));
        };
        if call.has("--no-merges") {
            ids.retain(|id| self.commits[id].parents.len() < 2);
        }
        if call.has(FLAG_COUNT) {
            return ok(format!("{}\n", ids.len()));
        }
        if call.has(FLAG_REVERSE) {
            ids.reverse();
        }
        lines(ids)
    }

    fn merge_tree(&self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        let (Some(first), Some(second)) = (positional.first(), positional.get(1)) else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let (Some(ours), Some(theirs)) =
            (self.resolve(context, first), self.resolve(context, second))
        else {
            return fail(EXIT_FATAL, UNKNOWN_REVISION);
        };
        let base = self
            .merge_base(&ours, &theirs)
            .map(|id| self.files_of(&id))
            .unwrap_or_default();
        let (_, conflicts) =
            merge::three_way(&base, &self.files_of(&ours), &self.files_of(&theirs));
        let tree = "0".repeat(ID_LENGTH);
        if conflicts.is_empty() {
            return ok(format!("{tree}\n"));
        }
        Reply {
            code: EXIT_FAILED,
            stdout: format!("{tree}\n{}\n\n", conflicts.join("\n")),
            stderr: String::new(),
        }
    }

    fn fetch(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        let remote = positional
            .first()
            .copied()
            .unwrap_or(DEFAULT_REMOTE)
            .to_string();
        let Some(source) = self
            .repos
            .get(&context.repo)
            .and_then(|repo| repo.remotes.get(&remote))
            .cloned()
        else {
            return fail(EXIT_FATAL, format!("{NO_SUCH_REMOTE}: {remote}"));
        };
        let Some(remote_refs) = self.repos.get(&source).map(|repo| repo.refs.clone()) else {
            return fail(EXIT_FATAL, format!("{NO_SUCH_REMOTE}: {remote}"));
        };
        let heads = |branch: &str| format!("{HEADS_PREFIX}{branch}");
        let wanted: Vec<String> = if positional.len() > 1 {
            positional[1..]
                .iter()
                .map(|branch| branch.to_string())
                .collect()
        } else {
            remote_refs
                .keys()
                .filter_map(|name| name.strip_prefix(HEADS_PREFIX))
                .map(str::to_string)
                .collect()
        };
        if let Some(missing) = wanted
            .iter()
            .find(|branch| !remote_refs.contains_key(&heads(branch)))
        {
            return fail(EXIT_FATAL, format!("{REMOTE_REF_MISSING} {missing}"));
        }
        let Some(repo) = self.repos.get_mut(&context.repo) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        for branch in wanted {
            let id = remote_refs[&heads(&branch)].clone();
            repo.refs
                .insert(format!("{REMOTES_PREFIX}{remote}/{branch}"), id);
        }
        ok("")
    }

    fn worktree(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        match positional.first().copied() {
            Some(SUB_LIST) => self.worktree_list(context),
            Some(SUB_ADD) => self.worktree_add(call, context),
            Some(SUB_REMOVE) => self.worktree_remove(call, context),
            Some(SUB_PRUNE) => {
                self.trees.retain(|path, _| path.exists());
                ok("")
            }
            _ => fail(
                EXIT_FATAL,
                format!("{UNSUPPORTED}: {}", call.args.join(" ")),
            ),
        }
    }

    fn worktree_list(&self, context: &Context) -> Reply {
        let mut text = String::new();
        let mut entries: Vec<(&PathBuf, &Tree)> = self
            .trees
            .iter()
            .filter(|(_, tree)| tree.repo == context.repo)
            .collect();
        entries.sort_by_key(|(path, _)| **path != context.repo);
        for (path, tree) in entries {
            text.push_str(&format!("worktree {}\n", path.display()));
            text.push_str(&format!(
                "HEAD {}\n",
                self.head_id(tree).unwrap_or_default()
            ));
            match &tree.head {
                Head::Branch(name) => text.push_str(&format!("branch {HEADS_PREFIX}{name}\n")),
                Head::Detached(_) => text.push_str("detached\n"),
            }
            text.push('\n');
        }
        ok(text)
    }

    fn worktree_add(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let new_branch = call.after(FLAG_BRANCH_CREATE).map(str::to_string);
        let skip = usize::from(new_branch.is_some());
        let operands: Vec<&str> = call.positional()[1..].iter().copied().skip(skip).collect();
        let Some(path) = operands.first() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let start = operands.get(1).copied().unwrap_or(HEAD_NAME);
        let Some(commit) = self.resolve(context, start) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {start}"));
        };
        let head = match &new_branch {
            Some(name) => {
                let reference = format!("{HEADS_PREFIX}{name}");
                let Some(repo) = self.repos.get_mut(&context.repo) else {
                    return fail(EXIT_FATAL, NOT_A_REPOSITORY);
                };
                if repo.refs.contains_key(&reference) {
                    return fail(EXIT_FATAL, format!("{BRANCH_EXISTS}: {name}"));
                }
                repo.refs.insert(reference, commit.clone());
                Head::Branch(name.clone())
            }
            None => Head::Detached(commit.clone()),
        };
        let target = Path::new(path);
        let _ = std::fs::create_dir_all(target);
        let files = self.files_of(&commit);
        write_files(target, &Files::new(), &files);
        let _ = std::fs::write(
            target.join(GIT_MARKER),
            format!("{GITDIR_PREFIX}{}\n", context.repo.display()),
        );
        self.trees.insert(
            key(target),
            Tree {
                repo: context.repo.clone(),
                head,
                index: files,
            },
        );
        ok("")
    }

    fn worktree_remove(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(path) = call.positional().get(1).copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let wanted = key(Path::new(path));
        let known = self
            .trees
            .get(&wanted)
            .is_some_and(|tree| tree.repo == context.repo);
        if !known {
            return fail(EXIT_FATAL, format!("{NOT_A_REPOSITORY}: {path}"));
        }
        let forced = call.has(FLAG_FORCE) || call.has(FLAG_FORCE_SHORT);
        if !forced && !self.status_lines(&wanted, &[], false).is_empty() {
            return fail(EXIT_FATAL, DIRTY_WORKTREE);
        }
        self.trees.remove(&wanted);
        let _ = std::fs::remove_dir_all(path);
        ok("")
    }

    fn show_ref(&self, call: &Call<'_>, context: &Context) -> Reply {
        let reference = call.positional().last().copied().unwrap_or_default();
        let found = self
            .repos
            .get(&context.repo)
            .is_some_and(|repo| repo.refs.contains_key(reference));
        if found {
            ok("")
        } else {
            fail(EXIT_FAILED, "")
        }
    }

    fn symbolic_ref(&self, call: &Call<'_>, context: &Context) -> Reply {
        let reference = call.positional().last().copied().unwrap_or_default();
        let repo = self.repos.get(&context.repo);
        let target = reference
            .strip_prefix(REMOTES_PREFIX)
            .and_then(|rest| rest.strip_suffix("/HEAD"))
            .and_then(|remote| {
                repo?
                    .remote_heads
                    .get(remote)
                    .map(|branch| (remote, branch))
            });
        match target {
            Some((remote, branch)) => ok(format!("{REMOTES_PREFIX}{remote}/{branch}\n")),
            None => fail(EXIT_FAILED, ""),
        }
    }

    fn branch(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        if call.has(FLAG_LIST) || positional.is_empty() {
            let current = context
                .tree
                .as_ref()
                .and_then(|path| self.trees.get(path))
                .and_then(|tree| match &tree.head {
                    Head::Branch(name) => Some(name.clone()),
                    Head::Detached(_) => None,
                });
            let names = self.repos.get(&context.repo).map(|repo| {
                repo.refs
                    .keys()
                    .filter_map(|name| name.strip_prefix(HEADS_PREFIX))
                    .map(|name| {
                        let marker = if current.as_deref() == Some(name) {
                            "*"
                        } else {
                            " "
                        };
                        format!("{marker} {name}")
                    })
                    .collect::<Vec<String>>()
            });
            return lines(names.unwrap_or_default());
        }
        let name = positional[0];
        let start = positional.get(1).copied().unwrap_or(HEAD_NAME);
        let Some(commit) = self.resolve(context, start) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {start}"));
        };
        let Some(repo) = self.repos.get_mut(&context.repo) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let reference = format!("{HEADS_PREFIX}{name}");
        if repo.refs.contains_key(&reference) {
            return fail(EXIT_FATAL, format!("{BRANCH_EXISTS}: {name}"));
        }
        repo.refs.insert(reference, commit);
        ok("")
    }

    fn move_head(&mut self, tree_path: &Path, commit: &str) {
        let Some(tree) = self.trees.get_mut(tree_path) else {
            return;
        };
        match tree.head.clone() {
            Head::Branch(name) => {
                if let Some(repo) = self.repos.get_mut(&tree.repo) {
                    repo.refs
                        .insert(format!("{HEADS_PREFIX}{name}"), commit.to_string());
                }
            }
            Head::Detached(_) => tree.head = Head::Detached(commit.to_string()),
        }
    }

    fn checkout_files(&mut self, tree_path: &Path, files: &Files) {
        let previous = scan(tree_path);
        write_files(tree_path, &previous, files);
        if let Some(tree) = self.trees.get_mut(tree_path) {
            tree.index = files.clone();
        }
    }

    fn reset(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree_path) = context.tree.clone() else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        if !call.has(FLAG_HARD) {
            return fail(
                EXIT_FATAL,
                format!("{UNSUPPORTED}: {}", call.args.join(" ")),
            );
        }
        let spec = call.positional().last().copied().unwrap_or(HEAD_NAME);
        let Some(commit) = self.resolve(context, spec) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}"));
        };
        self.move_head(&tree_path, &commit);
        let stale = self
            .trees
            .get(&tree_path)
            .map(|tree| tree.index.clone())
            .unwrap_or_default();
        let files = self.files_of(&commit);
        write_files(&tree_path, &stale, &files);
        if let Some(tree) = self.trees.get_mut(&tree_path) {
            tree.index = files;
        }
        ok("")
    }

    fn checkout(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree_path) = context.tree.clone() else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let positional = call.positional();
        let paths = call.paths();
        if !paths.is_empty() {
            let Some(spec) = positional.first().copied() else {
                return fail(EXIT_FATAL, UNSUPPORTED);
            };
            let Some(commit) = self.resolve(context, spec) else {
                return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}"));
            };
            let files = self.files_of(&commit);
            if let Some(missing) = paths.iter().find(|path| {
                !files
                    .keys()
                    .any(|name| matches_paths(name, std::slice::from_ref(path)))
            }) {
                return fail(EXIT_FAILED, format!("{MISSING_PATH}: {missing}"));
            }
            for (name, bytes) in files.iter().filter(|(name, _)| matches_paths(name, &paths)) {
                let target = tree_path.join(name);
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(target, bytes);
                if let Some(tree) = self.trees.get_mut(&tree_path) {
                    tree.index.insert(name.clone(), bytes.clone());
                }
            }
            return ok("");
        }
        if let Some(name) = call.after(FLAG_BRANCH_CREATE).map(str::to_string) {
            let start = positional.get(1).copied().unwrap_or(HEAD_NAME);
            let Some(commit) = self.resolve(context, start) else {
                return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {start}"));
            };
            if let Some(repo) = self.repos.get_mut(&context.repo) {
                repo.refs.insert(format!("{HEADS_PREFIX}{name}"), commit);
            }
            if let Some(tree) = self.trees.get_mut(&tree_path) {
                tree.head = Head::Branch(name);
            }
            return ok("");
        }
        let Some(target) = positional.first().copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let Some(commit) = self.resolve(context, target) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {target}"));
        };
        let is_branch = self
            .repos
            .get(&context.repo)
            .is_some_and(|repo| repo.refs.contains_key(&format!("{HEADS_PREFIX}{target}")));
        let files = self.files_of(&commit);
        self.checkout_files(&tree_path, &files);
        if let Some(tree) = self.trees.get_mut(&tree_path) {
            tree.head = if is_branch {
                Head::Branch(target.to_string())
            } else {
                Head::Detached(commit)
            };
        }
        ok("")
    }

    fn add(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree_path) = context.tree.clone() else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let mut filters = call.paths();
        if filters.is_empty() {
            filters = call.positional().into_iter().map(str::to_string).collect();
        }
        let disk = scan(&tree_path);
        let Some(tree) = self.trees.get_mut(&tree_path) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        tree.index
            .retain(|path, _| !matches_paths(path, &filters) || disk.contains_key(path));
        for (path, bytes) in disk
            .iter()
            .filter(|(path, _)| matches_paths(path, &filters))
        {
            tree.index.insert(path.clone(), bytes.clone());
        }
        ok("")
    }

    fn remove_from_index(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree) = context
            .tree
            .as_ref()
            .and_then(|path| self.trees.get_mut(path))
        else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let filters = call.paths();
        tree.index.retain(|path, _| !matches_paths(path, &filters));
        ok("")
    }

    fn commit(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree_path) = context.tree.clone() else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let message = call.after(FLAG_MESSAGE).unwrap_or_default().to_string();
        let Some(tree) = self.trees.get(&tree_path) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let parent = self.head_id(tree);
        let index = tree.index.clone();
        let unchanged = parent.as_ref().is_some_and(|id| self.files_of(id) == index);
        if unchanged {
            return Reply {
                code: EXIT_FAILED,
                stdout: format!("{NOTHING_TO_COMMIT}\n"),
                stderr: String::new(),
            };
        }
        let id = self.make_commit(parent.into_iter().collect(), &message, index);
        self.move_head(&tree_path, &id);
        ok(format!("[{}] {message}\n", &id[..ABBREVIATION_LENGTH]))
    }

    fn merge_command(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(tree_path) = context.tree.clone() else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        if call.has("--abort") {
            return fail(EXIT_FATAL, NO_MERGE_IN_PROGRESS);
        }
        let message = call.after(FLAG_MESSAGE).map(str::to_string);
        let target = call
            .positional()
            .into_iter()
            .rev()
            .find(|arg| Some(*arg) != message.as_deref());
        let Some(target) = target else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        let (Some(ours), Some(theirs)) = (
            self.resolve(context, HEAD_NAME),
            self.resolve(context, target),
        ) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {target}"));
        };
        if self.ancestors(&ours).contains(&theirs) {
            return ok("Already up to date.\n");
        }
        let base = self
            .merge_base(&ours, &theirs)
            .map(|id| self.files_of(&id))
            .unwrap_or_default();
        let (merged, conflicts) =
            merge::three_way(&base, &self.files_of(&ours), &self.files_of(&theirs));
        if !conflicts.is_empty() || call.has(FLAG_NO_COMMIT) {
            return Reply {
                code: EXIT_FAILED,
                stdout: format!("{MERGE_CONFLICT}: {}\n", conflicts.join(", ")),
                stderr: String::new(),
            };
        }
        let subject = message.unwrap_or_else(|| format!("Merge {target}"));
        let id = self.make_commit(vec![ours, theirs], &subject, merged.clone());
        self.move_head(&tree_path, &id);
        self.checkout_files(&tree_path, &merged);
        ok(format!("{subject}\n"))
    }

    fn push(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let positional = call.positional();
        let remote = positional
            .first()
            .copied()
            .unwrap_or(DEFAULT_REMOTE)
            .to_string();
        let spec = positional.get(1).copied().unwrap_or(HEAD_NAME);
        let forced = spec.starts_with('+') || call.has(FLAG_FORCE) || call.has(FLAG_FORCE_SHORT);
        let spec = spec.trim_start_matches('+');
        let (source, destination) = spec.split_once(':').unwrap_or((spec, spec));
        let destination = if destination == HEAD_NAME {
            source
        } else {
            destination
        };
        let destination = destination
            .strip_prefix(HEADS_PREFIX)
            .unwrap_or(destination);
        let Some(commit) = self.resolve(context, source) else {
            return fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {source}"));
        };
        let Some(url) = self
            .repos
            .get(&context.repo)
            .and_then(|repo| repo.remotes.get(&remote))
            .cloned()
        else {
            return fail(EXIT_FATAL, format!("{NO_SUCH_REMOTE}: {remote}"));
        };
        let reference = format!("{HEADS_PREFIX}{destination}");
        let existing = self
            .repos
            .get(&url)
            .and_then(|repo| repo.refs.get(&reference))
            .cloned();
        if let Some(old) = existing {
            if !forced && !self.ancestors(&commit).contains(&old) {
                return fail(EXIT_FAILED, PUSH_REJECTED);
            }
        }
        if let Some(target) = self.repos.get_mut(&url) {
            target.refs.insert(reference, commit.clone());
        }
        if let Some(repo) = self.repos.get_mut(&context.repo) {
            repo.refs
                .insert(format!("{REMOTES_PREFIX}{remote}/{destination}"), commit);
        }
        ok("")
    }

    fn apply(&mut self, call: &Call<'_>, context: &Context) -> Reply {
        let (Some(root), Some(file)) = (&context.tree, call.positional().last().copied()) else {
            return fail(EXIT_FATAL, NOT_A_REPOSITORY);
        };
        let Ok(text) = std::fs::read_to_string(file) else {
            return fail(EXIT_FATAL, format!("error: can't open patch '{file}'"));
        };
        match merge::apply(&text, root, !call.has(FLAG_CHECK)) {
            Ok(()) => ok(""),
            Err(message) => fail(EXIT_FAILED, message),
        }
    }

    fn show(&self, call: &Call<'_>, context: &Context) -> Reply {
        let Some(spec) = call.positional().last().copied() else {
            return fail(EXIT_FATAL, UNSUPPORTED);
        };
        if spec.contains(REVISION_PATH_SEPARATOR) {
            return match self.blob(context, spec) {
                Some(bytes) => Reply {
                    code: EXIT_OK,
                    stdout: String::from_utf8_lossy(&bytes).into_owned(),
                    stderr: String::new(),
                },
                None => fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}")),
            };
        }
        match self.resolve(context, spec) {
            Some(id) => ok(format!(
                "commit {id}\n\n    {}\n",
                self.commits[&id].message
            )),
            None => fail(EXIT_FATAL, format!("{UNKNOWN_REVISION} {spec}")),
        }
    }

    pub fn seed(&mut self, workspace: &Path, origin: Option<&Path>, branch: &str, files: &Files) {
        let id = self.make_commit(Vec::new(), "initial", files.clone());
        let heads = format!("{HEADS_PREFIX}{branch}");
        let mut repo = Repo::default();
        repo.refs.insert(heads.clone(), id.clone());
        let workspace_key = key(workspace);
        write_files(workspace, &Files::new(), files);
        let _ = std::fs::create_dir_all(workspace.join(GIT_MARKER));
        if let Some(origin) = origin {
            let origin_key = key(origin);
            let _ = std::fs::create_dir_all(origin);
            let mut bare = Repo::default();
            bare.refs.insert(heads, id.clone());
            self.repos.insert(origin_key.clone(), bare);
            repo.remotes.insert(DEFAULT_REMOTE.to_string(), origin_key);
            repo.refs
                .insert(format!("{REMOTES_PREFIX}{DEFAULT_REMOTE}/{branch}"), id);
            repo.remote_heads
                .insert(DEFAULT_REMOTE.to_string(), branch.to_string());
        }
        self.repos.insert(workspace_key.clone(), repo);
        self.trees.insert(
            workspace_key.clone(),
            Tree {
                repo: workspace_key,
                head: Head::Branch(branch.to_string()),
                index: files.clone(),
            },
        );
    }
}

fn line_delta(left: Option<&Vec<u8>>, right: Option<&Vec<u8>>) -> (usize, usize) {
    let text = |bytes: Option<&Vec<u8>>| {
        bytes
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    };
    let (before, after) = (text(left), text(right));
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let added = new.iter().filter(|line| !old.contains(line)).count();
    let removed = old.iter().filter(|line| !new.contains(line)).count();
    (added, removed)
}
