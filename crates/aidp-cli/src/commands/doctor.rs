use super::vault::collect_vault_status;
use super::CommandOutput;
use crate::ctx::{Ctx, AGENTS_DIR, CLAUDE_DIR, SKILL_ROOT_ENV, TEMPLATES_DIR};
use aidp_agents::PROVIDER_VERSION_ARG;
use aidp_core::hooks::{RUN_SUMMARY_MARKER, SCOPE_CHECK_MARKER};
use aidp_core::models::enums::SkillCompat;
use aidp_core::ExitStatus;
use std::path::{Path, PathBuf};

const AIDP_NOT_ON_PATH_DETAIL: &str = "aidp is not on PATH";
const AIDP_ISSUE_SEPARATOR: &str = ", ";
const FIX_TARGET_BUILD: &str = "PATH resolves aidp to a development build; run /pipeline-setup-v7 so PATH points at the release install";
const FIX_OLDER_THAN_SOURCES: &str =
    "aidp is older than its sources; rebuild with /pipeline-setup-v7";
const RUNNING_VERSION: &str = env!("CARGO_PKG_VERSION");
const SKILL_COMPAT_UNDECLARED_DETAIL: &str = "the skill declares no min_aidp_version";
const SKILL_COMPAT_TOO_OLD_FIX: &str =
    "pull the aidp repository (AIDP_SRC) and re-run /pipeline-setup-v7";
const SKILL_COMPAT_UNREADABLE_FIX: &str =
    "write min_aidp_version as a dotted version in the frontmatter of the skill's SKILL.md";
const GIT_PROGRAM: &str = "git";
const GIT_LONGPATHS_ARGS: [&str; 4] = ["config", "--global", "--get", "core.longpaths"];
const GIT_LONGPATHS_ENABLED: &str = "true";
const GIT_LONGPATHS_OK_DETAIL: &str = "git core.longpaths is true";
const GIT_LONGPATHS_WARN_DETAIL: &str =
    "warning: git core.longpaths is not true; deep vault and worktree paths can exceed 260 characters on Windows";
const GIT_LONGPATHS_FIX: &str = "git config --global core.longpaths true";
const PROVIDER_LIST_SEPARATOR: &str = "; ";
const PROVIDERS_FIX: &str =
    "install the missing provider binary or fix its `program` in ~/.aidp/config.json";
const EXTERNAL_BINARIES: [&str; 3] = ["gh", "git", "cargo"];
const MAX_HEALTHY_FEATURE_NOTES: usize = 60;
const SETTINGS_FILE: &str = "settings.json";
const YAML_EXTENSIONS: [&str; 2] = ["yaml", "yml"];
const MARKDOWN_EXTENSIONS: [&str; 1] = ["md"];
const PIPELINE_SKILL_MARKER: &str = "ai-dev-pipeline";
const AGENT_LOCK_MARKER: &str = "agent-session.lock";
const ORCHESTRATOR_EDIT_BLOCK_MARKER: &str = "main orchestrator cannot Edit/Write";
const ORCHESTRATOR_COMMAND_LIMIT_MARKER: &str = "main orchestrator may only run aidp commands";
const AIDP_HOOK_MARKERS: [&str; 3] = [
    RUN_SUMMARY_MARKER,
    SCOPE_CHECK_MARKER,
    PIPELINE_SKILL_MARKER,
];
const GUARD_HOOK_MARKERS: [&str; 4] = [
    AGENT_LOCK_MARKER,
    SCOPE_CHECK_MARKER,
    ORCHESTRATOR_EDIT_BLOCK_MARKER,
    ORCHESTRATOR_COMMAND_LIMIT_MARKER,
];

pub struct DoctorArgs {
    pub format: Option<aidp_core::Format>,
    pub workspace_root: Option<PathBuf>,
}

