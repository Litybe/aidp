use super::CommandOutput;
use crate::args::flow::Op;
use crate::ctx::Ctx;
use crate::models::constants::flow_git::*;
use crate::models::constants::flow_pr::*;
use crate::models::enums::{BranchKind, ValidationStatus};
use crate::models::ticket_heading::TicketHeading;
use crate::models::validation_report_body::ValidationReportBody;
use aidp_core::process_runner::ProcessSpec;
use aidp_core::{ExitStatus, Format, TypedExit};
use aidp_flows::{validate, Catalog, FlowDef, Report};
use aidp_gateway::flows::events::{FIELD_CURRENT, FIELD_KEY, FIELD_STATUS};
use aidp_gateway::flows::http_routes::FlowVerb;
use aidp_gateway::flows::registry::{BuiltinAction, BuiltinProbe};
use aidp_integrations::models::flow_branch_report::FlowBranchReport;
use aidp_integrations::models::flow_pr_file::FlowPrFile;
use aidp_integrations::models::flow_pr_report::FlowPrReport;
use aidp_integrations::{branch_publish, gh};
use aidp_tickets::model::{IssueType, Ticket};
use aidp_tickets::store::TicketStore;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod bug;
mod checkout;
mod unity_refresh;

const HTTP_CONFLICT_PREFIX: &str = "HTTP 409";
const HTTP_NOT_FOUND_PREFIX: &str = "HTTP 404";
const HTTP_BAD_REQUEST_PREFIX: &str = "HTTP 400";
const HTTP_UNAUTHORIZED_PREFIX: &str = "HTTP 401";
const UNREACHABLE_PREFIX: &str = "gateway unreachable";
const SETTING_SEPARATOR: char = '=';
const NO_VALUE_LABEL: &str = "-";
const LINE_SEPARATOR: &str = "\n";
const GIT_FAILED_MESSAGE: &str = "git failed";

/// Dispatches `aidp flow`: `validate` and `show` read a file, every other verb calls the daemon.
pub fn run(ctx: &Ctx, op: Op) -> anyhow::Result<CommandOutput> {
    match op {
        Op::Validate { path } => validate_file(ctx, &path),
        Op::Show { path } => show_file(ctx, &path),
        Op::GitBranch { key } => git_branch(ctx, &key),
        Op::GitCheckout { key } => checkout::git_checkout(ctx, &key),
        Op::GitRestore { key } => checkout::git_restore(ctx, &key),
        Op::UnityRefresh { timeout } => unity_refresh::unity_refresh(&ctx.workspace(), timeout),
        Op::GitPush { branch } => git_push(ctx, branch.as_deref()),
        Op::BugCreate {
            key,
            report_file,
            body_file,
            summary,
            allow_extra,
        } => bug::bug_create(
            ctx,
            &key,
            report_file.as_deref(),
            body_file.as_deref(),
            summary.as_deref().unwrap_or_default(),
            allow_extra,
        ),
        Op::BugRearm { key } => bug::bug_rearm(ctx, &key),
        Op::PrCreate {
            key,
            branch,
            body_file,
            draft,
        } => pr_create(ctx, &key, branch.as_deref(), &body_file, draft),
        Op::List { url } => daemon(ctx, aidp_gateway::client::list_flows(&url)),
        Op::Start {
            ticket,
            flow,
            settings,
            url,
        } => {
            let settings = parse_settings(&settings)?;
            daemon(
                ctx,
                aidp_gateway::client::start_flow(&url, &ticket, flow.as_deref(), settings),
            )
        }
        Op::Status { key, url } => daemon(ctx, aidp_gateway::client::get_flow(&url, &key)),
        Op::Approve {
            key,
            step,
            reject,
            url,
        } => {
            let verb = if reject {
                FlowVerb::Reject
            } else {
                FlowVerb::Approve
            };
            step_verb(ctx, &url, &key, &step, verb)
        }
        Op::Verdict {
            key,
            step,
            ok,
            report,
            url,
        } => daemon(
            ctx,
            aidp_gateway::client::verdict(&url, &key, &step, ok, report.as_deref()),
        ),
        Op::Retry { key, step, url } => step_verb(ctx, &url, &key, &step, FlowVerb::Retry),
        Op::Skip { key, step, url } => step_verb(ctx, &url, &key, &step, FlowVerb::Skip),
        Op::Pause { key, url } => flow_verb(ctx, &url, &key, FlowVerb::Pause),
        Op::Resume { key, url } => flow_verb(ctx, &url, &key, FlowVerb::Resume),
        Op::Abort { key, url } => flow_verb(ctx, &url, &key, FlowVerb::Abort),
    }
}

