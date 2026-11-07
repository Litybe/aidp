//! Starting a run: id allocation, effective pipeline, initial state.

use super::layout::{save, ticket_segment};
use super::{RunState, StageState, StageStatus};
use crate::clock::SystemClock;
use crate::cost::PriceTable;
use crate::git::GitCli;
use crate::id::{IdGen, SystemIdGen};
use crate::knowledge::LEGACY_KNOWLEDGE_DIR;
use crate::models::constants::messages::UNKNOWN_REFUSAL;
use crate::models::constants::stack as stack_const;
use crate::models::enums::Stack;
use crate::models::run_state::RunPorts;
use crate::pipeline;
use crate::pipeline::Stage;
use crate::stack;
use std::collections::BTreeMap;
use std::path::Path;

const SCALAR_INPUT_KEY: &str = "value";

/// `RunState::mode` value for a run driven by the daemon instead of the Workflow tool.
pub const NATIVE_RUN_MODE: &str = "native";

pub fn new_run_id() -> String {
    SystemIdGen.new_run_id(&SystemClock)
}

/// A declared [`stack_const::INPUT`] left at [`stack_const::AUTO`] becomes the stack
/// [`stack::detect`] finds in the workspace, before `skills_by` reads it.
fn resolve_auto_stack(
    pipe: &crate::pipeline::Pipeline,
    inputs: &mut BTreeMap<String, serde_json::Value>,
    workspace_root: &Path,
) -> anyhow::Result<()> {
    let Some(spec) = pipe.inputs.get(stack_const::INPUT) else {
        return Ok(());
    };
    if inputs.get(stack_const::INPUT).and_then(|v| v.as_str()) != Some(stack_const::AUTO) {
        return Ok(());
    }
    let found = stack::detect(workspace_root);
    if !spec.values.is_empty() && !spec.values.iter().any(|v| v == found) {
        anyhow::bail!(
            "input '{}': detected {found:?} in {} but pipeline {} only allows {:?}",
            stack_const::INPUT,
            workspace_root.display(),
            pipe.id,
            spec.values
        );
    }
    inputs.insert(stack_const::INPUT.into(), serde_json::json!(found));
    Ok(())
}

/// A declared [`stack_const::INPUT`] that has no default and was left out becomes the
/// stack [`stack::detect_project`] finds in the workspace, when the pipeline allows it.
fn fill_missing_stack(
    pipe: &crate::pipeline::Pipeline,
    inputs: &mut BTreeMap<String, serde_json::Value>,
    workspace_root: &Path,
) {
    let Some(spec) = pipe.inputs.get(stack_const::INPUT) else {
        return;
    };
    if !spec.required || spec.default.is_some() || inputs.contains_key(stack_const::INPUT) {
        return;
    }
    let found = stack::detect_project(workspace_root);
    let Ok(value) = serde_json::to_value(found) else {
        return;
    };
    let allowed = spec.values.is_empty()
        || spec
            .values
            .iter()
            .any(|allowed| Some(allowed.as_str()) == value.as_str());
    if allowed {
        inputs.insert(stack_const::INPUT.into(), value);
    }
}

