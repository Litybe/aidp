//! Obsidian note generation (v7).
//!
//! Turns a run's on-disk artefacts into a navigable set of notes. Everything
//! here is **generated** and safe to regenerate: each note is named
//! `_<folder-name>.md` (see [`crate::vault::NOTE_PREFIX`]) so it can never
//! overwrite an artifact a sub-agent wrote, and [`refresh`] is idempotent —
//! it rewrites a note only when the content actually changed, so Obsidian Sync
//! is not churned on every `aidp` call.
//!
//! Four note kinds, each linking to the next:
//!
//! | Note | Path | Holds |
//! |---|---|---|
//! | dashboard | `Pipelines/_Pipelines.md` | every repo with runs |
//! | repo MOC | `Pipelines/<repo>/_<repo>.md` | every feature of one repo |
//! | feature MOC | `…/<feature>/_<feature>.md` | every run + the shared research |
//! | run note | `…/<run_id>/_<run_id>.md` | the stage table, inputs, gates |
//! | stage note | `…/<run_id>/<stage>/_<stage>.md` | agent, summary, artefacts, links |
//!
//! Links are emitted **vault-absolute** (`[[Pipelines/repo/…/plan|plan]]`)
//! rather than by bare note name: stage folders repeat across runs, so bare
//! `[[plan]]` links would be ambiguous. Wikilinks resolve for any file type, so
//! `plan.json` is linkable the same way `plan.md` is.

use crate::clock::{Clock, SystemClock};
use crate::findings;
use crate::models::constants::obsidian::{
    FM_AGENT, FM_AIDP, FM_ANCHOR_PERSISTED, FM_COST_SOURCE, FM_COST_USD, FM_DURATION_MS,
    FM_FEATURE, FM_FEATURES, FM_FINISHED, FM_LAST_WORKFLOW_STATUS, FM_PENDING_GATE, FM_PIPELINE,
    FM_PROJECTS, FM_REPO, FM_REPOS, FM_RUNS, FM_RUN_ID, FM_SHARED_RESEARCH, FM_STAGE,
    FM_STAGES_DONE, FM_STAGES_TOTAL, FM_STARTED, FM_STATUS, FM_TAGS, FM_TOKENS, FM_TOKENS_BILLABLE,
    FM_TOPICS, FM_UPDATED, FM_WORKFLOW_RUN_ID, NOTE_KIND_DASHBOARD, NOTE_KIND_FEATURE,
    NOTE_KIND_KNOWLEDGE, NOTE_KIND_KNOWLEDGE_FEATURES, NOTE_KIND_KNOWLEDGE_INDEX, NOTE_KIND_REPO,
    NOTE_KIND_RUN, NOTE_KIND_STAGE, TAG_DASHBOARD, TAG_KNOWLEDGE_INDEX,
};
use crate::models::run_started_at::RunStartedAt;
use crate::models::run_stub::RunStub;
use crate::pipeline::{self, Pipeline, Placement};
use crate::run::{RunState, StageState, StageStatus, RUN_STATE_FILE};
use crate::vault;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Separator between the failures listed in [`refresh`]'s error.
const WRITE_FAILURE_SEPARATOR: &str = "; ";

/// Number of runs the dashboard lists under "Runs récents".
const DASHBOARD_RECENT_RUNS: usize = 25;

/// Threads scanning run states for the dashboard.
const SCAN_THREADS: usize = 8;

/// Regenerate every note touching `state`. A no-op for a legacy (non-vault) run.
///
/// Best-effort for the run, never silent: every note is attempted even when
/// an earlier one failed, and each failure is named in the returned error so
/// the caller can log it without failing the run.
pub fn refresh(state: &RunState) -> anyhow::Result<()> {
    refresh_with(state, &SystemClock)
}

/// [`refresh`], taking every note's `updated` timestamp from `clock` instead
/// of reading the wall clock directly.
pub fn refresh_with(state: &RunState, clock: &dyn Clock) -> anyhow::Result<()> {
    let Some(vault_root) = state.vault_root.clone() else {
        return Ok(());
    };
    let pipe = pipeline::load(&state.pipeline_path).ok();

    let mut outcomes = vec![write_run_note(&vault_root, state, pipe.as_ref())];
    for stage in &state.stages {
        outcomes.push(write_stage_note(&vault_root, state, stage, pipe.as_ref()));
    }
    outcomes.push(write_feature_moc(&vault_root, state, clock));
    if let (Some(dir), Some(repo)) = (&state.knowledge_root, &state.repo) {
        if dir.is_dir() {
            outcomes.push(write_knowledge_features_note(&vault_root, dir, repo, clock));
            outcomes.push(write_knowledge_project_note(&vault_root, dir, repo, clock));
            outcomes.push(write_knowledge_index(&vault_root, clock));
        }
    }
    if let Some(repo_dir) = state.run_dir.parent().and_then(Path::parent) {
        outcomes.push(write_repo_moc(&vault_root, repo_dir, clock));
    }
    outcomes.push(write_dashboard(&vault_root, clock));
    aggregate_write_failures(outcomes)
}

/// One error naming every failed note, or `Ok` when every note was written.
fn aggregate_write_failures(outcomes: Vec<Result<(), String>>) -> anyhow::Result<()> {
    let failures: Vec<String> = outcomes.into_iter().filter_map(Result::err).collect();
    if failures.is_empty() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "{} note(s) not written: {}",
        failures.len(),
        failures.join(WRITE_FAILURE_SEPARATOR)
    ))
}

/// Log one warning for a failed note write; a success prints nothing.
fn report_write_failure(outcome: Result<(), String>) {
    if let Err(error) = outcome {
        tracing::warn!(%error, "failed to write obsidian note");
    }
}

/// Write the vault dashboard on its own, without a run in hand — used by
/// `aidp vault init` (a fresh vault has no runs yet) and `aidp vault reindex`.
pub fn ensure_dashboard(vault_root: &Path) {
    let _ = std::fs::create_dir_all(vault_root.join(vault::PIPELINES_DIR));
    let _ = std::fs::create_dir_all(vault_root.join(vault::KNOWLEDGE_DIR));
    report_write_failure(write_dashboard(vault_root, &SystemClock));
    report_write_failure(write_knowledge_index(vault_root, &SystemClock));
}

/// Regenerate the knowledge-base index notes for every documented project, with
/// no run in hand — used by `aidp knowledge index` and `aidp vault reindex`.
///
/// Needed on its own because the feature layer is edited by hand as much as it is
/// written by pipelines: adding a feature note in Obsidian must be enough to see
/// it in the index, without waiting for the next run to call [`refresh`].
/// Returns the number of projects reindexed.
pub fn refresh_knowledge(vault_root: &Path) -> usize {
    let root = vault_root.join(vault::KNOWLEDGE_DIR);
    let mut n = 0;
    for dir in child_dirs(&root) {
        let Some(repo) = dir.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        report_write_failure(write_knowledge_features_note(
            vault_root,
            &dir,
            repo,
            &SystemClock,
        ));
        report_write_failure(write_knowledge_project_note(
            vault_root,
            &dir,
            repo,
            &SystemClock,
        ));
        n += 1;
    }
    report_write_failure(write_knowledge_index(vault_root, &SystemClock));
    n
}

