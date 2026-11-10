//! Evidence about bugs that do not converge, computed by the daemon from git and from the test
//! reports, never by the model: what each bug changed, which test-infrastructure files the branch
//! changed, how far it lags behind `main`, and the errors several tickets share.

use crate::models::constants::doctor::{
    BUG_COMMITS_MAX, CAUSE_WORD_MIN_CHARS, HARNESS_BASE, HARNESS_REMOTE, PARENT_BRANCH_PREFIX,
    SAME_CAUSE_PERCENT, SHARED_PATTERNS_MAX, SHARED_PATTERN_MIN_TICKETS, SIGNATURE_MAX_CHARS,
    SIGNATURE_MIN_CHARS, STAT_MAX_CHARS, SUSPECT_SIZE_PERCENT, TEST_INFRA_PATHS,
};
use crate::models::{
    DoctorBugFix, DoctorFileChange, DoctorInfraFile, DoctorPrLag, DoctorSharedPattern,
};
use aidp_core::process_runner::{ProcessRunner, ProcessSpec};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const GIT_PROGRAM: &str = "git";
const NO_PROMPT_VARIABLE: &str = "GIT_TERMINAL_PROMPT";
const NO_PROMPT_VALUE: &str = "0";
const FIELD_SEPARATOR: char = '\u{1f}';
const FIELD_SEPARATOR_FORMAT: &str = "%x1f";
const BUG_MARKERS: [&str; 2] = ["fix/", "fix("];
const PATH_TOKEN: &str = "<path>";
const ID_TOKEN: &str = "<id>";
const NUMBER_TOKEN: char = '#';
const HEX_ID_MIN_CHARS: usize = 8;
const TOKEN_EDGES: [char; 10] = ['.', ',', ';', ':', '\'', '"', '(', ')', '[', ']'];
const MERGE_CLEAN_CODE: i32 = 0;
const MERGE_CONFLICT_CODE: i32 = 1;
pub const STATUS_ADDED: &str = "A";
const STATUS_DELETED: &str = "D";
const BINARY_COUNT: &str = "-";
const NO_ERROR_FRAGMENTS: [&str; 3] = ["0 error", "no error", "errors: 0"];
const ERROR_WORDS: [&str; 2] = ["error", "exception"];
const FIDELITY_WORD: &str = "fidelity";
const EXIT_ONE_FRAGMENTS: [&str; 7] = [
    "exit 1",
    "exit code 1",
    "exit code: 1",
    "exited with 1",
    "exited with code 1",
    "exit status 1",
    "exit=1",
];
const FIDELITY_KEY: &str = "fidelity_exit_1";
const KNOWN_SIGNATURES: [(&str, &str); 2] = [
    ("no_player_in_scene", "no player in the scene"),
    (
        "input_settings_lost_native_object",
        "inputsettings has lost its native object",
    ),
];
const MERGE_TREE: [&str; 4] = ["merge-tree", "--write-tree", "--name-only", "--no-messages"];

/// Runs `git` in one directory through the daemon's process runner.
pub struct Git<'a> {
    pub runner: &'a dyn ProcessRunner,
    pub cwd: &'a Path,
}