/// Identifier of one doctor check, serialized as its snake_case name.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DoctorCheckName {
    AidpExecutable,
    AidpOnPath,
    SkillRoot,
    SkillCompat,
    Templates,
    Agents,
    HooksInstalled,
    PipelineDiscover,
    PipelineGuardHooksRemoved,
    Vault,
    VaultAmbiguous,
    LegacyRuns,
    KnowledgeBaseSize,
    KnowledgeHygiene,
    ExternalBinaries,
    EffortEmission,
    AgentToolsets,
    GitLongpaths,
    Providers,
}

#[derive(serde::Serialize)]
struct DoctorCheck {
    name: DoctorCheckName,
    ok: bool,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
}

/// Envelope of `aidp doctor`, fields in alphabetical order to match the wire output.
#[derive(serde::Serialize)]
struct DoctorReport<'a> {
    checks: &'a [DoctorCheck],
    ok: bool,
}

impl DoctorCheck {
    fn new(name: DoctorCheckName, ok: bool, detail: String, fix: Option<String>) -> Self {
        Self {
            name,
            ok,
            detail,
            fix,
        }
    }
}

pub fn run(ctx: &Ctx, args: DoctorArgs) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(args.workspace_root);
    let settings_path = ctx
        .home
        .as_ref()
        .map(|h| h.join(CLAUDE_DIR).join(SETTINGS_FILE));
    let mut checks: Vec<DoctorCheck> = Vec::new();
    checks.extend(install_checks(ctx));
    if let Some(sp) = &settings_path {
        checks.extend(hook_checks(sp));
    }
    checks.push(pipeline_discover_check(ctx, &workspace));
    if let Some(sp) = &settings_path {
        checks.push(guard_hooks_check(sp));
    }
    checks.extend(vault_checks(&workspace));
    checks.push(external_binaries_check());
    checks.push(effort_emission_check(ctx, &workspace));
    checks.push(check_agent_toolsets(ctx, &workspace));
    checks.push(providers_check(ctx));
    checks.extend(git_longpaths_check(ctx));

    let all_ok = checks.iter().all(|c| c.ok);
    let rendered = render_checks(args.format, &checks, all_ok)?;
    let output = CommandOutput::text(rendered);
    if !all_ok {
        return Ok(output.with_exit(ExitStatus::NegativeVerdict));
    }
    Ok(output)
}

fn render_checks(
    format: Option<aidp_core::Format>,
    checks: &[DoctorCheck],
    all_ok: bool,
) -> anyhow::Result<String> {
    let report = serde_json::to_value(DoctorReport { checks, ok: all_ok })?;
    let format = format.unwrap_or_default();
    let value = if format == aidp_core::Format::Ndjson {
        serde_json::to_value(checks)?
    } else {
        report
    };
    crate::output::render_value(format, &value)
}

/// Executable, `PATH` resolution, skill root and the template, agent and schema directories.
fn install_checks(ctx: &Ctx) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let exe = std::env::current_exe().ok();
    checks.push(DoctorCheck::new(
        DoctorCheckName::AidpExecutable,
        exe.is_some(),
        exe.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into()),
        None,
    ));
    checks.push(aidp_on_path_check(&aidp_core::aidp_binary::probe()));

    let root = ctx.skill_root.as_ref();
    checks.push(DoctorCheck::new(
        DoctorCheckName::SkillRoot,
        root.is_some(),
        root.map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into()),
        root.is_none()
            .then(|| format!("Set {SKILL_ROOT_ENV} or run aidp from the skill directory.")),
    ));

    if let Some(r) = root {
        checks.push(skill_compat_check(
            &aidp_core::skill_compat::check(r, RUNNING_VERSION),
            RUNNING_VERSION,
        ));
        let tpl_dir = r.join(TEMPLATES_DIR);
        let count = count_files(&tpl_dir, &YAML_EXTENSIONS);
        checks.push(DoctorCheck::new(
            DoctorCheckName::Templates,
            count > 0,
            format!("{} pipeline(s) at {}", count, tpl_dir.display()),
            (count == 0).then(|| {
                "templates/ directory is empty — run /pipeline-new or restore templates.".into()
            }),
        ));
        let ag_dir = r.join(AGENTS_DIR);
        let ag_count = count_files(&ag_dir, &MARKDOWN_EXTENSIONS);
        checks.push(DoctorCheck::new(
            DoctorCheckName::Agents,
            ag_count > 0,
            format!("{} agent(s) at {}", ag_count, ag_dir.display()),
            (ag_count == 0).then(|| "agents/ directory is empty — run /agent-new.".into()),
        ));
    }
    checks
}

