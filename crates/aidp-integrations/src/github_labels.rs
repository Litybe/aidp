//! Migration of a repository's labels to the aidp vocabulary (plan §3.1): creates the missing
//! labels, `aidp:need-fix` becomes `aidp:fixing`, and `aidp:done` on an issue whose PR is still
//! open becomes `aidp:awaiting-merge`. A plan is computed from reads only; [`apply`] writes.

use crate::github_issues::GhIssuePort;
use crate::pr_writes::{run_gh, strings};
use aidp_core::process_runner::ProcessRunner;
use aidp_tickets::models::enums::state_label::StateLabel;
use aidp_tickets::models::github_port::GithubPort;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// The retired label `aidp:fixing` replaces.
pub const NEED_FIX_LABEL: &str = "aidp:need-fix";

/// Most labels, issues and pull requests read per call.
const READ_LIMIT: &str = "500";

/// Route and priority labels the plan's §3.1 routing rules look for, besides the state labels:
/// `(name, colour, description)`.
const ROUTE_LABELS: [(&str, &str, &str); 6] = [
    ("code", "1d76db", "aidp : route code"),
    ("art", "d876e3", "aidp : route art"),
    ("map", "0e8a16", "aidp : route map (LDtk)"),
    ("doc", "0075ca", "aidp : route doc (BIBLE, Docs)"),
    ("prio:high", "b60205", "aidp : passe devant dans la file"),
    ("prio:low", "c5def5", "aidp : passe derrière dans la file"),
];

/// One label to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelSpec {
    pub name: String,
    pub color: String,
    pub description: String,
}

/// One issue whose state label changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relabel {
    pub number: u64,
    pub title: String,
    pub add: Vec<String>,
    pub remove: Vec<String>,
    pub reason: String,
}

/// What the migration would do (or did) on one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationPlan {
    pub repo: String,
    pub create: Vec<LabelSpec>,
    pub relabel: Vec<Relabel>,
    /// Labels left in place that nothing writes any more (delete them by hand).
    pub obsolete: Vec<String>,
    pub applied: bool,
    pub errors: Vec<String>,
}

/// Every label aidp expects under `prefix`.
pub fn wanted_labels(prefix: &str) -> Vec<LabelSpec> {
    let mut out: Vec<LabelSpec> = StateLabel::ALL
        .into_iter()
        .map(|state| LabelSpec {
            name: state.label(prefix),
            color: state.color().to_string(),
            description: state.description().to_string(),
        })
        .collect();
    out.extend(
        ROUTE_LABELS
            .iter()
            .map(|(name, color, description)| LabelSpec {
                name: name.to_string(),
                color: color.to_string(),
                description: description.to_string(),
            }),
    );
    out
}

fn names(json: &str) -> anyhow::Result<Vec<String>> {
    let items: Vec<Value> = serde_json::from_str(json)?;
    Ok(items
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string))
        .collect())
}

fn read_issues(
    runner: &dyn ProcessRunner,
    repo: &str,
    state: &str,
    label: &str,
) -> anyhow::Result<Vec<(u64, String)>> {
    let json = run_gh(
        runner,
        strings(&[
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--label",
            label,
            "--limit",
            READ_LIMIT,
            "--json",
            "number,title",
        ]),
    )?;
    let items: Vec<Value> = serde_json::from_str(&json)?;
    Ok(items
        .iter()
        .filter_map(|item| {
            Some((
                item.get("number")?.as_u64()?,
                item.get("title")?.as_str()?.to_string(),
            ))
        })
        .collect())
}

/// Issue numbers that an open pull request closes (`closingIssuesReferences`, else `Closes #n`).
fn issues_with_open_pr(runner: &dyn ProcessRunner, repo: &str) -> anyhow::Result<HashSet<u64>> {
    let json = run_gh(
        runner,
        strings(&[
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            READ_LIMIT,
            "--json",
            "number,body,closingIssuesReferences",
        ]),
    )?;
    let closing = Regex::new(r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)")?;
    let prs: Vec<Value> = serde_json::from_str(&json)?;
    let mut out = HashSet::new();
    for pr in &prs {
        if let Some(refs) = pr.get("closingIssuesReferences").and_then(Value::as_array) {
            out.extend(
                refs.iter()
                    .filter_map(|r| r.get("number").and_then(Value::as_u64)),
            );
        }
        let body = pr.get("body").and_then(Value::as_str).unwrap_or_default();
        for capture in closing.captures_iter(body) {
            if let Ok(number) = capture[1].parse::<u64>() {
                out.insert(number);
            }
        }
    }
    Ok(out)
}

/// Reads `repo` and computes the migration. Writes nothing.
pub fn plan(runner: &dyn ProcessRunner, repo: &str, prefix: &str) -> anyhow::Result<MigrationPlan> {
    let present: HashSet<String> = names(&run_gh(
        runner,
        strings(&[
            "label", "list", "--repo", repo, "--limit", READ_LIMIT, "--json", "name",
        ]),
    )?)?
    .into_iter()
    .map(|name| name.to_ascii_lowercase())
    .collect();

    let fixing = StateLabel::Fixing.label(prefix);
    let awaiting = StateLabel::AwaitingMerge.label(prefix);
    let done = StateLabel::Done.label(prefix);

    let mut relabel = Vec::new();
    for (number, title) in read_issues(runner, repo, "all", NEED_FIX_LABEL)? {
        relabel.push(Relabel {
            number,
            title,
            add: vec![fixing.clone()],
            remove: vec![NEED_FIX_LABEL.to_string()],
            reason: format!("{NEED_FIX_LABEL} devient {fixing}"),
        });
    }
    let open_pr = issues_with_open_pr(runner, repo)?;
    for (number, title) in read_issues(runner, repo, "open", &done)? {
        if open_pr.contains(&number) {
            relabel.push(Relabel {
                number,
                title,
                add: vec![awaiting.clone()],
                remove: vec![done.clone()],
                reason: format!("PR ouverte : {done} devient {awaiting}"),
            });
        }
    }

    let create = wanted_labels(prefix)
        .into_iter()
        .filter(|spec| !present.contains(&spec.name.to_ascii_lowercase()))
        .collect();
    let obsolete = if present.contains(NEED_FIX_LABEL) {
        vec![NEED_FIX_LABEL.to_string()]
    } else {
        Vec::new()
    };
    Ok(MigrationPlan {
        repo: repo.to_string(),
        create,
        relabel,
        obsolete,
        applied: false,
        errors: Vec::new(),
    })
}

/// Executes `plan`: creates the labels first (the relabels add them), then relabels the issues.
/// A failure is recorded in [`MigrationPlan::errors`] and the run goes on.
pub fn apply(runner: Arc<dyn ProcessRunner>, mut plan: MigrationPlan) -> MigrationPlan {
    for spec in &plan.create {
        let result = run_gh(
            runner.as_ref(),
            strings(&[
                "label",
                "create",
                &spec.name,
                "--repo",
                &plan.repo,
                "--color",
                &spec.color,
                "--description",
                &spec.description,
            ]),
        );
        if let Err(e) = result {
            plan.errors.push(format!("create {}: {e}", spec.name));
        }
    }
    let port = GhIssuePort::with_runner(runner);
    for change in &plan.relabel {
        if let Err(e) = port.edit_labels(&plan.repo, change.number, &change.add, &change.remove) {
            plan.errors.push(format!("#{}: {e:#}", change.number));
        }
    }
    plan.applied = true;
    plan
}

#[cfg(test)]
#[path = "tests/units/github_labels_tests.rs"]
mod tests;