// ---------------------------------------------------------------------------
// Run note
// ---------------------------------------------------------------------------

fn write_run_note(
    vault_root: &Path,
    state: &RunState,
    pipe: Option<&Pipeline>,
) -> Result<(), String> {
    let path = vault::index_note(&state.run_dir);
    let feature = state.feature.clone().unwrap_or_else(|| "_no-ticket".into());
    let repo = state.repo.clone().unwrap_or_else(|| "_workspace".into());
    let status = crate::run::overall_status(state);

    let mut fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_RUN.into()),
        (FM_RUN_ID, yaml_str(&state.run_id)),
        (FM_PIPELINE, yaml_str(&state.pipeline_id)),
        (FM_REPO, yaml_str(&repo)),
        (FM_FEATURE, yaml_str(&feature)),
        (FM_STATUS, status.into()),
        (FM_STARTED, yaml_str(&obsidian_datetime(&state.started_at))),
    ];
    if let Some(t) = total_tokens(state) {
        fm.push((FM_TOKENS, t.to_string()));
    }
    if let Some(cost) = &state.cost {
        if let Some(usd) = cost.total_usd {
            fm.push((FM_COST_USD, format!("{usd:.4}")));
        }
        if cost.tokens_billable > 0 {
            fm.push((FM_TOKENS_BILLABLE, cost.tokens_billable.to_string()));
        }
    }
    fm.push((FM_STAGES_TOTAL, state.stages.len().to_string()));
    fm.push((FM_STAGES_DONE, done_count(state).to_string()));
    let unpersisted_gate = unpersisted_gate(state);
    if let Some(gate) = &unpersisted_gate {
        fm.push((FM_ANCHOR_PERSISTED, "false".into()));
        fm.push((FM_PENDING_GATE, yaml_str(gate)));
    } else if let Some(g) = &state.pending_gate {
        fm.push((FM_PENDING_GATE, yaml_str(g)));
    }
    if let Some(w) = &state.workflow_run_id {
        fm.push((FM_WORKFLOW_RUN_ID, yaml_str(w)));
    }
    if let Some(ws) = &state.last_workflow_status {
        fm.push((FM_LAST_WORKFLOW_STATUS, yaml_str(ws)));
    }
    fm.push((
        "tags",
        yaml_list(&[
            "aidp/run".to_string(),
            format!("aidp/pipeline/{}", tag_safe(&state.pipeline_id)),
            format!("aidp/repo/{}", tag_safe(&repo)),
            format!("aidp/status/{status}"),
        ]),
    ));

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# Run `{}`\n\n", state.run_id));
    b.push_str(&format!(
        "Pipeline **{}** sur **{}** — feature {}.\n\n",
        state.pipeline_id,
        repo,
        link_or_text(
            vault_root,
            &vault::index_note(state.run_dir.parent().unwrap_or(&state.run_dir)),
            &feature
        ),
    ));
    if let Some(gate) = &unpersisted_gate {
        b.push_str(&anchor_warning_callout(state, gate));
    }

    if !state.inputs.is_empty() {
        b.push_str("## Inputs\n\n");
        for (k, v) in &state.inputs {
            b.push_str(&format!("- **{k}** — `{}`\n", compact_json(v)));
        }
        b.push('\n');
    }

    b.push_str("## Stages\n\n");
    b.push_str("| Stage | Agent | Statut | Tokens | $ | Artefacts |\n");
    b.push_str("|---|---|---|---|---|---|\n");
    for s in &state.stages {
        let agent = pipe
            .and_then(|p| pipeline::find_stage(p, &s.name))
            .and_then(|st| st.agent.clone())
            .unwrap_or_else(|| "—".into());
        let note = vault::index_note(&s.stage_dir);
        let name_cell = link_or_text(vault_root, &note, &s.name);
        let outs = if s.outputs.is_empty() {
            "—".to_string()
        } else {
            s.outputs
                .iter()
                .map(|(k, p)| output_link(vault_root, k, p))
                .collect::<Vec<_>>()
                .join(", ")
        };
        b.push_str(&format!(
            "| {name_cell} | {agent} | {} | {} | {} | {outs} |\n",
            status_label(s.status),
            s.tokens_used
                .map(|t| t.to_string())
                .unwrap_or_else(|| "—".into()),
            cost_cell(s.usage.as_ref()),
        ));
    }
    b.push('\n');

    if !state.gates.is_empty() {
        b.push_str("## Gates\n\n");
        for g in &state.gates {
            let when = g.asked_at.as_deref().unwrap_or("—");
            b.push_str(&format!(
                "- `{}` → **{}** ({when})",
                g.after_stage, g.decision
            ));
            if let Some(n) = &g.notes {
                b.push_str(&format!(" — {n}"));
            }
            b.push('\n');
        }
        b.push('\n');
    }

    let follow_ups: Vec<(&str, &String)> = state
        .stages
        .iter()
        .flat_map(|s| s.follow_ups.iter().map(move |f| (s.name.as_str(), f)))
        .collect();
    if !follow_ups.is_empty() {
        b.push_str("## Follow-ups\n\n");
        for (stage, f) in follow_ups {
            b.push_str(&format!("- [ ] ({stage}) {}\n", inline(f)));
        }
        b.push('\n');
    }

    if !state.plan_schedules.is_empty() {
        b.push_str("## Parallélisation (plan_tasks)\n\n");
        for (stage, sched) in &state.plan_schedules {
            b.push_str(&format!(
                "- **{stage}** — {} tâches, {} vagues, max {} en parallèle",
                sched.tasks.len(),
                sched.waves.len(),
                sched.max_parallel
            ));
            if !sched.serializations.is_empty() {
                b.push_str(&format!(
                    " ({} sérialisation(s) insérée(s))",
                    sched.serializations.len()
                ));
            }
            b.push('\n');
        }
        b.push('\n');
    }

    b.push_str(&machinery_section(state));
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

/// The gate recorded in the Workflow result file when run-state holds no
/// resume anchor, `None` once the anchor is persisted or no gate is pending.
fn unpersisted_gate(state: &RunState) -> Option<String> {
    if !crate::run::anchor_not_persisted(state) {
        return None;
    }
    crate::run::read_resume_anchor(&state.run_dir)
        .ok()
        .flatten()
        .and_then(|anchor| anchor.pending_gate)
}

/// Callout telling the reader the run waits on `gate` with no persisted
/// anchor, followed by the two commands that recover it.
fn anchor_warning_callout(state: &RunState, gate: &str) -> String {
    let result_path = state.run_dir.join(crate::run::WORKFLOW_RESULT_FILE);
    format!(
        "> [!warning] En attente de gate `{gate}`, ancre non persistée\n\
         > Le résultat Workflow enregistre la gate, mais `run-state.json` n'a pas d'ancre de reprise. Pour la récupérer :\n\
         >\n\
         > `aidp run record-resume --run {run_id} --from-result {result}`\n\
         >\n\
         > puis `/pipeline-resume-v7 {run_id}`\n\n",
        run_id = state.run_id,
        result = result_path.display(),
    )
}

