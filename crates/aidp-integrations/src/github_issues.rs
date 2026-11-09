//! GitHub issues through the `gh` CLI: the adapter of [`aidp_tickets::models::github_port::GithubPort`].
//!
//! Reads go through `gh issue list --json` (the CLI paginates) and `gh api` GETs; writes
//! (labels, comments) go through `gh api` with a JSON body file, never a body on argv.

use crate::pr_writes::{run_gh, send, strings};
use crate::IntegrationError;
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_tickets::models::created_issue::{CreatedIssue, SubIssue};
use aidp_tickets::models::enums::issue_state::IssueState;
use aidp_tickets::models::github_issue::{GithubIssue, IssueDependency};
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::models::milestone::Milestone;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Fields requested from `gh issue list`.
const LIST_FIELDS: &str = "number,title,body,labels,milestone,state,url,createdAt,updatedAt";

/// Words GitHub answers with when an issue or a route does not exist.
const NOT_FOUND_MARKERS: [&str; 2] = ["Not Found", "HTTP 404"];

/// `gh api` JSON bodies.
#[derive(Serialize)]
struct LabelsBody<'a> {
    labels: &'a [String],
}

#[derive(Serialize)]
struct NewIssueBody<'a> {
    title: &'a str,
    body: &'a str,
    labels: &'a [String],
}

#[derive(Serialize)]
struct SubIssueBody {
    sub_issue_id: u64,
}

#[derive(Serialize)]
struct CloseBody {
    state: &'static str,
    state_reason: &'static str,
}

#[derive(Serialize)]
struct CommentBody<'a> {
    body: &'a str,
}

/// The GitHub issue port over `gh`. `runner` is swapped for a scripted one in tests.
pub struct GhIssuePort {
    runner: Arc<dyn ProcessRunner>,
}

impl Default for GhIssuePort {
    fn default() -> Self {
        Self::new()
    }
}

impl GhIssuePort {
    /// The port over the real `gh`.
    pub fn new() -> Self {
        Self {
            runner: Arc::new(OsProcessRunner),
        }
    }

    /// The port over `runner`.
    pub fn with_runner(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }

    fn gh(&self, args: Vec<String>) -> Result<String, IntegrationError> {
        run_gh(self.runner.as_ref(), args)
    }