/// Whether `running` meets the `min_aidp_version` the skill declares in its `SKILL.md`.
fn skill_compat_check(verdict: &SkillCompat, running: &str) -> DoctorCheck {
    let (ok, detail, fix) = match verdict {
        SkillCompat::Compatible { required } => (
            true,
            format!("aidp {running} meets the skill's min_aidp_version {required}"),
            None,
        ),
        SkillCompat::Undeclared => (true, SKILL_COMPAT_UNDECLARED_DETAIL.to_string(), None),
        SkillCompat::TooOld { required } => (
            false,
            format!("aidp {running} is older than the skill's min_aidp_version {required}"),
            Some(SKILL_COMPAT_TOO_OLD_FIX.to_string()),
        ),
        SkillCompat::Unreadable { reason } => (
            false,
            reason.clone(),
            Some(SKILL_COMPAT_UNREADABLE_FIX.to_string()),
        ),
    };
    DoctorCheck::new(DoctorCheckName::SkillCompat, ok, detail, fix)
}

/// Files directly under `dir` carrying one of `extensions`.
fn count_files(dir: &Path, extensions: &[&str]) -> usize {
    walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| extensions.iter().any(|wanted| x == *wanted))
                .unwrap_or(false)
        })
        .count()
}

/// Hooks installed in `~/.claude/settings.json`.
fn hook_checks(settings: &Path) -> Vec<DoctorCheck> {
    let installed = settings_has_aidp_hook(settings);
    vec![
        DoctorCheck::new(
            DoctorCheckName::HooksInstalled,
            installed,
            if installed {
                format!("aidp hooks present in {}", settings.display())
            } else {
                format!("aidp hooks NOT found in {}", settings.display())
            },
            (!installed).then(|| "Merge hooks/settings.example.json into ~/.claude/settings.json (bootstrap.ps1 does this).".into()),
        ),
    ]
}

fn pipeline_discover_check(ctx: &Ctx, workspace: &Path) -> DoctorCheck {
    let roots = ctx.pipeline_roots(workspace);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let discovered = aidp_core::pipeline::discover(&refs);
    DoctorCheck::new(
        DoctorCheckName::PipelineDiscover,
        discovered.is_ok(),
        match &discovered {
            Ok(pipelines) => format!("{} pipeline(s) discoverable", pipelines.len()),
            Err(e) => format!("discovery failed: {e}"),
        },
        None,
    )
}

/// The removed pipeline-guard hooks must stay removed: they arm on variables never set.
/// `ok` only when none is installed.
fn guard_hooks_check(settings: &Path) -> DoctorCheck {
    let guard_installed = settings_has_guard_hooks(settings);
    DoctorCheck::new(
        DoctorCheckName::PipelineGuardHooksRemoved,
        !guard_installed,
        if guard_installed {
            format!(
                "{} still carries a pipeline-guard hook (main-agent write block or `aidp scope check`). These hooks arm on AIDP_RUN/AIDP_STAGE in the hook's process environment, which is never set — they cannot fire and should be removed.",
                settings.display()
            )
        } else {
            "no pipeline-guard hooks installed (expected: they were removed as inarmable; scope is checked after the fact instead)".into()
        },
        guard_installed.then(|| "Remove the PreToolUse write-block and PostToolUse `aidp scope check` entries from your settings.json — re-merge hooks/settings.example.json to replace them.".into()),
    )
}