fn fmt(ctx: &Ctx) -> Format {
    ctx.format().unwrap_or(Format::JsonCompact)
}

fn flow_verb(
    ctx: &Ctx,
    url: &Option<String>,
    key: &str,
    verb: FlowVerb,
) -> anyhow::Result<CommandOutput> {
    daemon(ctx, aidp_gateway::client::flow_action(url, key, verb))
}

fn step_verb(
    ctx: &Ctx,
    url: &Option<String>,
    key: &str,
    step: &str,
    verb: FlowVerb,
) -> anyhow::Result<CommandOutput> {
    daemon(ctx, aidp_gateway::client::step_action(url, key, step, verb))
}

/// Renders a daemon answer, or turns a daemon failure into the documented exit status.
fn daemon(ctx: &Ctx, answer: anyhow::Result<Value>) -> anyhow::Result<CommandOutput> {
    match answer {
        Ok(value) => render(fmt(ctx), &value),
        Err(error) => Err(daemon_failure(error)),
    }
}

/// Maps the client's error text to an exit status; the message is kept as is.
pub(super) fn daemon_failure(error: anyhow::Error) -> anyhow::Error {
    let message = format!("{error}");
    let status = if message.starts_with(HTTP_CONFLICT_PREFIX)
        || message.starts_with(HTTP_UNAUTHORIZED_PREFIX)
    {
        ExitStatus::PolicyRefusal
    } else if message.starts_with(HTTP_NOT_FOUND_PREFIX) {
        ExitStatus::NotFound
    } else if message.starts_with(HTTP_BAD_REQUEST_PREFIX) {
        ExitStatus::Usage
    } else if message.starts_with(UNREACHABLE_PREFIX) {
        ExitStatus::UpstreamFailure
    } else {
        ExitStatus::Internal
    };
    TypedExit { status, message }.into()
}

fn render(format: Format, value: &Value) -> anyhow::Result<CommandOutput> {
    let rendered = match format {
        Format::Text => text_view(value),
        other => crate::output::render_value(other, value)?,
    };
    Ok(CommandOutput::text(rendered))
}

/// One line per run for a list, a key/status line for one run, pretty JSON otherwise.
fn text_view(value: &Value) -> String {
    match value {
        Value::Array(runs) => runs
            .iter()
            .map(run_line)
            .collect::<Vec<_>>()
            .join(LINE_SEPARATOR),
        Value::Object(map) if map.contains_key(FIELD_KEY) && map.contains_key(FIELD_STATUS) => {
            run_line(value)
        }
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn run_line(run: &Value) -> String {
    format!(
        "{}  {}  {}",
        label(&run[FIELD_KEY]),
        label(&run[FIELD_STATUS]),
        label(&run[FIELD_CURRENT])
    )
}

fn label(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| NO_VALUE_LABEL.to_string(), str::to_string)
}

/// Turns repeated `key=value` flags into a JSON object. A value that parses as JSON keeps its type.
fn parse_settings(pairs: &[String]) -> anyhow::Result<Option<Value>> {
    if pairs.is_empty() {
        return Ok(None);
    }
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let Some((key, raw)) = pair.split_once(SETTING_SEPARATOR) else {
            return Err(TypedExit {
                status: ExitStatus::Usage,
                message: format!("--setting expects key=value, got `{pair}`"),
            }
            .into());
        };
        let value = serde_json::from_str(raw).unwrap_or_else(|_| json!(raw));
        map.insert(key.to_string(), value);
    }
    Ok(Some(Value::Object(map)))
}

/// Reads and parses a definition file; a missing file is `NotFound`, a bad one a negative verdict.
fn load_definition(path: &Path) -> anyhow::Result<FlowDef> {
    let text = std::fs::read_to_string(path).map_err(|error| TypedExit {
        status: if error.kind() == std::io::ErrorKind::NotFound {
            ExitStatus::NotFound
        } else {
            ExitStatus::Internal
        },
        message: format!("flow file {}: {error}", path.display()),
    })?;
    FlowDef::from_yaml(&text).map_err(|error| {
        TypedExit {
            status: ExitStatus::NegativeVerdict,
            message: format!("{}: {error}", path.display()),
        }
        .into()
    })
}