impl Git<'_> {
    /// The raw output of `git args`.
    pub fn output(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        let mut spec = ProcessSpec::new(GIT_PROGRAM).args(args.iter().map(|part| part.to_string()));
        spec.cwd = Some(self.cwd.to_path_buf());
        spec.envs
            .insert(NO_PROMPT_VARIABLE.to_string(), NO_PROMPT_VALUE.to_string());
        self.runner.output(&spec)
    }

    /// The trimmed stdout of `git args`, or the error with stderr.
    pub fn run(&self, args: &[&str]) -> Result<String, String> {
        let output = self
            .output(args)
            .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(format!(
                "git {} exited {:?}: {}",
                args.join(" "),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    pub fn resolves(&self, reference: &str) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", reference])
            .is_ok()
    }
}

/// Branch of feature ticket `ticket`: the one its bugs start from and merge back into.
pub fn parent_branch(ticket: &str) -> String {
    format!("{PARENT_BRANCH_PREFIX}{ticket}")
}

/// `origin/<branch>` when the remote branch is known, else the local branch, else `None`.
pub fn tip_of(git: &Git<'_>, branch: &str) -> Option<String> {
    let remote = format!("{HARNESS_REMOTE}/{branch}");
    [remote, branch.to_string()]
        .into_iter()
        .find(|reference| git.resolves(reference))
}

/// `origin/main`, else the local `main`.
pub fn base_of(git: &Git<'_>) -> Option<String> {
    tip_of(git, HARNESS_BASE)
}

/// Brings the remote refs of `main` and of `branch` up to date; the working tree is not touched.
pub fn fetch(git: &Git<'_>, branch: &str) {
    let _ = git.run(&["fetch", HARNESS_REMOTE, HARNESS_BASE, branch]);
}

fn leading_key(text: &str) -> Option<String> {
    let prefix: String = text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let rest = text[prefix.len()..].strip_prefix('-')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let starts_with_letter = prefix
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic());
    (starts_with_letter && !digits.is_empty()).then(|| format!("{prefix}-{digits}"))
}

/// The key of the bug a commit subject names: `Merge … from owner/fix/GH-228`, `fix(GH-228): …`.
pub fn bug_key_in(subject: &str) -> Option<String> {
    for marker in BUG_MARKERS {
        let mut from = 0;
        while let Some(at) = subject[from..].find(marker) {
            let start = from + at + marker.len();
            if let Some(key) = leading_key(&subject[start..]) {
                return Some(key);
            }
            from = start;
        }
    }
    None
}

fn truncated(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn file_changes(numstat: &str) -> Vec<DoctorFileChange> {
    numstat
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?;
            let deleted = fields.next()?;
            let path = fields.next()?;
            let count = |text: &str| (text != BINARY_COUNT).then(|| text.parse().ok()).flatten();
            Some(DoctorFileChange {
                path: path.to_string(),
                added: count(added),
                deleted: count(deleted),
            })
        })
        .collect()
}

/// What each bug of `parent` changed on `tip`, oldest first: the commits of the first-parent
/// history since `base` whose subject names a `fix/<key>` branch or a `fix(<key>)` commit.
pub fn bug_history(git: &Git<'_>, base: &str, tip: &str, parent: &str) -> Vec<DoctorBugFix> {
    let range = format!("{base}..{tip}");
    let format = format!("--format=%H{FIELD_SEPARATOR_FORMAT}%s");
    let Ok(log) = git.run(&["log", "--first-parent", "--reverse", &format, &range]) else {
        return Vec::new();
    };
    let mut fixes = Vec::new();
    for line in log.lines() {
        let Some((commit, subject)) = line.split_once(FIELD_SEPARATOR) else {
            continue;
        };
        let Some(bug) = bug_key_in(subject).filter(|key| key != parent) else {
            continue;
        };
        let before = format!("{commit}^1");
        let Ok(numstat) = git.run(&["diff", "--numstat", &before, commit]) else {
            continue;
        };
        let span = format!("{before}..{commit}");
        let limit = format!("-n{BUG_COMMITS_MAX}");
        let commits = git
            .run(&["log", "--format=%h %s", &limit, &span])
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let stat = git
            .run(&["diff", "--stat", &before, commit])
            .map(|text| truncated(&text, STAT_MAX_CHARS))
            .unwrap_or_default();
        fixes.push(DoctorBugFix {
            bug,
            commit: commit.to_string(),
            subject: subject.to_string(),
            commits,
            files: file_changes(&numstat),
            stat,
        });
    }
    fixes
}

/// True when `path` is inside one of the test-infrastructure folders and escapes none.
pub fn is_infra_path(path: &str) -> bool {
    let clean = !path.starts_with('/')
        && !path.contains(':')
        && !path
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".git");
    clean
        && TEST_INFRA_PATHS
            .iter()
            .any(|folder| path.starts_with(&format!("{folder}/")))
}

fn size_at(git: &Git<'_>, reference: &str, path: &str) -> Option<u64> {
    git.run(&["cat-file", "-s", &format!("{reference}:{path}")])
        .ok()
        .and_then(|text| text.parse().ok())
}