/// Refuses a declared [`stack_const::INPUT`] whose resolved value is [`stack_const::UNKNOWN`].
fn refuse_unknown_stack(
    pipe: &crate::pipeline::Pipeline,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> anyhow::Result<()> {
    let is_unknown = pipe.inputs.contains_key(stack_const::INPUT)
        && inputs.get(stack_const::INPUT).is_some_and(|value| {
            serde_json::from_value::<Stack>(value.clone()).ok() == Some(Stack::Unknown)
        });
    if is_unknown {
        anyhow::bail!("{}", UNKNOWN_REFUSAL);
    }
    Ok(())
}

/// Resolve every stage's `skills_by:` against `inputs` in place, writing the result into
/// `stage.skills` and clearing `skills_by` .
fn resolve_skills_by(stages: &mut [Stage], inputs: &BTreeMap<String, serde_json::Value>) {
    for stage in stages.iter_mut() {
        if let Some(sb) = stage.skills_by.take() {
            let value = inputs.get(&sb.input).and_then(|v| v.as_str());
            stage.skills = sb.resolve(&stage.skills, value);
        }
        resolve_skills_by(&mut stage.body, inputs);
    }
}

/// Start a run. `vault` decides **where everything the run produces lives**.
pub fn init(
    pipeline_path: &Path,
    workspace_root: &Path,
    inputs: serde_json::Value,
    vault: Option<&crate::vault::Vault>,
) -> anyhow::Result<RunState> {
    let clock = SystemClock;
    let ids = SystemIdGen;
    let git = GitCli;
    let prices = PriceTable::from_env_or_embedded();
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    init_with(pipeline_path, workspace_root, inputs, vault, &ports)
}

/// [`init`] with its clock/id/git ports injected — see [`RunPorts`].
pub fn init_with(
    pipeline_path: &Path,
    workspace_root: &Path,
    inputs: serde_json::Value,
    vault: Option<&crate::vault::Vault>,
    ports: &RunPorts,
) -> anyhow::Result<RunState> {
    init_with_mode(pipeline_path, workspace_root, inputs, vault, ports, None)
}

/// [`init_with`] recording `mode` on the run; `Some(NATIVE_RUN_MODE)` marks a daemon-driven run.
pub fn init_with_mode(
    pipeline_path: &Path,
    workspace_root: &Path,
    inputs: serde_json::Value,
    vault: Option<&crate::vault::Vault>,
    ports: &RunPorts,
    mode: Option<&str>,
) -> anyhow::Result<RunState> {
    let pipe = pipeline::load(pipeline_path)?;
    let run_id = ports.ids.new_run_id(ports.clock);

    let mut inputs_map: BTreeMap<String, serde_json::Value> = match inputs {
        serde_json::Value::Object(m) => m.into_iter().collect(),
        serde_json::Value::Null => BTreeMap::new(),
        other => {
            let mut m = BTreeMap::new();
            m.insert(SCALAR_INPUT_KEY.into(), other);
            m
        }
    };
    fill_missing_stack(&pipe, &mut inputs_map, workspace_root);
    let mut init_warnings = Vec::new();
    let mut inputs_map = pipeline::validate_inputs_collect(&pipe, &inputs_map, &mut init_warnings)?;
    resolve_auto_stack(&pipe, &mut inputs_map, workspace_root)?;
    refuse_unknown_stack(&pipe, &inputs_map)?;

    let roots = pipeline::discovery_roots(workspace_root, pipeline_path);
    let root_refs: Vec<&Path> = roots.iter().map(|p| p.as_path()).collect();
    let mut effective = crate::compose::expand(&pipe, &root_refs)?;
    let plan_layers = pipeline::topo_sort(&effective)
        .map_err(|error| anyhow::anyhow!("pipeline {}: {error}", pipeline_path.display()))?;

    let ticket = ticket_segment(&inputs_map, Some(&pipe));
    let repo = vault.map(|_| crate::vault::repo_segment(workspace_root));
    let pipelines_root = match (vault, &repo) {
        (Some(v), Some(r)) => {
            v.ensure_skeleton()?;
            let _ = crate::vault::ensure_repo_ignores_vault_paths(workspace_root);
            v.repo_dir(r)
        }
        _ => workspace_root.join(".aidp/pipelines"),
    };
    let knowledge_root = match (vault, &repo) {
        (Some(v), Some(r)) => v.knowledge_dir(r),
        _ => workspace_root.join(LEGACY_KNOWLEDGE_DIR),
    };
    let ticket_root = pipelines_root.join(&ticket);
    let run_dir = ticket_root.join(&run_id);
    std::fs::create_dir_all(&run_dir)?;

    resolve_skills_by(&mut effective.stages, &inputs_map);
    let eff_path = run_dir.join("effective-pipeline.yaml");
    std::fs::write(&eff_path, serde_yaml::to_string(&effective)?)?;

    let stages: Vec<StageState> = pipeline::flat_stages(&effective)
        .into_iter()
        .map(|s| StageState {
            name: s.name.clone(),
            status: StageStatus::Pending,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            tokens_used: None,
            stage_dir: match s.placement() {
                pipeline::Placement::Knowledge => knowledge_root.clone(),
                pipeline::Placement::KnowledgeFeature => {
                    crate::knowledge::features_dir(&knowledge_root)
                }
                pipeline::Placement::Ticket => ticket_root.join(&s.name),
                pipeline::Placement::Run => run_dir.join(&s.name),
            },
            outputs: Default::default(),
            summary: None,
            follow_ups: Vec::new(),
            skills_not_loaded: Vec::new(),
            usage: None,
            verification: Vec::new(),
        })
        .collect();

    let base_sha = ports.git.head(workspace_root);
    let branch = ports.git.branch(workspace_root);

    let mut state = RunState {
        run_id: run_id.clone(),
        pipeline_path: eff_path,
        pipeline_id: pipe.id.clone(),
        workspace_root: workspace_root.to_path_buf(),
        run_dir: run_dir.clone(),
        started_at: ports.clock.now_rfc3339(),
        inputs: inputs_map,
        plan: vec![],
        stages,
        loops: Default::default(),
        knowledge_graph: None,
        workflow_run_id: None,
        gates: vec![],
        pending_gate: None,
        crossed_boundaries: Vec::new(),
        last_workflow_status: None,
        plan_schedules: Default::default(),
        vault_root: vault.map(|v| v.root.clone()),
        knowledge_root: Some(knowledge_root),
        repo,
        feature: Some(ticket.clone()),
        cost: None,
        base_sha,
        head_sha_end: None,
        branch,
        commits: Vec::new(),
        scope_violations: Vec::new(),
        vault_leaks: Vec::new(),
        guard_findings: Vec::new(),
        knowledge_snapshot: None,
        init_warnings,
        mode: mode.map(str::to_string),
        autonomous: false,
        assumptions: Vec::new(),
        escalations: Vec::new(),
        model_overrides: BTreeMap::new(),
    };

    state.plan = plan_layers;

    if let Some(kg_settings) = pipe.settings.knowledge_graph.as_ref() {
        if kg_settings.should_run() {
            let kg =
                crate::kg::ensure(kg_settings, workspace_root, vault.map(|v| v.root.as_path()));
            if let Some(reason) = kg.containment_refusal() {
                anyhow::bail!("knowledge graph refused: {reason}");
            }
            state.knowledge_graph = Some(kg);
        }
    }

    save(&state)?;
    Ok(state)
}

/// Freeze [`RunState::knowledge_snapshot`] from disk, and persist it, unless this run already
/// has one.
pub fn ensure_knowledge_snapshot(state: &mut RunState) -> anyhow::Result<()> {
    if state.knowledge_snapshot.is_some() {
        return Ok(());
    }
    let pipe = pipeline::load(&state.pipeline_path)?;
    let max_bytes = crate::prompt::knowledge_max_note_bytes(&pipe);
    let mut snap = crate::knowledge::snapshot(&state.knowledge_dir(), max_bytes);
    let has_feature_writer = pipe
        .stages
        .iter()
        .flat_map(|s| std::iter::once(s).chain(s.body.iter()))
        .any(|s| {
            matches!(
                s.knowledge_layer(),
                Some(pipeline::KnowledgeLayer::Feature | pipeline::KnowledgeLayer::All)
            )
        });
    if has_feature_writer {
        snap.feature_resolution = Some(crate::prompt::feature_resolution_block(state));
    }
    state.knowledge_snapshot = Some(snap);
    save(state)
}