/// The Obsidian vault pipeline output is written to, and what lives in it.
/// No vault is a supported setup; only a configured but missing vault is flagged.
fn vault_checks(workspace: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let vs = collect_vault_status(workspace, None);
    let detail = match (&vs.root, &vs.source) {
        (Some(root), Some(src)) => format!(
            "{root} (via {src}) — {} repo(s), {} run(s); this repo maps to Pipelines/{}{}",
            vs.repos,
            vs.runs,
            vs.repo_segment,
            if vs.is_obsidian_vault {
                ""
            } else {
                "; NOT an Obsidian vault yet (no .obsidian/)"
            }
        ),
        _ => format!(
            "no vault configured — runs fall back to the legacy {}/.aidp/pipelines/ layout",
            workspace.display()
        ),
    };
    let fix = if vs.configured && !vs.exists {
        Some(
            "The configured vault directory does not exist — run `aidp vault init --vault <path>`."
                .into(),
        )
    } else if !vs.configured {
        Some(
            "Run `aidp vault init --vault <path>` to send pipeline artefacts to an Obsidian vault."
                .into(),
        )
    } else {
        None
    };
    checks.push(DoctorCheck::new(
        DoctorCheckName::Vault,
        !vs.configured || vs.exists,
        detail,
        fix,
    ));
    if let Some(a) = &vs.ambiguity {
        checks.push(DoctorCheck::new(
            DoctorCheckName::VaultAmbiguous,
            true,
            a.clone(),
            Some("Pin the vault explicitly with `aidp vault init --vault <path>` (writes ~/.aidp/config.json).".into()),
        ));
    }
    if vs.legacy_runs > 0 {
        checks.push(DoctorCheck::new(
            DoctorCheckName::LegacyRuns,
            true,
            format!(
                "{} pre-v7 run(s) still under {}/.aidp/pipelines/ — readable and resumable, not migrated",
                vs.legacy_runs,
                workspace.display()
            ),
            None,
        ));
    }
    if let Some(kd) = &vs.knowledge_dir {
        checks.push(knowledge_size_check(Path::new(kd)));
    }
    if let Some(c) = check_knowledge_hygiene(vs.knowledge_dir.as_deref().map(Path::new)) {
        checks.push(c);
    }
    checks
}

/// Knowledge-base size; never blocking, since an oversized base only risks the prompt budget.
fn knowledge_size_check(knowledge_dir: &Path) -> DoctorCheck {
    let stats = aidp_core::knowledge::stats(knowledge_dir);
    let flagged = stats.features > MAX_HEALTHY_FEATURE_NOTES || !stats.oversized.is_empty();
    DoctorCheck::new(
        DoctorCheckName::KnowledgeBaseSize,
        true,
        format!(
            "{} feature note(s), {} root note(s), {} byte(s), {} oversized (> {} lines), {} colliding handle group(s), {} without files:",
            stats.features,
            stats.root_notes,
            stats.total_bytes,
            stats.oversized.len(),
            aidp_core::knowledge::MAX_FEATURE_NOTE_LINES,
            stats.colliding_handles.len(),
            stats.without_files,
        ),
        flagged.then(|| "aidp knowledge stats".into()),
    )
}

/// Where the external binaries `aidp-integrations` shells out to resolve, without a shell fallback.
/// Non-blocking: an optional tool such as `cargo` may be absent.
fn external_binaries_check() -> DoctorCheck {
    let resolved: Vec<String> = EXTERNAL_BINARIES
        .iter()
        .map(|name| match aidp_core::program::resolve_program(name) {
            Some(p) => format!("{name} -> {}", p.display()),
            None => format!("{name} -> NOT FOUND on PATH"),
        })
        .collect();
    DoctorCheck::new(
        DoctorCheckName::ExternalBinaries,
        true,
        resolved.join("; "),
        None,
    )
}