fn show_file(ctx: &Ctx, path: &Path) -> anyhow::Result<CommandOutput> {
    let def = load_definition(path)?;
    let value = serde_json::to_value(&def)?;
    let rendered = match fmt(ctx) {
        Format::Text => serde_json::to_string_pretty(&value)?,
        other => crate::output::render_value(other, &value)?,
    };
    Ok(CommandOutput::text(rendered))
}

/// The registry ids and, when readable, the pipeline templates a definition may reference.
fn build_catalog(ctx: &Ctx) -> Catalog {
    let pipelines = crate::pipelines::discover_pipelines(ctx, &ctx.workspace())
        .ok()
        .map(|found| found.into_iter().map(|(id, _)| id).collect::<BTreeSet<_>>());
    builtin_catalog(pipelines)
}

/// A catalog holding every builtin action and wait probe of the gateway registry.
pub(crate) fn builtin_catalog(pipelines: Option<BTreeSet<String>>) -> Catalog {
    Catalog {
        actions: BuiltinAction::ALL
            .iter()
            .map(|action| action.as_str().to_string())
            .collect(),
        wait_probes: BuiltinProbe::ALL
            .iter()
            .map(|probe| probe.as_str().to_string())
            .collect(),
        pipelines,
    }
}

fn validate_file(ctx: &Ctx, path: &Path) -> anyhow::Result<CommandOutput> {
    let def = load_definition(path)?;
    let report = validate(&def, &build_catalog(ctx));
    let valid = report.is_valid();
    let rendered = render_report(fmt(ctx), path, &def, &report)?;
    let output = CommandOutput::text(rendered);
    Ok(if valid {
        output
    } else {
        output.with_exit(ExitStatus::NegativeVerdict)
    })
}

fn render_report(
    format: Format,
    path: &Path,
    def: &FlowDef,
    report: &Report,
) -> anyhow::Result<String> {
    if format == Format::Text {
        return Ok(text_report(path, report));
    }
    let body = ValidationReportBody {
        ok: report.is_valid(),
        id: &def.id,
        path,
        errors: &report.errors,
        warnings: &report.warnings,
    };
    let value = serde_json::to_value(&body)?;
    crate::output::render_value(format, &value)
}

fn text_report(path: &Path, report: &Report) -> String {
    let mut lines = vec![format!(
        "{}: {}",
        path.display(),
        ValidationStatus::of(report.is_valid()).as_str()
    )];
    lines.extend(
        report
            .errors
            .iter()
            .map(|issue| format!("error: {}", issue.message)),
    );
    lines.extend(
        report
            .warnings
            .iter()
            .map(|issue| format!("warning: {}", issue.message)),
    );
    lines.join(LINE_SEPARATOR)
}

fn typed(status: ExitStatus, message: impl Into<String>) -> anyhow::Error {
    TypedExit {
        status,
        message: message.into(),
    }
    .into()
}

fn report_output(report: &impl serde::Serialize) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::json(
        Some(Format::JsonCompact),
        serde_json::to_value(report)?,
    ))
}

#[cfg(test)]
/// The branch prefix of an issue type name; an unparsable name counts as a story.
pub(crate) fn branch_prefix(issue_type: &str) -> &'static str {
    BranchKind::from(parse_issue_type(issue_type)).prefix()
}

/// The issue type named `raw`, `Story` when it is not one of the known types.
fn parse_issue_type(raw: &str) -> IssueType {
    IssueType::from_str(raw).unwrap_or(IssueType::Story)
}

#[cfg(test)]
/// The working branch of ticket `key` whose issue type is named `issue_type`.
pub(crate) fn branch_name(key: &str, issue_type: &str) -> String {
    branch_of(key, parse_issue_type(issue_type))
}

fn branch_of(key: &str, issue_type: IssueType) -> String {
    format!("{}{key}", BranchKind::from(issue_type).prefix())
}

/// The author emails of `lines` (`%ae<TAB>%cn<TAB>%ce`, one commit per line) that are not
/// `own_email`, without duplicates. A commit committed by GitHub itself (host-published) is not foreign.
pub(crate) fn foreign_authors(lines: &str, own_email: &str) -> Vec<String> {
    let mut foreign: Vec<String> = lines
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(LOG_FIELD_SEPARATOR).map(str::trim);
            let author = fields.next().filter(|author| !author.is_empty())?;
            let by_github = fields
                .next()
                .is_some_and(|name| name == GITHUB_COMMITTER_NAME)
                && fields
                    .next()
                    .is_some_and(|email| email.eq_ignore_ascii_case(GITHUB_COMMITTER_EMAIL));
            let own = author.eq_ignore_ascii_case(own_email.trim());
            (!own && !by_github).then(|| author.to_string())
        })
        .collect();
    foreign.sort();
    foreign.dedup();
    foreign
}