/// The test-infrastructure files `tip` changed against the merge base with `base`, with their
/// sizes before and after; `suspect` marks a deletion or a shrink to under 70 %.
pub fn infra_changes(git: &Git<'_>, base: &str, tip: &str) -> Vec<DoctorInfraFile> {
    let Ok(merge_base) = git.run(&["merge-base", base, tip]) else {
        return Vec::new();
    };
    let mut args = vec![
        "diff",
        "--name-status",
        "--no-renames",
        merge_base.as_str(),
        tip,
        "--",
    ];
    args.extend(TEST_INFRA_PATHS);
    let Ok(listing) = git.run(&args) else {
        return Vec::new();
    };
    listing
        .lines()
        .filter_map(|line| {
            let (status, path) = line.split_once('\t')?;
            let before_bytes = (status != STATUS_ADDED)
                .then(|| size_at(git, &merge_base, path))
                .flatten();
            let after_bytes = (status != STATUS_DELETED)
                .then(|| size_at(git, tip, path))
                .flatten();
            let shrunk = match (before_bytes, after_bytes) {
                (Some(before), Some(after)) => after * 100 < before * SUSPECT_SIZE_PERCENT,
                _ => false,
            };
            Some(DoctorInfraFile {
                path: path.to_string(),
                status: status.to_string(),
                before_bytes,
                after_bytes,
                suspect: status == STATUS_DELETED || shrunk,
            })
        })
        .collect()
}

/// How far `tip` lags behind `base`, and whether merging `base` into it is clean.
pub fn pr_lag(
    git: &Git<'_>,
    branch: &str,
    base: &str,
    tip: &str,
    pr_status: Option<&Value>,
) -> DoctorPrLag {
    let behind_by = git
        .run(&["rev-list", "--count", &format!("{tip}..{base}")])
        .ok()
        .and_then(|text| text.parse().ok());
    let mut args = MERGE_TREE.to_vec();
    args.extend([base, tip]);
    let (mergeable, conflicts) = match git.output(&args) {
        Ok(output) => match output.status.code() {
            Some(MERGE_CLEAN_CODE) => (Some(true), Vec::new()),
            Some(MERGE_CONFLICT_CODE) => (
                Some(false),
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .skip(1)
                    .take_while(|line| !line.trim().is_empty())
                    .map(str::to_string)
                    .collect(),
            ),
            _ => (None, Vec::new()),
        },
        Err(_) => (None, Vec::new()),
    };
    DoctorPrLag {
        branch: branch.to_string(),
        behind_by,
        mergeable,
        conflicts,
        pr_mergeable: pr_status
            .and_then(|status| status.get("mergeable"))
            .cloned(),
        unavailable: None,
    }
}

/// The text of a test report, with the ticket and the run it belongs to.
pub struct ReportText {
    pub ticket: String,
    pub run_id: String,
    pub text: String,
}