/// Pure core of the providers check; `lookup` returns the version of a program, `None` when absent.
fn providers_check_from(
    config: Result<aidp_agents::ProvidersConfig, aidp_agents::AgentError>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> DoctorCheck {
    let config = match config {
        Ok(config) => config,
        Err(error) => {
            return DoctorCheck::new(
                DoctorCheckName::Providers,
                false,
                error.to_string(),
                Some(PROVIDERS_FIX.into()),
            )
        }
    };
    let mut missing: Vec<String> = Vec::new();
    let mut found: Vec<String> = Vec::new();
    for (id, program) in config.providers_to_probe() {
        match lookup(&program) {
            Some(version) => found.push(format!("{id} ({program}) {version}")),
            None => missing.push(format!("{id}: binary `{program}` not found")),
        }
    }
    if missing.is_empty() {
        return DoctorCheck::new(
            DoctorCheckName::Providers,
            true,
            found.join(PROVIDER_LIST_SEPARATOR),
            None,
        );
    }
    DoctorCheck::new(
        DoctorCheckName::Providers,
        false,
        missing.join(PROVIDER_LIST_SEPARATOR),
        Some(PROVIDERS_FIX.into()),
    )
}

/// First line of `<program> --version`, `None` when the program is absent from PATH or fails.
fn probe_provider_version(ctx: &Ctx, program: &str) -> Option<String> {
    aidp_core::program::resolve_program(program)?;
    let spec = aidp_core::models::process::ProcessSpec::new(program).args([PROVIDER_VERSION_ARG]);
    let output = ctx.runner.output(&spec).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().next().unwrap_or_default().trim().to_string())
}

/// Every configured agent provider must resolve to a runnable binary.
fn providers_check(ctx: &Ctx) -> DoctorCheck {
    let config = match aidp_core::vault::home_dir() {
        Some(home) => aidp_agents::ProvidersConfig::load(&home),
        None => Ok(aidp_agents::ProvidersConfig::default()),
    };
    providers_check_from(config, &|program| probe_provider_version(ctx, program))
}

/// `settings.emit_effort` per discovered pipeline; non-blocking, `false` is the stable default.
fn effort_emission_check(ctx: &Ctx, workspace: &Path) -> DoctorCheck {
    let roots = ctx.pipeline_roots(workspace);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let detail = match aidp_core::pipeline::discover(&refs) {
        Ok(found) if !found.is_empty() => found
            .iter()
            .map(|(id, path)| match aidp_core::pipeline::load(path) {
                Ok(pipe) => format!("{id}: emit_effort={}", pipe.settings.emit_effort),
                Err(e) => format!("{id}: could not load ({e})"),
            })
            .collect::<Vec<_>>()
            .join("; "),
        Ok(_) => "no pipelines discovered".into(),
        Err(e) => format!("discovery failed: {e}"),
    };
    DoctorCheck::new(DoctorCheckName::EffortEmission, true, detail, None)
}

/// Reads the global `core.longpaths` value through the process port; `None` when unset or unreadable.
fn read_git_longpaths(ctx: &Ctx) -> Option<String> {
    let spec = aidp_core::models::process::ProcessSpec::new(GIT_PROGRAM).args(GIT_LONGPATHS_ARGS);
    let output = ctx.runner.output(&spec).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Windows-only, non-blocking check on the global git `core.longpaths` setting.
fn git_longpaths_check(ctx: &Ctx) -> Option<DoctorCheck> {
    let value = read_git_longpaths(ctx);
    longpaths_check(value.as_deref(), cfg!(windows))
}

/// Pure decision: `None` off Windows, ok when the value is `true`, otherwise a warning with its fix.
/// The warning stays non-blocking (`ok: true`); doctor never edits git config.
fn longpaths_check(value: Option<&str>, is_windows: bool) -> Option<DoctorCheck> {
    if !is_windows {
        return None;
    }
    let enabled = value.is_some_and(|v| v.eq_ignore_ascii_case(GIT_LONGPATHS_ENABLED));
    Some(if enabled {
        DoctorCheck::new(
            DoctorCheckName::GitLongpaths,
            true,
            GIT_LONGPATHS_OK_DETAIL.to_string(),
            None,
        )
    } else {
        DoctorCheck::new(
            DoctorCheckName::GitLongpaths,
            true,
            GIT_LONGPATHS_WARN_DETAIL.to_string(),
            Some(GIT_LONGPATHS_FIX.to_string()),
        )
    })
}

/// Directory of the running executable, or an empty string when unknown.
fn running_exe_dir(report: &aidp_core::aidp_binary::AidpBinaryReport) -> String {
    report
        .running
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.display().to_string())
        .unwrap_or_default()
}