/// The ticket `key` of the vault (local or a GitHub mirror) and its note path; `None` for an unknown key.
fn local_ticket(ctx: &Ctx, key: &str) -> Option<(Ticket, PathBuf)> {
    let vault = aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace())?;
    let store = TicketStore::open(&vault.root).ok()?;
    let path = store.path_of(key)?;
    let (ticket, _, _) = store.get(key).ok()?;
    Some((ticket, path))
}

/// The issue type of ticket `key`: the local ticket's, else Jira's, `Story` when unparsable.
fn issue_type_of(ctx: &Ctx, key: &str) -> anyhow::Result<IssueType> {
    if let Some((ticket, _)) = local_ticket(ctx, key) {
        return Ok(ticket.r#type);
    }
    let flow_ticket = flow_ticket(ctx, key);
    if let Some(kind) = flow_ticket
        .as_ref()
        .and_then(|ticket| ticket.get(aidp_gateway::models::constants::flow_execution::TICKET_TYPE))
        .and_then(Value::as_str)
    {
        return Ok(parse_issue_type(kind));
    }
    Ok(IssueType::Task)
}

fn flow_ticket(ctx: &Ctx, key: &str) -> Option<Value> {
    let vault = aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace())?;
    let store = aidp_gateway::flows::store::FlowStore::new(
        vault.root,
        aidp_gateway::paths::state_dir().ok()?,
    );
    let entry = store.index_get(key)?;
    store.load_state(&entry.repo, key).map(|state| state.ticket)
}

fn git_output(ctx: &Ctx, workspace: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let spec = ProcessSpec::new(GIT_PROGRAM)
        .args(args.iter().copied())
        .cwd(workspace);
    Ok(ctx.runner.output(&spec)?)
}

fn git_succeeds(ctx: &Ctx, workspace: &Path, args: &[&str]) -> anyhow::Result<bool> {
    Ok(git_output(ctx, workspace, args)?.status.success())
}