/// Non-note files `aidp` keeps in the run dir. Listed rather than hidden: they
/// are the run's audit trail, and Obsidian shows them in the file explorer
/// anyway (they stay out of the graph because they are not markdown).
fn machinery_section(state: &RunState) -> String {
    let mut s = String::from("## Machinerie du run\n\n");
    for (label, name) in [
        ("état du run", "run-state.json"),
        ("pipeline effectif", "effective-pipeline.yaml"),
        ("script Workflow", "workflow.js"),
        ("schedule du plan", "plan-schedule.json"),
        ("résultat Workflow", crate::run::WORKFLOW_RESULT_FILE),
    ] {
        if state.run_dir.join(name).exists() {
            s.push_str(&format!("- {label} — `{name}`\n"));
        }
    }
    s.push('\n');
    s
}

// ---------------------------------------------------------------------------
// Stage note
// ---------------------------------------------------------------------------

/// This stage's `checks[]`, read straight off its `findings.json` (a reviewer
/// or `merge_findings:` synthesis stage's own output) — best-effort, structural
/// parse only (NOT the `jsonschema` validation `crate::findings::load` does),
/// so a note still renders for a pre-C6 findings.json, or one that doesn't
/// parse at all: `None` either way, never a fabricated count.
fn stage_findings_checks(stage: &StageState) -> Option<Vec<findings::Check>> {
    let path = stage.outputs.get("findings").cloned().or_else(|| {
        let p = stage.stage_dir.join("findings.json");
        p.is_file().then_some(p)
    })?;
    let text = std::fs::read_to_string(path).ok()?;
    let f: findings::Findings = serde_json::from_str(&text).ok()?;
    Some(f.checks)
}