/// The remedy for the first issue of a failing report.
fn aidp_on_path_fix(
    report: &aidp_core::aidp_binary::AidpBinaryReport,
    issue: aidp_core::aidp_binary::AidpBinaryIssue,
) -> String {
    use aidp_core::aidp_binary::AidpBinaryIssue;
    let exe_dir = running_exe_dir(report);
    match issue {
        AidpBinaryIssue::Absent => {
            format!("Add '{exe_dir}' to your PATH (or run /pipeline-setup-v7)")
        }
        AidpBinaryIssue::TargetBuild => FIX_TARGET_BUILD.to_string(),
        AidpBinaryIssue::Mismatch | AidpBinaryIssue::OlderThanRunning => format!(
            "PATH resolves a different aidp than the one running; run /pipeline-setup-v7 or put '{exe_dir}' first on PATH"
        ),
        AidpBinaryIssue::OlderThanSources => FIX_OLDER_THAN_SOURCES.to_string(),
    }
}

/// Human text for the `aidp_on_path` check: the resolved path and build time,
/// or the issue slugs followed by the resolved path.
fn aidp_on_path_detail(report: &aidp_core::aidp_binary::AidpBinaryReport) -> String {
    let resolved = report
        .resolved
        .as_ref()
        .map(|path| path.display().to_string());
    if report.ok() {
        let path = resolved.unwrap_or_default();
        return match report.resolved_modified {
            Some(time) => format!(
                "{path} (built {})",
                chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
            ),
            None => path,
        };
    }
    let slugs: Vec<&str> = report
        .issues
        .iter()
        .map(aidp_core::aidp_binary::AidpBinaryIssue::slug)
        .collect();
    let mut detail = format!(
        "{}: {}",
        slugs.join(AIDP_ISSUE_SEPARATOR),
        resolved.unwrap_or_else(|| AIDP_NOT_ON_PATH_DETAIL.to_string())
    );
    if let Some(running) = &report.running {
        if report.resolved.as_ref() != Some(running) {
            detail.push_str(&format!(" (running: {})", running.display()));
        }
    }
    detail
}

/// Doctor check that the `aidp` found on `PATH` is the one to run.
fn aidp_on_path_check(report: &aidp_core::aidp_binary::AidpBinaryReport) -> DoctorCheck {
    DoctorCheck {
        name: DoctorCheckName::AidpOnPath,
        ok: report.ok(),
        detail: aidp_on_path_detail(report),
        fix: report
            .issues
            .first()
            .map(|issue| aidp_on_path_fix(report, *issue)),
    }
}

/// Scans feature-note metadata for prompt-injection patterns; `None` without a knowledge base.
/// Notes with `aidp_guard: off` are always listed, so the escape hatch stays visible.
fn check_knowledge_hygiene(knowledge_dir: Option<&Path>) -> Option<DoctorCheck> {
    let kd = knowledge_dir?;
    let notes = aidp_core::knowledge::list(kd);
    let flagged: Vec<&aidp_core::knowledge::FeatureNote> =
        notes.iter().filter(|n| !n.findings.is_empty()).collect();
    let worst = flagged
        .iter()
        .flat_map(|n| n.findings.iter().map(|f| f.sev))
        .max();
    let severe = matches!(
        worst,
        Some(aidp_core::guard::Sev::High) | Some(aidp_core::guard::Sev::Critical)
    );
    let guard_off: Vec<&str> = notes
        .iter()
        .filter(|n| n.guard_off)
        .map(|n| n.slug.as_str())
        .collect();
    Some(DoctorCheck {
        name: DoctorCheckName::KnowledgeHygiene,
        ok: !severe,
        detail: format!(
            "{} of {} feature note(s) flagged by the injection guard{}{}",
            flagged.len(),
            notes.len(),
            match worst {
                Some(sev) => format!(" (worst: {})", sev.as_str()),
                None => String::new(),
            },
            if guard_off.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} note(s) have aidp_guard: off — confirm a human set this ({})",
                    guard_off.len(),
                    guard_off.join(", ")
                )
            }
        ),
        fix: if severe {
            Some("aidp knowledge list — review the flagged note(s); the guard frames, it never deletes, so nothing was lost.".into())
        } else {
            None
        },
    })
}