    /// A GET that reads "not found" as `None`.
    fn get_optional(&self, path: &str) -> anyhow::Result<Option<Value>> {
        match self.gh(strings(&["api", path])) {
            Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
            Err(IntegrationError::Upstream(message))
                if NOT_FOUND_MARKERS
                    .iter()
                    .any(|marker| message.contains(marker)) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// Percent-encodes `text` for one URL path segment.
pub fn encode_segment(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn timestamp(value: &Value, key: &str) -> Option<DateTime<Utc>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Label names of an issue: `gh issue list` gives `[{name}]`, the REST API the same.
fn labels_of(value: &Value) -> Vec<String> {
    value
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| {
                    label
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| label.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The milestone of an issue; `gh issue list` says `dueOn`, the REST API `due_on`.
fn milestone_of(value: &Value) -> Option<Milestone> {
    let milestone = value.get("milestone").filter(|m| !m.is_null())?;
    let title = text(milestone, "title");
    if title.is_empty() {
        return None;
    }
    Some(Milestone {
        title,
        number: milestone.get("number").and_then(Value::as_u64),
        due_on: timestamp(milestone, "dueOn").or_else(|| timestamp(milestone, "due_on")),
    })
}

/// One issue from either `gh issue list --json` or the REST API.
pub fn issue_from_json(value: &Value) -> Option<GithubIssue> {
    Some(GithubIssue {
        number: value.get("number").and_then(Value::as_u64)?,
        title: text(value, "title"),
        body: text(value, "body"),
        labels: labels_of(value),
        milestone: milestone_of(value),
        state: IssueState::from_wire(&text(value, "state")),
        url: {
            let url = text(value, "url");
            if url.is_empty() {
                text(value, "html_url")
            } else {
                url
            }
        },
        created_at: timestamp(value, "createdAt").or_else(|| timestamp(value, "created_at")),
        updated_at: timestamp(value, "updatedAt").or_else(|| timestamp(value, "updated_at")),
    })
}

/// `owner/name` out of a REST `repository_url` (`https://api.github.com/repos/o/r`).
fn repo_of_url(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/repos/")?;
    Some(rest.trim_end_matches('/').to_string())
}

impl GithubPort for GhIssuePort {
    fn list_open(&self, repo: &str, limit: u32) -> anyhow::Result<Vec<GithubIssue>> {
        let output = self.gh(strings(&[
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            "open",
            "--limit",
            &limit.to_string(),
            "--json",
            LIST_FIELDS,
        ]))?;
        let items: Vec<Value> = serde_json::from_str(&output)?;
        Ok(items.iter().filter_map(issue_from_json).collect())
    }

    fn view(&self, repo: &str, number: u64) -> anyhow::Result<Option<GithubIssue>> {
        let path = format!("repos/{repo}/issues/{number}");
        Ok(self.get_optional(&path)?.as_ref().and_then(issue_from_json))
    }

    fn blocked_by(&self, repo: &str, number: u64) -> anyhow::Result<Vec<IssueDependency>> {
        let path = format!("repos/{repo}/issues/{number}/dependencies/blocked_by");
        let Some(Value::Array(items)) = self.get_optional(&path)? else {
            return Ok(Vec::new());
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                let number = item.get("number").and_then(Value::as_u64)?;
                let repo = item
                    .get("repository_url")
                    .and_then(Value::as_str)
                    .and_then(repo_of_url)
                    .unwrap_or_else(|| repo.to_string());
                Some(IssueDependency {
                    repo,
                    number,
                    state: IssueState::from_wire(&text(item, "state")),
                })
            })
            .collect())
    }

    fn edit_labels(
        &self,
        repo: &str,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> anyhow::Result<()> {
        if !add.is_empty() {
            send(
                self.runner.as_ref(),
                "POST",
                format!("repos/{repo}/issues/{number}/labels"),
                Some(&LabelsBody { labels: add }),
            )?;
        }
        for label in remove {
            let path = format!(
                "repos/{repo}/issues/{number}/labels/{}",
                encode_segment(label)
            );
            match send(self.runner.as_ref(), "DELETE", path, None::<&()>) {
                Ok(_) => {}
                // A label already gone is the state we wanted.
                Err(IntegrationError::Upstream(message))
                    if NOT_FOUND_MARKERS
                        .iter()
                        .any(|marker| message.contains(marker)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn comment(&self, repo: &str, number: u64, body: &str) -> anyhow::Result<Option<String>> {
        let reply = send(
            self.runner.as_ref(),
            "POST",
            format!("repos/{repo}/issues/{number}/comments"),
            Some(&CommentBody { body }),
        )?;
        let value: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
        Ok(value
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    fn create_issue(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> anyhow::Result<CreatedIssue> {
        let reply = send(
            self.runner.as_ref(),
            "POST",
            format!("repos/{repo}/issues"),
            Some(&NewIssueBody {
                title,
                body,
                labels,
            }),
        )?;
        let value: Value = serde_json::from_str(&reply)?;
        Ok(CreatedIssue {
            number: value
                .get("number")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("GitHub answered without an issue number"))?,
            id: value
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("GitHub answered without an issue id"))?,
            url: text(&value, "html_url"),
        })
    }

    fn add_sub_issue(&self, repo: &str, parent: u64, sub_issue_id: u64) -> anyhow::Result<()> {
        send(
            self.runner.as_ref(),
            "POST",
            format!("repos/{repo}/issues/{parent}/sub_issues"),
            Some(&SubIssueBody { sub_issue_id }),
        )?;
        Ok(())
    }

    fn sub_issues(&self, repo: &str, parent: u64) -> anyhow::Result<Vec<SubIssue>> {
        let path = format!("repos/{repo}/issues/{parent}/sub_issues?per_page=100");
        let Some(Value::Array(items)) = self.get_optional(&path)? else {
            return Ok(Vec::new());
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                Some(SubIssue {
                    number: item.get("number").and_then(Value::as_u64)?,
                    state: IssueState::from_wire(&text(item, "state")),
                    labels: labels_of(item),
                })
            })
            .collect())
    }

    fn close_issue(&self, repo: &str, number: u64) -> anyhow::Result<()> {
        send(
            self.runner.as_ref(),
            "PATCH",
            format!("repos/{repo}/issues/{number}"),
            Some(&CloseBody {
                state: "closed",
                state_reason: "completed",
            }),
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/units/github_issues_tests.rs"]
pub(crate) mod tests;