fn write_stage_note(
    vault_root: &Path,
    state: &RunState,
    stage: &StageState,
    pipe: Option<&Pipeline>,
) -> Result<(), String> {
    // A stage that never ran has no directory; creating one just to hold an
    // empty note would litter the vault with folders for skipped branches.
    if !stage.stage_dir.is_dir() {
        return Ok(());
    }
    let def = pipe.and_then(|p| pipeline::find_stage(p, &stage.name));
    let placement = def.map(|d| d.placement()).unwrap_or(Placement::Run);
    let research = placement == Placement::Ticket;
    // A knowledge stage's dir is the project knowledge folder (or its
    // `Features/` subfolder), whose index note `write_knowledge_project_note` /
    // `write_knowledge_features_note` owns. Writing a stage note there would
    // fight it for the same filename on every run.
    if matches!(
        placement,
        Placement::Knowledge | Placement::KnowledgeFeature
    ) {
        return Ok(());
    }
    let path = vault::index_note(&stage.stage_dir);
    let feature = state.feature.clone().unwrap_or_else(|| "_no-ticket".into());
    let repo = state.repo.clone().unwrap_or_else(|| "_workspace".into());
    let agent = def.and_then(|d| d.agent.clone());

    let mut fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_STAGE.into()),
        (FM_STAGE, yaml_str(&stage.name)),
        (FM_RUN_ID, yaml_str(&state.run_id)),
        (FM_PIPELINE, yaml_str(&state.pipeline_id)),
        (FM_REPO, yaml_str(&repo)),
        (FM_FEATURE, yaml_str(&feature)),
        (FM_STATUS, stage.status.to_string()),
    ];
    if let Some(a) = &agent {
        fm.push((FM_AGENT, yaml_str(a)));
    }
    if research {
        fm.push((FM_SHARED_RESEARCH, "true".into()));
    }
    if let Some(t) = stage.tokens_used {
        fm.push((FM_TOKENS, t.to_string()));
    }
    if let Some(u) = &stage.usage {
        if let Some(usd) = u.cost_usd {
            fm.push((FM_COST_USD, format!("{usd:.4}")));
        }
        fm.push((FM_COST_SOURCE, yaml_str(u.source.as_str())));
    }
    if let Some(d) = stage.duration_ms {
        fm.push((FM_DURATION_MS, d.to_string()));
    }
    if let Some(t) = &stage.started_at {
        fm.push((FM_STARTED, yaml_str(&obsidian_datetime(t))));
    }
    if let Some(t) = &stage.finished_at {
        fm.push((FM_FINISHED, yaml_str(&obsidian_datetime(t))));
    }
    let mut tags = vec![
        "aidp/stage".to_string(),
        format!("aidp/status/{}", stage.status),
    ];
    if let Some(a) = &agent {
        tags.push(format!("aidp/agent/{}", tag_safe(a)));
    }
    if research {
        tags.push("aidp/research".into());
    }
    fm.push((FM_TAGS, yaml_list(&tags)));

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# {} — stage `{}`\n\n", state.run_id, stage.name));
    b.push_str(&format!(
        "Run parent : {}\n\n",
        link_or_text(
            vault_root,
            &vault::index_note(&state.run_dir),
            &state.run_id
        )
    ));

    if research {
        b.push_str("> [!info] Recherche partagée\n");
        b.push_str("> Ce dossier vit à la racine de la feature, pas dans le run : tous les runs de cette feature le lisent et l'enrichissent au lieu de refaire la recherche.\n\n");
    }

    if let Some(sum) = &stage.summary {
        b.push_str("## Résumé de l'agent\n\n");
        b.push_str(sum.trim());
        b.push_str("\n\n");
    }

    // C6 (Lot 3, Capitalisation): a best-effort read, never schema-validated —
    // this line must still render for a findings.json that predates
    // `checks[]`, or that fails to parse at all (silently skipped, then).
    // The point is to make an unevaluated control visible right in the vault,
    // next to the summary, instead of only inside the JSON a human would
    // otherwise have to go open.
    if let Some(checks) = stage_findings_checks(stage) {
        let (mut pass, mut fail, mut skip) = (0u32, 0u32, 0u32);
        for c in &checks {
            match c.status {
                findings::CheckStatus::Pass => pass += 1,
                findings::CheckStatus::Fail => fail += 1,
                findings::CheckStatus::Skip => skip += 1,
            }
        }
        b.push_str(&format!(
            "**Contrôles :** {pass} pass / {fail} fail / {skip} skip\n\n"
        ));
    }

    if let Some(d) = def {
        if !d.needs.is_empty() {
            b.push_str("## Dépend de\n\n");
            for n in &d.needs {
                let dir = state
                    .stages
                    .iter()
                    .find(|s| &s.name == n)
                    .map(|s| s.stage_dir.clone())
                    .unwrap_or_else(|| state.run_dir.join(n));
                b.push_str(&format!(
                    "- {}\n",
                    link_or_text(vault_root, &vault::index_note(&dir), n)
                ));
            }
            b.push('\n');
        }
        let consumers: Vec<&str> = pipe
            .map(|p| {
                pipeline::flat_stages(p)
                    .into_iter()
                    .filter(|s| s.needs.iter().any(|n| n == &stage.name))
                    .map(|s| s.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        if !consumers.is_empty() {
            b.push_str("## Consommé par\n\n");
            for c in consumers {
                let dir = state
                    .stages
                    .iter()
                    .find(|s| s.name == c)
                    .map(|s| s.stage_dir.clone())
                    .unwrap_or_else(|| state.run_dir.join(c));
                b.push_str(&format!(
                    "- {}\n",
                    link_or_text(vault_root, &vault::index_note(&dir), c)
                ));
            }
            b.push('\n');
        }
        if !d.skills.is_empty() {
            b.push_str(&format!(
                "**Skills chargées :** {}\n\n",
                d.skills
                    .iter()
                    .map(|s| format!("`{s}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if !stage.outputs.is_empty() {
        b.push_str("## Artefacts déclarés\n\n");
        for (k, p) in &stage.outputs {
            b.push_str(&format!("- {}\n", output_link(vault_root, k, p)));
        }
        b.push('\n');
    }

    let others = sibling_files(
        &stage.stage_dir,
        path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    );
    if !others.is_empty() {
        b.push_str("## Fichiers du dossier\n\n");
        for f in others {
            b.push_str(&format!(
                "- {}\n",
                link_or_text(vault_root, &f, &file_label(&f))
            ));
        }
        b.push('\n');
    }

    if !stage.follow_ups.is_empty() {
        b.push_str("## Follow-ups\n\n");
        for f in &stage.follow_ups {
            b.push_str(&format!("- [ ] {}\n", inline(f)));
        }
        b.push('\n');
    }

    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

/// Everything in the stage dir except the generated note itself, sorted, capped
/// so a stage that wrote hundreds of files does not produce an unreadable note.
fn sibling_files(dir: &Path, own_name: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n != own_name && !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    v.sort();
    v.truncate(40);
    v
}

// ---------------------------------------------------------------------------
// Feature MOC / repo MOC / dashboard
// ---------------------------------------------------------------------------

fn write_feature_moc(vault_root: &Path, state: &RunState, clock: &dyn Clock) -> Result<(), String> {
    let ticket_root = state.ticket_root();
    let feature = state.feature.clone().unwrap_or_else(|| "_no-ticket".into());
    let repo = state.repo.clone().unwrap_or_else(|| "_workspace".into());
    let path = vault::index_note(&ticket_root);

    let runs = scan_runs(&ticket_root);
    let research = scan_research_dirs(&ticket_root);

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_FEATURE.into()),
        (FM_FEATURE, yaml_str(&feature)),
        (FM_REPO, yaml_str(&repo)),
        (FM_RUNS, runs.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (
            "tags",
            yaml_list(&[
                "aidp/feature".to_string(),
                format!("aidp/repo/{}", tag_safe(&repo)),
            ]),
        ),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# {feature}\n\n"));
    b.push_str(&format!(
        "Tous les runs de pipeline pour cette feature sur {}.\n\n",
        link_or_text(
            vault_root,
            &vault::index_note(ticket_root.parent().unwrap_or(&ticket_root)),
            &repo
        )
    ));

    if !research.is_empty() {
        b.push_str("## Recherche partagée\n\n");
        b.push_str("Persistée à la racine de la feature et réutilisée par chaque run.\n\n");
        for d in &research {
            b.push_str(&format!(
                "- {}\n",
                link_or_text(vault_root, &vault::index_note(d), &file_label(d))
            ));
        }
        b.push('\n');
    }

    b.push_str("## Runs\n\n");
    if runs.is_empty() {
        b.push_str("_Aucun run enregistré._\n\n");
    } else {
        b.push_str("| Run | Pipeline | Statut | Démarré | Stages | $ |\n");
        b.push_str("|---|---|---|---|---|---|\n");
        for r in &runs {
            b.push_str(&format!(
                "| {} | {} | {} | {} | {}/{} | {} |\n",
                link_or_text(vault_root, &vault::index_note(&r.dir), &r.run_id),
                r.pipeline_id,
                r.status,
                obsidian_datetime(&r.started_at),
                r.stages_done,
                r.stages_total,
                run_row_cost_cell(r),
            ));
        }
        b.push('\n');
    }

    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

fn write_repo_moc(vault_root: &Path, repo_dir: &Path, clock: &dyn Clock) -> Result<(), String> {
    let repo = repo_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string();
    let path = vault::index_note(repo_dir);
    let features = child_dirs(repo_dir);

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_REPO.into()),
        (FM_REPO, yaml_str(&repo)),
        (FM_FEATURES, features.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (
            "tags",
            yaml_list(&[
                "aidp/repo-moc".to_string(),
                format!("aidp/repo/{}", tag_safe(&repo)),
            ]),
        ),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# {repo}\n\n"));
    b.push_str("## Features\n\n");
    if features.is_empty() {
        b.push_str("_Aucune feature._\n\n");
    } else {
        b.push_str("| Feature | Runs | $ |\n|---|---|---|\n");
        for f in &features {
            let runs = scan_runs(f);
            let costs: Vec<f64> = runs.iter().filter_map(|r| r.cost_usd).collect();
            let cost_cell = if costs.is_empty() {
                "—".to_string()
            } else {
                format!("{:.2} $", costs.iter().sum::<f64>())
            };
            b.push_str(&format!(
                "| {} | {} | {} |\n",
                link_or_text(vault_root, &vault::index_note(f), &file_label(f)),
                runs.len(),
                cost_cell,
            ));
        }
        b.push('\n');
    }
    let knowledge = vault_root.join(vault::KNOWLEDGE_DIR).join(&repo);
    if knowledge.is_dir() {
        b.push_str(&format!(
            "Connaissance projet : {}\n\n",
            link_or_text(
                vault_root,
                &vault::index_note(&knowledge),
                &format!("{repo} (knowledge)")
            )
        ));
    }
    b.push_str(&format!(
        "Retour au tableau de bord : {}\n\n",
        link_or_text(
            vault_root,
            &vault::index_note(repo_dir.parent().unwrap_or(repo_dir)),
            vault::PIPELINES_DIR
        )
    ));
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

// ---------------------------------------------------------------------------
// Knowledge base notes (`Knowledge/`, a sibling of `Pipelines/`)
// ---------------------------------------------------------------------------

/// `Knowledge/<repo>/_<repo>.md` — the entry point into a project's durable
/// documentation. Lists each topic (one folder per knowledge stage) and the
/// notes it holds, so the base is navigable without knowing which run wrote what.
fn write_knowledge_project_note(
    vault_root: &Path,
    dir: &Path,
    repo: &str,
    clock: &dyn Clock,
) -> Result<(), String> {
    let path = vault::index_note(dir);
    let features = crate::knowledge::list(dir);
    let features_dir = crate::knowledge::features_dir(dir);
    // `Features/` has its own index note; listing it again under "Sujets" would
    // read as a second, different thing.
    let topics: Vec<PathBuf> = child_dirs(dir)
        .into_iter()
        .filter(|d| *d != features_dir)
        .collect();

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_KNOWLEDGE.into()),
        (FM_REPO, yaml_str(repo)),
        (FM_FEATURES, features.len().to_string()),
        (FM_TOPICS, topics.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (
            "tags",
            yaml_list(&[
                "aidp/knowledge".to_string(),
                format!("aidp/repo/{}", tag_safe(repo)),
            ]),
        ),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# {repo} — connaissance projet\n\n"));
    b.push_str("Documentation durable du service : ce qui reste vrai d'un ticket à l'autre. Les pipelines écrivent ici depuis leurs stages `knowledge:`, et **relisent** ce dossier avant toute recherche ou planification.\n\n");
    b.push_str("Deux couches : les **notes transverses** ci-dessous (ce que le service *est*) et une **note par feature métier** dans `Features/` — comment une capacité métier donnée fonctionne réellement : règles, flux, fichiers, cas limites. Un agent qui implémente un ticket lit la note de sa feature ; un agent qui découvre le service lit les notes transverses.\n\n");
    b.push_str("> [!tip] Éditable à la main\n> Corriger ou compléter une note ici bénéficie à tous les runs suivants : rien ne l'écrase, les stages mettent à jour en place.\n\n");

    let root_files = artifact_files(dir);
    if !root_files.is_empty() {
        b.push_str("## Notes transverses\n\n");
        for f in &root_files {
            b.push_str(&format!(
                "- {}\n",
                link_or_text(vault_root, f, &file_label(f))
            ));
        }
        b.push('\n');
    }

    b.push_str("## Features métier\n\n");
    if features.is_empty() {
        b.push_str("_Aucune feature documentée — le prochain run de dev en créera une._\n\n");
    } else {
        b.push_str(&format!(
            "Index complet : {}\n\n",
            link_or_text(vault_root, &vault::index_note(&features_dir), "Features")
        ));
        b.push_str("| Feature | Statut | Tickets |\n|---|---|---|\n");
        for f in &features {
            b.push_str(&format!(
                "| {} | {} | {} |\n",
                link_or_text(vault_root, &f.path, &inline(&f.name)),
                inline(&f.status),
                if f.tickets.is_empty() {
                    "—".to_string()
                } else {
                    inline(&f.tickets.join(", "))
                },
            ));
        }
        b.push('\n');
    }

    // "Sujets" only exists when something was actually filed under a subfolder:
    // an empty section headed "Sujets" reads as a missing feature of the base
    // rather than as the absence of subfolders, which is the normal state.
    if !topics.is_empty() {
        b.push_str("## Autres sujets\n\n");
        for t in &topics {
            b.push_str(&format!(
                "### {}\n\n",
                link_or_text(vault_root, &vault::index_note(t), &file_label(t))
            ));
            let files = artifact_files(t);
            if files.is_empty() {
                b.push_str("_(vide)_\n\n");
                continue;
            }
            for f in files {
                b.push_str(&format!(
                    "- {}\n",
                    link_or_text(vault_root, &f, &file_label(&f))
                ));
            }
            b.push('\n');
        }
    } else if root_files.is_empty() && features.is_empty() {
        b.push_str("_Base vide — lancer `knowledge-init` sur ce service pour l'amorcer._\n\n");
    }

    b.push_str(&format!(
        "Runs de ce repo : {}\n\n",
        link_or_text(
            vault_root,
            &vault::index_note(&vault_root.join(vault::PIPELINES_DIR).join(repo)),
            repo
        )
    ));
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

/// `Knowledge/<repo>/Features/_Features.md` — the index of the per-feature layer.
///
/// Two tables, built from each note's own frontmatter so they stay a
/// by-product of the notes rather than a second thing to maintain: **Features
/// actives** (what `resolve` can update) and, only when non-empty,
/// **Historique — supersédées** (kept visible, never a target — see
/// [`crate::knowledge::FeatureNote::is_superseded`]), whose extra "Remplacée
/// par" column links to `superseded_by`'s target when it resolves in this
/// base. Skipped entirely when the layer is empty: an index note for an empty
/// folder is noise, and the project note already says the layer has nothing
/// in it.
fn write_knowledge_features_note(
    vault_root: &Path,
    dir: &Path,
    repo: &str,
    clock: &dyn Clock,
) -> Result<(), String> {
    let features_dir = crate::knowledge::features_dir(dir);
    let features = crate::knowledge::list(dir);
    if features.is_empty() {
        return Ok(());
    }
    let path = vault::index_note(&features_dir);

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_KNOWLEDGE_FEATURES.into()),
        (FM_REPO, yaml_str(repo)),
        (FM_FEATURES, features.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (
            "tags",
            yaml_list(&[
                "aidp/knowledge-features".to_string(),
                format!("aidp/repo/{}", tag_safe(repo)),
            ]),
        ),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str(&format!("# {repo} — features métier\n\n"));
    b.push_str("Une note par **feature métier** : ce que la feature fait, ses règles, son flux, les fichiers qui la portent, ses cas limites. C'est la granularité qu'un agent lit avant d'implémenter un ticket — les notes transverses du dossier parent disent ce que le service *est*, celles-ci disent comment une capacité métier *fonctionne*.\n\n");
    b.push_str("> [!info] Comment ces notes sont maintenues\n> À chaque run de dev, `aidp knowledge resolve` décide si le travail porte sur une feature déjà documentée (la note est mise à jour en place) ou sur une nouvelle (une note est créée). Les `aliases:` et `tickets:` du frontmatter sont ce qui rend cette décision fiable : les compléter à la main évite les doublons.\n\n");

    // Split active from superseded: a superseded note stays in the base (it
    // is the trace of how the feature used to work — see
    // `crate::knowledge::Decision::Supersede`), but listing it beside the
    // notes an agent should actually update would make it look like a live
    // target. The active table lists only what `resolve` can still update.
    let active: Vec<&crate::knowledge::FeatureNote> =
        features.iter().filter(|f| !f.is_superseded()).collect();
    let history: Vec<&crate::knowledge::FeatureNote> =
        features.iter().filter(|f| f.is_superseded()).collect();

    b.push_str("## Features actives\n\n");
    if active.is_empty() {
        b.push_str(
            "_Aucune feature active — tout est supersédé, voir l'historique ci-dessous._\n\n",
        );
    } else {
        b.push_str("| Feature | Statut | Domaine | Tickets | MAJ |\n|---|---|---|---|---|\n");
        for f in &active {
            b.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                link_or_text(vault_root, &f.path, &inline(&f.name)),
                inline(&f.status),
                f.area.as_deref().map(inline).unwrap_or_else(|| "—".into()),
                if f.tickets.is_empty() {
                    "—".to_string()
                } else {
                    inline(&f.tickets.join(", "))
                },
                f.updated
                    .as_deref()
                    .map(inline)
                    .unwrap_or_else(|| "—".into()),
            ));
        }
        b.push('\n');
    }

    if !history.is_empty() {
        b.push_str("## Historique — supersédées\n\n");
        b.push_str("Ces notes restent en place et lisibles à dessein : elles tracent comment la feature fonctionnait avant, ce qu'un agent doit connaître pour ne pas réintroduire l'ancien design. `resolve` ne les propose plus jamais comme cible de mise à jour.\n\n");
        b.push_str("| Feature | Statut | Domaine | Tickets | MAJ | Remplacée par |\n|---|---|---|---|---|---|\n");
        for f in &history {
            let replacement = match &f.superseded_by {
                Some(by) => {
                    let by_slug = crate::knowledge::slugify(by);
                    match features.iter().find(|n| n.slug == by_slug) {
                        Some(target) => {
                            link_or_text(vault_root, &target.path, &inline(&target.name))
                        }
                        None => format!("`{}` (introuvable)", inline(by)),
                    }
                }
                None => "—".to_string(),
            };
            b.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                link_or_text(vault_root, &f.path, &inline(&f.name)),
                inline(&f.status),
                f.area.as_deref().map(inline).unwrap_or_else(|| "—".into()),
                if f.tickets.is_empty() {
                    "—".to_string()
                } else {
                    inline(&f.tickets.join(", "))
                },
                f.updated
                    .as_deref()
                    .map(inline)
                    .unwrap_or_else(|| "—".into()),
                replacement,
            ));
        }
        b.push('\n');
    }

    let with_aliases: Vec<&crate::knowledge::FeatureNote> =
        features.iter().filter(|f| !f.aliases.is_empty()).collect();
    if !with_aliases.is_empty() {
        b.push_str("## Autres noms\n\n");
        for f in with_aliases {
            b.push_str(&format!(
                "- **{}** — {}\n",
                inline(&f.name),
                inline(&f.aliases.join(", "))
            ));
        }
        b.push('\n');
    }

    b.push_str(&format!(
        "Connaissance transverse du service : {}\n\n",
        link_or_text(vault_root, &vault::index_note(dir), repo)
    ));
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

/// `Knowledge/_Knowledge.md` — index of every documented project.
fn write_knowledge_index(vault_root: &Path, clock: &dyn Clock) -> Result<(), String> {
    let root = vault_root.join(vault::KNOWLEDGE_DIR);
    let path = vault::index_note(&root);
    let projects = child_dirs(&root);

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_KNOWLEDGE_INDEX.into()),
        (FM_PROJECTS, projects.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (FM_TAGS, yaml_list(&[TAG_KNOWLEDGE_INDEX.to_string()])),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str("# Connaissance projet\n\n");
    b.push_str("Un dossier par projet, indépendant des runs de pipeline. C'est la source de contexte que les pipelines lisent en premier.\n\n");
    if projects.is_empty() {
        b.push_str("_Aucun projet documenté._\n\n");
    } else {
        b.push_str("| Projet | Features | Sujets |\n|---|---|---|\n");
        for p in &projects {
            b.push_str(&format!(
                "| {} | {} | {} |\n",
                link_or_text(vault_root, &vault::index_note(p), &file_label(p)),
                crate::knowledge::list(p).len(),
                child_dirs(p).len()
            ));
        }
        b.push('\n');
    }
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

/// Files directly inside `dir`, sorted, excluding generated index notes and the
/// stage prompt — i.e. what an agent actually produced.
fn artifact_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| {
                    !n.starts_with(vault::NOTE_PREFIX) && !n.starts_with('.') && n != "prompt.md"
                })
                .unwrap_or(false)
        })
        .collect();
    out.sort();
    out
}

fn write_dashboard(vault_root: &Path, clock: &dyn Clock) -> Result<(), String> {
    let pipelines = vault_root.join(vault::PIPELINES_DIR);
    let path = vault::index_note(&pipelines);
    let repos = child_dirs(&pipelines);
    let repo_features: Vec<Vec<PathBuf>> = repos.iter().map(|r| child_dirs(r)).collect();
    let repo_stubs = scan_repo_run_stubs(&repo_features);

    let fm: Vec<(&str, String)> = vec![
        (FM_AIDP, NOTE_KIND_DASHBOARD.into()),
        (FM_REPOS, repos.len().to_string()),
        (FM_UPDATED, yaml_str(&now_datetime(clock))),
        (FM_TAGS, yaml_list(&[TAG_DASHBOARD.to_string()])),
    ];

    let mut b = String::new();
    b.push_str(&frontmatter(&fm));
    b.push_str("# Pipelines AIDP\n\n");
    b.push_str("Tout ce que les pipelines `ai-dev-pipeline-v7` produisent atterrit ici, un dossier par repo.\n\n");
    b.push_str("## Repos\n\n");
    if repos.is_empty() {
        b.push_str("_Aucun run pour l'instant._\n\n");
    } else {
        b.push_str("| Repo | Features | Runs |\n|---|---|---|\n");
        for ((r, features), stubs) in repos.iter().zip(&repo_features).zip(&repo_stubs) {
            b.push_str(&format!(
                "| {} | {} | {} |\n",
                link_or_text(vault_root, &vault::index_note(r), &file_label(r)),
                features.len(),
                stubs.len()
            ));
        }
        b.push('\n');
    }

    b.push_str("## Runs récents\n\n");
    let mut stubs: Vec<&RunStub> = repo_stubs.iter().flatten().collect();
    stubs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    stubs.truncate(DASHBOARD_RECENT_RUNS);
    let all: Vec<RunRow> = stubs
        .into_iter()
        .filter_map(|stub| read_run_row(stub.dir.clone()))
        .collect();
    if all.is_empty() {
        b.push_str("_Aucun run._\n\n");
    } else {
        b.push_str("| Run | Pipeline | Statut | Démarré | $ |\n|---|---|---|---|---|\n");
        for r in &all {
            b.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                link_or_text(vault_root, &vault::index_note(&r.dir), &r.run_id),
                r.pipeline_id,
                r.status,
                obsidian_datetime(&r.started_at),
                run_row_cost_cell(r),
            ));
        }
        b.push('\n');
    }

    // Tickets: a sibling root of Pipelines/ and Knowledge/, written by `aidp ticket`
    // (crate aidp-tickets), never by a run. The literal is duplicated in
    // aidp-tickets on purpose: that crate must not depend on aidp-core, so it
    // cannot share a const with vault.rs.
    if vault_root.join("TODO").is_dir() {
        b.push_str("## Tickets\n\n");
        b.push_str(&format!(
            "- {}\n\n",
            link_or_text(
                vault_root,
                &vault_root.join("TODO").join("_TODO.md"),
                "Tickets"
            )
        ));
    }

    b.push_str("## Conventions\n\n");
    b.push_str("- Les notes préfixées `_` sont **générées par `aidp`** et réécrites à chaque run — ne pas les éditer à la main.\n");
    b.push_str("- Tout le reste (`plan.md`, `findings.json`, `summary.md`, …) est produit par les sous-agents.\n");
    b.push_str("- Recherche par propriété : `aidp: run`, `aidp: stage`, `status: …`, `repo: …`, ou par tag `#aidp/run`.\n\n");
    b.push_str(&generated_footer());
    write_if_changed(&path, &b).map(|_| ())
}

// ---------------------------------------------------------------------------
// Filesystem scanning
// ---------------------------------------------------------------------------

struct RunRow {
    run_id: String,
    dir: PathBuf,
    pipeline_id: String,
    status: String,
    started_at: String,
    stages_done: usize,
    stages_total: usize,
    /// (total_usd, source-ish label) — `None` when the run carries no cost
    /// rollup at all (a pre-cost run-state, or the ledger was unavailable for
    /// every stage), so callers render `—` rather than a fabricated `0.0000`.
    cost_usd: Option<f64>,
}

/// Immediate sub-directories, sorted, excluding Obsidian's own folders.
fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| e.file_name().to_str().is_some_and(|n| !n.starts_with('.')))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// Direct children of a feature dir that hold a `run-state.json` — i.e. runs,
/// as opposed to the shared-research stage folders that sit beside them.
fn scan_runs(feature_dir: &Path) -> Vec<RunRow> {
    let mut rows: Vec<RunRow> = child_dirs(feature_dir)
        .into_iter()
        .filter_map(read_run_row)
        .collect();
    rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    rows
}

fn read_run_row(dir: PathBuf) -> Option<RunRow> {
    let text = std::fs::read_to_string(dir.join(RUN_STATE_FILE)).ok()?;
    let st: RunState = serde_json::from_str(&text).ok()?;
    Some(RunRow {
        run_id: st.run_id.clone(),
        dir,
        pipeline_id: st.pipeline_id.clone(),
        status: crate::run::overall_status(&st).to_string(),
        started_at: st.started_at.clone(),
        stages_done: done_count(&st),
        stages_total: st.stages.len(),
        cost_usd: st.cost.as_ref().and_then(|c| c.total_usd),
    })
}

/// Run stubs per repo, in input order, scanned on a few threads (I/O-bound).
fn scan_repo_run_stubs(repo_features: &[Vec<PathBuf>]) -> Vec<Vec<RunStub>> {
    let features: Vec<(usize, &PathBuf)> = repo_features
        .iter()
        .enumerate()
        .flat_map(|(repo, dirs)| dirs.iter().map(move |dir| (repo, dir)))
        .collect();
    let chunk_len = features.len().div_ceil(SCAN_THREADS).max(1);
    let mut stubs: Vec<Vec<RunStub>> = repo_features.iter().map(|_| Vec::new()).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = features
            .chunks(chunk_len)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(repo, dir)| (*repo, scan_run_stubs(dir)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for worker in workers {
            for (repo, found) in worker.join().unwrap_or_default() {
                stubs[repo].extend(found);
            }
        }
    });
    stubs
}

fn scan_run_stubs(feature_dir: &Path) -> Vec<RunStub> {
    child_dirs(feature_dir)
        .into_iter()
        .filter_map(|dir| {
            let text = std::fs::read_to_string(dir.join(RUN_STATE_FILE)).ok()?;
            let head: RunStartedAt = serde_json::from_str(&text).ok()?;
            Some(RunStub {
                dir,
                started_at: head.started_at,
            })
        })
        .collect()
}

/// Shared-research folders: direct children of the feature dir that are *not*
/// runs (no `run-state.json`).
fn scan_research_dirs(feature_dir: &Path) -> Vec<PathBuf> {
    child_dirs(feature_dir)
        .into_iter()
        .filter(|d| !d.join("run-state.json").is_file())
        .collect()
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn frontmatter(pairs: &[(&str, String)]) -> String {
    let mut s = String::from("---\n");
    for (k, v) in pairs {
        s.push_str(&format!("{k}: {v}\n"));
    }
    s.push_str("---\n\n");
    s
}

fn generated_footer() -> String {
    "\n---\n*Note générée par `aidp` — toute édition manuelle sera écrasée au prochain run.*\n"
        .to_string()
}

/// Quote a scalar only when YAML would otherwise mis-read it.
fn yaml_str(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.chars()
            .next()
            .map(|c| !c.is_ascii_alphanumeric())
            .unwrap_or(true)
        || s.contains(|c: char| {
            matches!(
                c,
                ':' | '#'
                    | '{'
                    | '}'
                    | '['
                    | ']'
                    | ','
                    | '&'
                    | '*'
                    | '!'
                    | '|'
                    | '>'
                    | '\''
                    | '"'
                    | '%'
                    | '@'
                    | '`'
            )
        })
        || matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off"
        );
    if needs_quotes {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn yaml_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| yaml_str(i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Obsidian tags allow letters, digits, `_`, `-`, `/`. Anything else breaks the
/// tag silently, so it becomes `-`.
fn tag_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// A vault-absolute wikilink when the target is inside the vault, otherwise the
/// raw path in code ticks — outputs may legitimately point at repo files or,
/// for `pr_url`-style outputs, at an external ID.
fn link_or_text(vault_root: &Path, target: &Path, alias: &str) -> String {
    match vault::rel_to(vault_root, target) {
        Some(rel) => {
            let rel = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            format!("[[{rel}|{alias}]]")
        }
        None => format!("`{}`", target.display()),
    }
}

fn output_link(vault_root: &Path, name: &str, value: &Path) -> String {
    let raw = value.to_string_lossy();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return format!("[{name}]({raw})");
    }
    match vault::rel_to(vault_root, value) {
        Some(rel) => {
            let display = rel.strip_suffix(".md").unwrap_or(&rel);
            format!("[[{display}|{name}]]")
        }
        None => format!("{name} — `{raw}`"),
    }
}

fn file_label(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

/// Flatten to one line: a summary or follow-up dropped verbatim into a table
/// cell or list item would otherwise break the markdown around it.
fn inline(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
        .replace('|', "\\|")
        .trim()
        .to_string()
}

fn compact_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => inline(s),
        other => inline(&serde_json::to_string(other).unwrap_or_default()),
    }
}

fn status_label(s: StageStatus) -> &'static str {
    match s {
        StageStatus::Pending => "⏳ pending",
        StageStatus::InProgress => "▶️ in_progress",
        StageStatus::Success => "✅ success",
        StageStatus::Cancel => "❌ cancel",
        StageStatus::Skipped => "⏭️ skipped",
    }
}

fn done_count(state: &RunState) -> usize {
    state
        .stages
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                StageStatus::Success | StageStatus::Cancel | StageStatus::Skipped
            )
        })
        .count()
}