fn normalized_token(token: &str) -> String {
    if token.contains('/') || token.contains('\\') {
        return PATH_TOKEN.to_string();
    }
    let trimmed = token.trim_matches(&TOKEN_EDGES[..]);
    if trimmed.len() >= HEX_ID_MIN_CHARS
        && trimmed.chars().all(|c| c.is_ascii_hexdigit())
        && trimmed.chars().any(|c| c.is_ascii_digit())
    {
        return ID_TOKEN.to_string();
    }
    let mut out = String::new();
    for c in trimmed.chars() {
        if c.is_ascii_digit() {
            if !out.ends_with(NUMBER_TOKEN) {
                out.push(NUMBER_TOKEN);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One error line reduced to what stays when the run, the path and the numbers change.
pub fn normalize_line(line: &str) -> String {
    let words: Vec<String> = line
        .to_lowercase()
        .split_whitespace()
        .map(normalized_token)
        .filter(|word| !word.is_empty())
        .collect();
    truncated(&words.join(" "), SIGNATURE_MAX_CHARS)
}

/// The error signatures of `text`, one entry per key: the known ones by name, then every line
/// that names an error or an exception, normalized.
pub fn signatures(text: &str) -> Vec<(String, String)> {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let lower = line.to_lowercase();
        let known = KNOWN_SIGNATURES
            .iter()
            .find(|(_, needle)| lower.contains(needle))
            .map(|(key, _)| key.to_string());
        let fidelity = lower.contains(FIDELITY_WORD)
            && EXIT_ONE_FRAGMENTS
                .iter()
                .any(|fragment| lower.contains(fragment));
        let key = if let Some(key) = known {
            key
        } else if fidelity {
            FIDELITY_KEY.to_string()
        } else if ERROR_WORDS.iter().any(|word| lower.contains(word))
            && !NO_ERROR_FRAGMENTS
                .iter()
                .any(|fragment| lower.contains(fragment))
        {
            normalize_line(line)
        } else {
            continue;
        };
        if key.chars().count() >= SIGNATURE_MIN_CHARS {
            found
                .entry(key)
                .or_insert_with(|| truncated(line, SIGNATURE_MAX_CHARS * 2));
        }
    }
    found.into_iter().collect()
}

/// The signatures found in the reports of at least two different tickets. `family` maps a ticket
/// to the one it counts for, so a bug and its parent are one ticket.
pub fn shared_patterns(
    reports: &[ReportText],
    family: &dyn Fn(&str) -> String,
) -> Vec<DoctorSharedPattern> {
    type Seen = (String, BTreeSet<String>, BTreeSet<String>);
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    for report in reports {
        for (key, sample) in signatures(&report.text) {
            let entry = seen
                .entry(key)
                .or_insert_with(|| (sample, BTreeSet::new(), BTreeSet::new()));
            entry.1.insert(family(&report.ticket));
            entry.2.insert(report.run_id.clone());
        }
    }
    let mut shared: Vec<DoctorSharedPattern> = seen
        .into_iter()
        .filter(|(_, (_, tickets, _))| tickets.len() >= SHARED_PATTERN_MIN_TICKETS)
        .map(|(key, (sample, tickets, runs))| DoctorSharedPattern {
            key,
            sample,
            tickets: tickets.into_iter().collect(),
            runs: runs.into_iter().collect(),
        })
        .collect();
    shared.sort_by(|a, b| {
        b.tickets
            .len()
            .cmp(&a.tickets.len())
            .then_with(|| a.key.cmp(&b.key))
    });
    shared.truncate(SHARED_PATTERNS_MAX);
    shared
}

fn words_of(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= CAUSE_WORD_MIN_CHARS)
        .map(str::to_string)
        .collect()
}

/// True when a cause about `file` is not the cause one of the earlier bugs already worked on: no
/// earlier bug touched `file` with a commit subject sharing half of the cause's words.
pub fn differs_from_previous(file: &str, cause: &str, bugs: &[DoctorBugFix]) -> bool {
    let wanted = words_of(cause);
    if wanted.is_empty() {
        return false;
    }
    !bugs.iter().any(|bug| {
        let touched = bug.files.iter().any(|change| change.path == file);
        let described = words_of(&format!("{} {}", bug.subject, bug.commits.join(" ")));
        let shared = wanted.intersection(&described).count();
        touched && shared * 100 >= wanted.len() * SAME_CAUSE_PERCENT
    })
}

/// The git evidence about the branch of one ticket.
pub struct Gathered {
    pub bugs: Vec<DoctorBugFix>,
    pub infra: Vec<DoctorInfraFile>,
    pub lag: DoctorPrLag,
}

/// Fetches `main` and the branch of `ticket`, then reads what its bugs changed, which
/// test-infrastructure files it changed, and how far it lags behind `main`.
pub fn gather(git: &Git<'_>, ticket: &str, pr_status: Option<&Value>) -> Result<Gathered, String> {
    let branch = parent_branch(ticket);
    fetch(git, &branch);
    let tip = tip_of(git, &branch).ok_or_else(|| format!("branch `{branch}` is unknown"))?;
    let base = base_of(git).ok_or_else(|| format!("`{HARNESS_BASE}` is unknown"))?;
    Ok(Gathered {
        bugs: bug_history(git, &base, &tip, ticket),
        infra: infra_changes(git, &base, &tip),
        lag: pr_lag(git, &branch, &base, &tip, pr_status),
    })
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_converge_tests.rs"]
mod tests;