/// Checks every `agentType` a `strict` pipeline could emit has a registry file with a `tools:` line.
/// `ok: false` is informational; `workflow emit` enforces.
fn check_agent_toolsets(ctx: &Ctx, workspace: &Path) -> DoctorCheck {
    let roots = ctx.pipeline_roots(workspace);
    let refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let registry = aidp_core::toolset::registry_dir();
    match aidp_core::pipeline::discover(&refs) {
        Ok(found) if !found.is_empty() => agent_toolsets_for(&found, registry),
        Ok(_) => DoctorCheck::new(
            DoctorCheckName::AgentToolsets,
            true,
            "no pipelines discovered (nothing to check)".into(),
            None,
        ),
        Err(e) => DoctorCheck::new(
            DoctorCheckName::AgentToolsets,
            false,
            format!("discovery failed: {e}"),
            None,
        ),
    }
}

fn agent_toolsets_for(found: &[(String, PathBuf)], registry: Option<PathBuf>) -> DoctorCheck {
    let mut needed: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut load_errors: Vec<String> = Vec::new();
    for (id, path) in found {
        match aidp_core::pipeline::load(path) {
            Ok(pipe) => {
                for t in aidp_core::toolset::strict_agent_types(&pipe) {
                    needed.entry(t).or_default().push(id.clone());
                }
            }
            Err(e) => load_errors.push(format!("{id}: {e}")),
        }
    }
    let missing: Vec<(String, Vec<String>)> = needed
        .into_iter()
        .filter(|(t, _)| match &registry {
            Some(dir) => !aidp_core::toolset::registry_has_toolset(dir, t),
            None => true,
        })
        .collect();
    let ok = missing.is_empty() && load_errors.is_empty();
    let detail = if ok {
        match &registry {
            Some(dir) => format!(
                "every restricted agentType a discovered template could emit under strict has a tools:-bearing persona in {}",
                dir.display()
            ),
            None => "no discovered template emits a restricted agentType under strict (nothing to check)".into(),
        }
    } else {
        let mut parts: Vec<String> = missing
            .iter()
            .map(|(t, ids)| format!("{t} (needed by: {})", ids.join(", ")))
            .collect();
        parts.extend(load_errors.iter().cloned());
        format!(
            "registry {}: {}",
            registry
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "location unresolved (no HOME/USERPROFILE)".into()),
            parts.join("; ")
        )
    };
    DoctorCheck::new(
        DoctorCheckName::AgentToolsets,
        ok,
        detail,
        (!ok).then(|| "aidp setup agents".into()),
    )
}

/// True when the settings file carries any aidp hook command or marker,
/// including the run-summary Stop hook.
fn settings_has_aidp_hook(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    AIDP_HOOK_MARKERS.iter().any(|marker| text.contains(marker))
}

/// True when an installed hook is one of the removed pipeline guards.
/// The harmless Stop run-summary hook carries none of these markers.
fn settings_has_guard_hooks(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| GUARD_HOOK_MARKERS.iter().any(|marker| s.contains(marker)))
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_aidp_on_path_tests.rs"]
mod doctor_aidp_on_path_tests;

#[cfg(test)]
#[path = "../../tests/unit/doctor_skill_compat_tests.rs"]
mod doctor_skill_compat_tests;

#[cfg(test)]
#[path = "../../tests/unit/doctor_longpaths_tests.rs"]
mod longpaths_tests;

#[cfg(test)]
#[path = "../../tests/unit/doctor_providers_tests.rs"]
mod doctor_providers_tests;

#[cfg(test)]
#[path = "../../tests/unit/hook_detection_tests.rs"]
mod hook_detection_tests;