fn total_tokens(state: &RunState) -> Option<u64> {
    let sum: u64 = state.stages.iter().filter_map(|s| s.tokens_used).sum();
    (sum > 0).then_some(sum)
}

/// Render a stage's cost with its provenance right next to the figure — a
/// number without `source` is a suspect number (mirrors `total_tokens`'
/// "hide zero" pattern so a stage with no usage attached shows `—`, not
/// `0.0000 $ (unavailable)`).
fn cost_cell(usage: Option<&crate::run::StageUsage>) -> String {
    let Some(u) = usage else {
        return "—".to_string();
    };
    match u.cost_usd {
        Some(usd) => format!("{usd:.2} $ ({})", u.source),
        None => format!("— ({})", u.source),
    }
}

/// Same "no figure without provenance" rule as [`cost_cell`], for a
/// [`RunRow`] — which only carries the run-level total, not a per-stage
/// source, so a run with a total is labeled `jsonl`/mixed rather than a bare
/// number, and a run with none shows `—`.
fn run_row_cost_cell(r: &RunRow) -> String {
    match r.cost_usd {
        Some(usd) => format!("{usd:.2} $"),
        None => "—".to_string(),
    }
}

/// Obsidian's Properties UI recognises `YYYY-MM-DDTHH:MM:SS` as a datetime; the
/// RFC3339 the run state stores (with its `+00:00` offset) is read as plain
/// text, which loses sorting and date filtering.
fn obsidian_datetime(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

fn now_datetime(clock: &dyn Clock) -> String {
    clock.now_utc().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Write only on a real change: `refresh` runs on every `aidp` call touching the
/// run, and rewriting identical bytes would make Obsidian Sync re-upload (and
/// the file-recovery plugin snapshot) every note each time. `Ok(true)` means
/// written, `Ok(false)` unchanged, `Err` names the path and the I/O error.
fn write_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path)
        .map(|old| old == content)
        .unwrap_or(false)
    {
        return Ok(false);
    }
    let describe = |error: std::io::Error| format!("{}: {error}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    std::fs::write(path, content).map_err(describe)?;
    Ok(true)
}

/// Map of stage name → its declared outputs, for callers that want the artifact
/// index without re-reading the pipeline.
pub fn outputs_index(state: &RunState) -> BTreeMap<String, Vec<String>> {
    state
        .stages
        .iter()
        .map(|s| (s.name.clone(), s.outputs.keys().cloned().collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_scalars_are_quoted_only_when_needed() {
        assert_eq!(yaml_str("PIXEL-1234"), "PIXEL-1234");
        assert_eq!(yaml_str("dev-csharp"), "dev-csharp");
        // A leading underscore starts no YAML construct, but `_no-ticket` next
        // to `[a, b]` lists reads better quoted, and `off`/`no` are YAML booleans.
        assert_eq!(yaml_str("_no-ticket"), "\"_no-ticket\"");
        assert_eq!(yaml_str("off"), "\"off\"");
        assert_eq!(yaml_str("a: b"), "\"a: b\"");
    }

    #[test]
    fn tags_drop_characters_obsidian_cannot_parse() {
        assert_eq!(tag_safe("dev-csharp"), "dev-csharp");
        assert_eq!(tag_safe("aidp/repo/cheat alerts"), "aidp/repo/cheat-alerts");
        assert_eq!(tag_safe("PIXEL+1234"), "PIXEL-1234");
    }

    // Stage folders repeat across runs, so a bare `[[plan]]` would be ambiguous:
    // links must carry the full vault-relative path.
    #[test]
    fn links_are_vault_absolute_and_drop_the_md_extension() {
        let vault = Path::new("/vault");
        let note = Path::new("/vault/Pipelines/repo/PIXEL-1/run-x/plan/_plan.md");
        assert_eq!(
            link_or_text(vault, note, "plan"),
            "[[Pipelines/repo/PIXEL-1/run-x/plan/_plan|plan]]"
        );
    }

    #[test]
    fn artefacts_outside_the_vault_degrade_to_text_not_a_broken_link() {
        let vault = Path::new("/vault");
        assert_eq!(
            output_link(vault, "changed", Path::new("/repo/src/changed-files.json")),
            "changed — `/repo/src/changed-files.json`"
        );
        assert_eq!(
            output_link(vault, "pr_url", Path::new("https://github.com/o/r/pull/1")),
            "[pr_url](https://github.com/o/r/pull/1)"
        );
    }

    #[test]
    fn non_markdown_artefacts_keep_their_extension_in_the_link() {
        let vault = Path::new("/vault");
        assert_eq!(
            output_link(
                vault,
                "plan",
                Path::new("/vault/Pipelines/r/F/run/plan/plan.json")
            ),
            "[[Pipelines/r/F/run/plan/plan.json|plan]]"
        );
    }

    #[test]
    fn multiline_summaries_cannot_break_the_surrounding_markdown() {
        assert_eq!(inline("a\nb | c"), "a b \\| c");
    }

    #[test]
    fn rfc3339_becomes_an_obsidian_datetime_property() {
        assert_eq!(
            obsidian_datetime("2026-08-25T10:11:12+00:00"),
            "2026-08-25T10:11:12"
        );
        assert_eq!(obsidian_datetime("not-a-date"), "not-a-date");
    }

    #[test]
    fn write_if_changed_leaves_identical_files_untouched() {
        let dir = std::env::temp_dir().join(format!("aidp-obs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("n.md");

        assert_eq!(write_if_changed(&f, "one"), Ok(true));
        let first = std::fs::metadata(&f).unwrap().modified().unwrap();
        assert_eq!(write_if_changed(&f, "one"), Ok(false));
        assert_eq!(std::fs::metadata(&f).unwrap().modified().unwrap(), first);

        assert_eq!(write_if_changed(&f, "two"), Ok(true));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two");
    }

    #[test]
    fn features_index_splits_active_from_superseded_with_a_replacement_link() {
        let root = std::env::temp_dir().join(format!(
            "aidp-obs-features-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let vault_root = root.join("vault");
        let knowledge_dir = vault_root.join("Knowledge").join("repo");
        let features_dir = crate::knowledge::features_dir(&knowledge_dir);
        std::fs::create_dir_all(&features_dir).unwrap();

        std::fs::write(
            features_dir.join("plafond-de-retrait-v2.md"),
            "---\nfeature: Plafond de retrait v2\nslug: plafond-de-retrait-v2\nstatus: active\n---\n\n# Plafond de retrait v2\n",
        )
        .unwrap();
        std::fs::write(
            features_dir.join("plafond-de-retrait.md"),
            "---\nfeature: Plafond de retrait\nslug: plafond-de-retrait\nstatus: deprecated\nvalid_until: 2026-02-01\nsuperseded_by: plafond-de-retrait-v2\n---\n\n# Plafond de retrait\n",
        )
        .unwrap();

        write_knowledge_features_note(&vault_root, &knowledge_dir, "repo", &SystemClock).unwrap();

        let note = std::fs::read_to_string(vault::index_note(&features_dir)).unwrap();
        assert!(note.contains("## Features actives"));
        assert!(note.contains("## Historique — supersédées"));
        assert!(note.contains("Remplacée par"));

        // The active feature must not appear in the history table, and vice
        // versa — cheaply checked by section-relative position.
        let active_pos = note.find("## Features actives").unwrap();
        let history_pos = note.find("## Historique").unwrap();
        assert!(note[active_pos..history_pos].contains("Plafond de retrait v2"));
        assert!(!note[active_pos..history_pos].contains("Plafond de retrait\n"));
        assert!(note[history_pos..].contains("Plafond de retrait v2")); // as the replacement link

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dashboard_links_tickets_only_when_todo_dir_exists() {
        let vault_root = std::env::temp_dir().join(format!(
            "aidp-obs-dashboard-tickets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&vault_root);
        std::fs::create_dir_all(vault_root.join(vault::PIPELINES_DIR)).unwrap();

        write_dashboard(&vault_root, &SystemClock).unwrap();
        let note =
            std::fs::read_to_string(vault::index_note(&vault_root.join(vault::PIPELINES_DIR)))
                .unwrap();
        assert!(!note.contains("## Tickets"));

        std::fs::create_dir_all(vault_root.join("TODO")).unwrap();
        write_dashboard(&vault_root, &SystemClock).unwrap();
        let note =
            std::fs::read_to_string(vault::index_note(&vault_root.join(vault::PIPELINES_DIR)))
                .unwrap();
        assert!(note.contains("[[TODO/_TODO|Tickets]]"));

        let _ = std::fs::remove_dir_all(&vault_root);
    }
}

#[cfg(test)]
#[path = "../tests/unit/obsidian_tests.rs"]
mod obsidian_tests;