fn git_stdout(ctx: &Ctx, workspace: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = git_output(ctx, workspace, args)?;
    if !output.status.success() {
        return Err(typed(
            ExitStatus::UpstreamFailure,
            format!(
                "{GIT_FAILED_MESSAGE}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The arguments of a quiet `git rev-parse --verify` of `reference`.
fn verify_args(reference: &str) -> Vec<&str> {
    VERIFY_ARGS.iter().copied().chain([reference]).collect()
}

fn git_branch(ctx: &Ctx, key: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let ticket_key = flow_ticket(ctx, key)
        .as_ref()
        .and_then(|ticket| ticket.get(aidp_gateway::models::constants::flow_execution::TICKET_KEY))
        .and_then(Value::as_str)
        .unwrap_or(key)
        .to_string();
    let branch = branch_of(&ticket_key, issue_type_of(ctx, key)?);
    let local_ref = format!("{LOCAL_REF_PREFIX}{branch}");
    if git_succeeds(ctx, &workspace, &verify_args(&local_ref))? {
        return report_output(&FlowBranchReport {
            branch,
            created: false,
        });
    }
    let listed = git_stdout(
        ctx,
        &workspace,
        &[GIT_LS_REMOTE, GIT_HEADS_FLAG, ORIGIN_REMOTE, &branch],
    )?;
    let ticket_base = base_branch_of(ctx, key);
    if listed.trim().is_empty() {
        match ticket_base.as_deref() {
            // A bug ticket leaves its parent's branch, never the default one.
            Some(own) => {
                let start = format!("{ORIGIN_REMOTE}/{own}");
                git_stdout(
                    ctx,
                    &workspace,
                    &[GIT_SWITCH, GIT_CREATE_FLAG, &branch, &start],
                )?;
            }
            None => {
                git_stdout(ctx, &workspace, &[GIT_SWITCH, GIT_CREATE_FLAG, &branch])?;
            }
        }
        return report_output(&FlowBranchReport {
            branch,
            created: true,
        });
    }
    git_stdout(ctx, &workspace, &[GIT_FETCH, ORIGIN_REMOTE, &branch])?;
    let has_origin_head = git_succeeds(ctx, &workspace, &verify_args(ORIGIN_HEAD_REF))?;
    let own_base = ticket_base.map(|own| format!("{ORIGIN_REMOTE}/{own}"));
    let base = match own_base.as_deref() {
        Some(own) => own,
        None if has_origin_head => ORIGIN_HEAD_REF,
        None => ORIGIN_DEFAULT_REF,
    };
    let remote_ref = format!("{ORIGIN_REMOTE}/{branch}");
    let excluded_base = format!("{EXCLUDE_MARK}{base}");
    let emails = git_stdout(
        ctx,
        &workspace,
        &[
            GIT_LOG,
            &remote_ref,
            &excluded_base,
            GIT_AUTHOR_EMAIL_FORMAT,
        ],
    )?;
    let own_email = git_stdout(ctx, &workspace, &[GIT_CONFIG, GIT_USER_EMAIL])?;
    let foreign = foreign_authors(&emails, &own_email);
    if !foreign.is_empty() {
        return Err(typed(
            ExitStatus::NegativeVerdict,
            format!("{remote_ref} carries commits from {}", foreign.join(", ")),
        ));
    }
    git_stdout(ctx, &workspace, &[GIT_SWITCH, &branch])?;
    report_output(&FlowBranchReport {
        branch,
        created: false,
    })
}

/// The branch ticket `key` starts from and its pull request targets, when it is not the default one
/// (a bug ticket: its parent's branch).
pub(crate) fn base_branch_of(ctx: &Ctx, key: &str) -> Option<String> {
    local_ticket(ctx, key)?
        .0
        .github
        .and_then(|github| github.base_branch)
        .filter(|base| !base.trim().is_empty())
}

/// Publishes the worktree's branch through the GitHub API and prints `{branch, sha, changed}`.
/// A new branch of a bug ticket starts where it leaves its parent's branch.
fn git_push(ctx: &Ctx, branch: Option<&str>) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let current = gh::in_dir(&workspace, gh::current_branch);
    let base = branch
        .map(str::to_string)
        .or(current)
        .and_then(|name| aidp_integrations::branch_key::ticket_key_from_branch(&name))
        .and_then(|key| base_branch_of(ctx, &key));
    report_output(&branch_publish::publish_in(
        ctx.runner.as_ref(),
        &workspace,
        branch,
        base.as_deref(),
    )?)
}

/// Opens the draft pull request of the ticket branch unless one is open, then writes its `pr.json`
/// next to the body file, where the flow reads the pull request from.
fn pr_create(
    ctx: &Ctx,
    key: &str,
    branch: Option<&str>,
    body_file: &Path,
    draft: bool,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    gh::in_dir(&workspace, || {
        let branch = branch
            .map(str::to_string)
            .or_else(gh::current_branch)
            .ok_or_else(|| typed(ExitStatus::NotFound, NO_BRANCH_MESSAGE))?;
        let open = open_pr_of(&branch)?;
        let changed = open.is_none();
        let pull = match open {
            Some(pull) => pull,
            None => {
                let body = closing_body(ctx, key, github_body(&read_pr_body(body_file)?));
                let title = pr_title(&ticket_heading(ctx, key)?);
                let base = base_branch_of(ctx, key);
                gh::pr_create_for_head(&title, &body, &branch, base.as_deref(), draft)
                    .map_err(|error| typed(ExitStatus::UpstreamFailure, error.to_string()))?;
                open_pr_of(&branch)?
                    .ok_or_else(|| typed(ExitStatus::UpstreamFailure, NO_OPENED_PR_MESSAGE))?
            }
        };
        let text = |field: &str| pull[field].as_str().unwrap_or_default().to_string();
        let file = FlowPrFile {
            pr_number: pull[PR_NUMBER_FIELD].as_u64().unwrap_or_default(),
            pr_url: text(PR_URL_FIELD),
            repository: text(PR_REPO_FIELD),
            branch: branch.clone(),
            commit_sha: pull[PR_HEAD_SHA_FIELD].as_str().map(str::to_string),
        };
        let pr_path = body_file.with_file_name(PR_FILE_NAME);
        std::fs::write(&pr_path, serde_json::to_string_pretty(&file)?)?;
        report_output(&FlowPrReport {
            pr: pr_path.display().to_string(),
            number: file.pr_number,
            url: file.pr_url,
            branch,
            changed,
        })
    })
}

/// `body` ending with `Closes #<n>` when ticket `key` mirrors GitHub issue `<n>`: merging the pull
/// request then closes the issue.
fn closing_body(ctx: &Ctx, key: &str, body: String) -> String {
    match local_ticket(ctx, key).and_then(|(ticket, _)| ticket.github) {
        Some(meta) => aidp_tickets::github_sync::with_closing_line(&body, meta.number),
        None => body,
    }
}

/// The pull request body; creating a pull request needs it, reusing one does not.
fn read_pr_body(body_file: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(body_file).map_err(|error| {
        typed(
            ExitStatus::Usage,
            format!(
                "body file {} is required to create the pull request: {error}",
                body_file.display()
            ),
        )
    })
}

/// The body as GitHub shows it: no leading YAML front matter, wikilinks turned into plain text.
pub(crate) fn github_body(markdown: &str) -> String {
    let mut lines = markdown.split_inclusive('\n');
    let body = match lines.next() {
        Some(first) if first.trim_end() == FRONT_MATTER_FENCE => {
            let mut rest = lines.clone();
            if rest.any(|line| line.trim_end() == FRONT_MATTER_FENCE) {
                rest.collect()
            } else {
                markdown.to_string()
            }
        }
        _ => markdown.to_string(),
    };
    unlink_wikilinks(&body)
}

/// `[[target|label]]` becomes `label`; `[[target]]` becomes the last segment of `target`, extension dropped.
fn unlink_wikilinks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(WIKILINK_OPEN) {
        let inner = &rest[start + WIKILINK_OPEN.len()..];
        let closing = inner
            .find(WIKILINK_CLOSE)
            .filter(|end| !inner[..*end].contains('\n'));
        let Some(end) = closing else {
            // An unclosed `[[` stays as text; later links are still converted.
            out.push_str(&rest[..start + WIKILINK_OPEN.len()]);
            rest = inner;
            continue;
        };
        out.push_str(&rest[..start]);
        let link = &inner[..end];
        match link.split_once(WIKILINK_ALIAS_SEPARATOR) {
            Some((_, label)) => out.push_str(label),
            None => {
                let name = link.rsplit('/').next().unwrap_or(link);
                out.push_str(name.rsplit_once('.').map_or(name, |(stem, _)| stem));
            }
        }
        rest = &inner[end + WIKILINK_CLOSE.len()..];
    }
    out.push_str(rest);
    out
}

/// The open pull request whose head is `branch`; a closed or merged one does not count.
fn open_pr_of(branch: &str) -> anyhow::Result<Option<Value>> {
    let pull = gh::pr_resolve_checked(Some(branch), None)
        .map_err(|error| typed(ExitStatus::UpstreamFailure, error.to_string()))?;
    Ok(pull.filter(|pull| pull[PR_STATE_FIELD].as_str() == Some(OPEN_PR_STATE)))
}

/// `<type>(<KEY>): <summary>`, the type being `fix` for a bug and `feat` otherwise.
pub(crate) fn pr_title(heading: &TicketHeading) -> String {
    format!(
        "{}({}){TITLE_SEPARATOR}{}",
        BranchKind::from(heading.issue_type).commit_type(),
        heading.key,
        heading.summary
    )
}

/// The type, business key and summary of ticket `key`: its local note, else the flow's ticket
/// snapshot, completed from Jira when the snapshot lacks the type or the summary.
fn ticket_heading(ctx: &Ctx, key: &str) -> anyhow::Result<TicketHeading> {
    if let Some((ticket, _)) = local_ticket(ctx, key) {
        return Ok(TicketHeading {
            issue_type: ticket.r#type,
            key: ticket.key,
            summary: ticket.summary,
        });
    }
    let snapshot = flow_ticket(ctx, key).unwrap_or(Value::Null);
    let text = |field: &str| {
        snapshot
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let ticket_key = text(aidp_gateway::models::constants::flow_execution::TICKET_KEY)
        .unwrap_or_else(|| key.to_string());
    let issue_type = text(aidp_gateway::models::constants::flow_execution::TICKET_TYPE);
    let summary = text(aidp_gateway::models::constants::flow_inputs::TICKET_SUMMARY);
    let issue_type = issue_type.unwrap_or_default();
    let summary = summary.unwrap_or_else(|| ticket_key.clone());
    Ok(TicketHeading {
        issue_type: parse_issue_type(&issue_type),
        key: ticket_key,
        summary,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/flow_composite_tests.rs"]
mod flow_composite_tests;

#[cfg(test)]
#[path = "../../tests/unit/flow_catalog_tests.rs"]
mod flow_catalog_tests;

#[cfg(test)]
#[path = "../../tests/unit/flow_pr_body_tests.rs"]
mod flow_pr_body_tests;
