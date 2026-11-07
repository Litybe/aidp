//! v7 engine: render the typed [`ExecPlan`] of a run into a Claude Code **Workflow** JS script.

mod contracts;
mod helpers_js;
mod plan_sections;
mod schemas;

use self::contracts::{
    fanout_contracts, merge_contracts, plan_contracts, review_contracts, validate_fanout,
    validate_merge_findings, validate_plan_tasks, validate_tool_enforcement,
};
use self::helpers_js::{FANOUT_HELPERS, HELPERS, PLAN_HELPERS, POOL_HELPERS, RETRY_HELPERS};
use self::plan_sections::{
    claimed_kinds, filtered_base, merge_section, restrict_skills_for_kind, task_label,
    task_section, task_skills,
};
use self::schemas::{reply_schema_json, reply_schema_reviewer_json, with_skills_loaded};
use crate::exec_plan::refs::{parse_fanout_from, parse_plan_from};
use crate::exec_plan::{
    compile_exec_plan, ContractFlag, ExecPlan, FanoutItems, PassEnd, ReplySchemaKind, StageSpec,
    Step,
};
use crate::guard;
pub use crate::models::constants::pass::DEFAULT_MAX_STAGES_PER_PASS;
use crate::models::enums::{AgentRole, Isolation, ItemFailurePolicy, WorkflowStatus};
use crate::models::loop_request::LoopRequest;
use crate::models::plan_request::PlanRequest;
pub use crate::models::port::PlanSource;
pub use crate::models::workflow_emit::{AwaitingComposite, AwaitingPlan, DiskPlanSource, Emitted};
use crate::pipeline::{self, Pipeline, Stage};
use crate::plan;
use crate::prompt;
use crate::run::RunState;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SCHEMA_STANDARD: &str = "REPLY";
const SCHEMA_STANDARD_SKILLS: &str = "REPLY_SKILLS";
const SCHEMA_REVIEWER: &str = "REPLY_REVIEWER";
const SCHEMA_REVIEWER_SKILLS: &str = "REPLY_REVIEWER_SKILLS";
const TOP_INDENT: &str = "  ";
const NESTED_INDENT: &str = "    ";

impl PlanSource for DiskPlanSource {
    fn load(&self, path: &Path) -> anyhow::Result<Option<plan::Plan>> {
        if !path.is_file() {
            return Ok(None);
        }
        plan::load(path).map(Some)
    }
}

/// Emit the full Workflow script for an initialized run.
pub fn emit(state: &RunState) -> anyhow::Result<String> {
    Ok(compile(state)?.script)
}

/// Compile a run into a Workflow script plus the plan-scheduling metadata. Loads the pipeline from
/// `state.pipeline_path` once, then delegates to [`compile_with`].
pub fn compile(state: &RunState) -> anyhow::Result<Emitted> {
    let pipe = pipeline::load(&state.pipeline_path)?;
    compile_with(&pipe, state)
}

/// Compile an already-loaded [`Pipeline`] into a Workflow script, reading plan
/// artifacts (`plan_tasks:` stages) from disk via [`DiskPlanSource`].
pub fn compile_with(pipe: &Pipeline, state: &RunState) -> anyhow::Result<Emitted> {
    compile_with_source(pipe, state, &DiskPlanSource)
}

/// Compile an already-loaded [`Pipeline`] into a Workflow script, reading plan artifacts through
/// the injected `plans` source.
pub fn compile_with_source(
    pipe: &Pipeline,
    state: &RunState,
    plans: &dyn PlanSource,
) -> anyhow::Result<Emitted> {
    let graph_available = state
        .knowledge_graph
        .as_ref()
        .is_some_and(|graph| graph.has_graph());
    let (pipe, skill_warnings) = crate::skills::without_unavailable(pipe, graph_available);
    let pipe = pipe.as_ref();
    validate_pipeline(pipe)?;
    let exec_plan = compile_exec_plan(pipe, state, plans)?;
    let ctx = EmitCtx::new(pipe, state, plans)?;
    let mut out = EmitOut::default();
    render_plan(&ctx, &exec_plan, &mut out)?;
    Ok(Emitted {
        script: assemble(&ctx, &exec_plan, &out),
        plan_schedules: out.schedules,
        awaiting_plan: awaiting_plan_of(&exec_plan),
        awaiting_composite: awaiting_composite_of(&exec_plan),
        gates: exec_plan.gates,
        warnings: exec_plan
            .warnings
            .into_iter()
            .chain(skill_warnings)
            .collect(),
    })
}

fn awaiting_plan_of(plan: &ExecPlan) -> Option<AwaitingPlan> {
    match &plan.end {
        Some(PassEnd::AwaitingPlan {
            stage,
            producer,
            plan_path,
        }) => Some(AwaitingPlan {
            stage: stage.clone(),
            producer: producer.clone(),
            plan_path: plan_path.clone(),
        }),
        _ => None,
    }
}

fn awaiting_composite_of(plan: &ExecPlan) -> Option<AwaitingComposite> {
    match &plan.end {
        Some(PassEnd::AwaitingComposite { stage, composite }) => Some(AwaitingComposite {
            stage: stage.clone(),
            composite: composite.clone(),
        }),
        _ => None,
    }
}

/// Everything a renderer reads and never changes while it renders a pipeline.
pub struct EmitCtx<'a> {
    pub state: &'a RunState,
    pub pipe: &'a Pipeline,
    pub plans: &'a dyn PlanSource,
    pub contracts: BTreeMap<String, String>,
}

impl<'a> EmitCtx<'a> {
    /// Derives the prompt contracts from an in-memory pipeline and run
    /// state. Touches no disk; plan artifacts are only read later, through `plans`.
    pub fn new(
        pipe: &'a Pipeline,
        state: &'a RunState,
        plans: &'a dyn PlanSource,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            state,
            pipe,
            plans,
            contracts: stage_contracts(state, pipe)?,
        })
    }
}

/// Everything the renderers accumulate while they render a plan.
#[derive(Default)]
pub struct EmitOut {
    pub body: String,
    pub phases: Vec<String>,
    pub schedules: Vec<plan::ScheduleState>,
    pub has_parallel_group: bool,
}

/// Rejects every pipeline whose stage configuration cannot be emitted.
fn validate_pipeline(pipe: &Pipeline) -> anyhow::Result<()> {
    validate_fanout(pipe)?;
    validate_plan_tasks(pipe)?;
    validate_merge_findings(pipe)?;
    validate_tool_enforcement(pipe)
}

/// The prompt contract annexed to each stage, keyed by stage name. Fan-out, plan, merge and review
/// contracts for one stage are concatenated in that order.
fn stage_contracts(state: &RunState, pipe: &Pipeline) -> anyhow::Result<BTreeMap<String, String>> {
    let mut contracts = fanout_contracts(pipe)?;
    let later_layers = [
        plan_contracts(state, pipe)?,
        merge_contracts(state, pipe)?,
        review_contracts(pipe),
    ];
    for layer in later_layers {
        for (stage_name, contract) in layer {
            contracts
                .entry(stage_name)
                .and_modify(|existing| existing.push_str(&contract))
                .or_insert(contract);
        }
    }
    Ok(contracts)
}

/// Renders every step of the plan, in order.
fn render_plan(ctx: &EmitCtx, plan: &ExecPlan, out: &mut EmitOut) -> anyhow::Result<()> {
    for step in &plan.steps {
        render_step(ctx, step, TOP_INDENT, out)?;
    }
    Ok(())
}

/// Renders one step and records the phases it dispatches.
fn render_step(ctx: &EmitCtx, step: &Step, indent: &str, out: &mut EmitOut) -> anyhow::Result<()> {
    match step {
        Step::Stage(spec) => {
            out.body.push_str(&gate_guard(spec));
            render_seq_stage(ctx, spec, indent, out)?;
            out.phases.push(spec.name.clone());
        }
        Step::Parallel { stages } => {
            render_parallel_group(ctx, stages, out)?;
            out.phases.extend(stages.iter().map(|g| g.name.clone()));
            out.has_parallel_group = true;
        }
        Step::Loop {
            container,
            source_stage,
            signal,
            body_producer,
            max,
            body,
            ..
        } => {
            let source = source_stage.as_deref().zip(signal.as_deref());
            let looped = LoopRequest {
                container,
                source,
                body_producer: body_producer.as_deref(),
                max: *max,
                body,
            };
            render_loop(ctx, &looped, out)?;
        }
        Step::Fanout {
            stage,
            items,
            max,
            fold,
            merge_hint,
        } => {
            out.body.push_str(&gate_guard(stage));
            render_fanout_stage(
                ctx,
                stage,
                items,
                *max,
                *fold,
                merge_hint.as_deref(),
                indent,
                out,
            )?;
            out.phases.push(stage.name.clone());
        }
        Step::PlanTasks {
            stage,
            from,
            plan_path,
            max,
            kinds,
            merge_hint,
            skills_by_kind,
        } => {
            out.body.push_str(&gate_guard(stage));
            let request = PlanRequest {
                from,
                plan_path,
                max: *max,
                kinds,
                merge_hint: merge_hint.as_deref(),
                skills_by_kind,
            };
            render_plan_stage(ctx, stage, &request, indent, out)?;
            out.phases.push(stage.name.clone());
        }
        Step::CompositeBoundary { stage, composite } => {
            render_composite_boundary(out, stage, composite)
        }
        Step::Skipped { stage, .. } => push_skip_log(out, stage),
    }
    Ok(())
}

/// The approval guard that precedes a gated stage, empty for an ungated one.
fn gate_guard(spec: &StageSpec) -> String {
    if spec.gate.is_none() {
        return String::new();
    }
    format!(
        "  if (!__approved.includes({n})) return {{ status: \"{status}\", gate: {n}, completed: Object.keys(results), results: results, guard_findings: __guardFindings }};\n",
        status = WorkflowStatus::AwaitingGate.as_str(),
        n = js_str(&spec.name)
    )
}

/// Emits the log line that records a stage skipped at compile time.
fn push_skip_log(out: &mut EmitOut, stage: &str) {
    out.body.push_str(&format!(
        "  log({});\n",
        js_str(&format!("skip `{stage}` (when=false at compile time)"))
    ));
}

/// Stops the pass before a composite that would overflow it, telling the driver to re-emit.
fn render_composite_boundary(out: &mut EmitOut, stage: &str, composite: &str) {
    out.body.push_str(&format!(
        "  log({});\n",
        js_str(&format!(
            "composite `{composite}` (stage `{stage}`): starts a new Workflow pass — `aidp run record-resume --crossed-boundary {composite}`, then re-run `aidp workflow emit`"
        ))
    ));
    out.body.push_str(&format!(
        "  return {{ status: \"{status}\", at: {n}, composite: {c}, completed: Object.keys(results), results: results, guard_findings: __guardFindings }};\n",
        status = WorkflowStatus::AwaitingComposite.as_str(),
        n = js_str(stage),
        c = js_str(composite),
    ));
}

/// The dispatch expression for a stage: `agent(...)` normally, or a budget-
/// guarded `dispatch(..., N)` retry when `retries > 0`.
fn dispatch_expr(prompt: &str, needs: &[String], opts: &str, retries: u32) -> String {
    let arg = format!(
        "{p} + up({needs})",
        p = js_str(prompt),
        needs = js_str_array(needs)
    );
    dispatch_expr_raw(&arg, opts, retries)
}

/// Same as `dispatch_expr` but takes an already-built JS prompt expression
/// (used by fan-out shards, whose prompt is concatenated at runtime).
fn dispatch_expr_raw(arg: &str, opts: &str, retries: u32) -> String {
    if retries > 0 {
        format!("dispatch({arg}, {opts}, {retries})")
    } else {
        format!("safeAgent({arg}, {opts})")
    }
}

/// The stage prompt, plus any contract injected because of the stage's place in the pipeline.
fn stage_prompt(ctx: &EmitCtx, name: &str) -> anyhow::Result<String> {
    let mut text = prompt::build_prompt_text(ctx.pipe, ctx.state, name)?;
    if let Some(contract) = ctx.contracts.get(name) {
        text.push_str(contract);
    }
    Ok(text)
}

/// Render a plain sequential stage.
pub fn render_seq_stage(
    ctx: &EmitCtx,
    spec: &StageSpec,
    indent: &str,
    out: &mut EmitOut,
) -> anyhow::Result<()> {
    let name = &spec.name;
    let prompt = stage_prompt(ctx, name)?;
    let snap = format!("__b_{}", ident(name));
    out.body
        .push_str(&format!("{indent}phase({});\n", js_str(name)));
    out.body
        .push_str(&format!("{indent}const {snap} = spent();\n"));
    out.body.push_str(&format!(
        "{indent}results[{n}] = await {expr};\n",
        n = js_str(name),
        expr = dispatch_expr(&prompt, &spec.needs, &agent_opts(spec), spec.retries),
    ));
    out.body.push_str(&format!(
        "{indent}withTokens(results[{n}], spent() - {snap});\n",
        n = js_str(name)
    ));
    out.body.push_str(&pause_check(spec, indent));
    out.body.push_str(&merge_findings_guard(spec, indent));
    out.body.push_str(&harness_gap_guard(spec, indent));
    Ok(())
}

/// Render an intra-stage fan-out.
#[allow(clippy::too_many_arguments)]
fn render_fanout_stage(
    ctx: &EmitCtx,
    spec: &StageSpec,
    items: &FanoutItems,
    max: u32,
    fold: bool,
    merge_hint: Option<&str>,
    indent: &str,
    out: &mut EmitOut,
) -> anyhow::Result<()> {
    let name = &spec.name;
    let id = ident(name);
    let prompt = stage_prompt(ctx, name)?;
    let continue_on_failure = ctx.pipe.settings.on_item_failure == ItemFailurePolicy::Continue;
    let pool_size = max.max(1);
    let floor = ctx.pipe.settings.budget_floor_tokens;
    let retries = spec.retries;
    let soft_arg = spec.soft_pause;

    let src = match items {
        FanoutItems::FromSignal(from) => {
            let (producer, prop) = parse_fanout_from(name, from)?;
            format!("sigArr({}, {})", js_str(&producer), js_str(&prop))
        }
        FanoutItems::Static(labels) => js_str_array(labels),
    };
    let hint = merge_hint.map(js_str).unwrap_or_else(|| "null".to_string());

    let shard_arg = format!(
        "{p} + up({needs}) + shardNote(__s, __i, __sh_{id}.length)",
        p = js_str(&prompt),
        needs = js_str_array(&spec.needs)
    );
    let shard_opts = agent_opts_with_label(
        spec,
        &format!(
            "{} + \" [\" + (__i + 1) + \"/\" + __sh_{id}.length + \"] \" + __s",
            js_str(name)
        ),
    );
    let merge_arg = format!(
        "{p} + up({needs}) + mergeNote({hint}) + fanin(__fo_{id}, __sh_{id})",
        p = js_str(&prompt),
        needs = js_str_array(&spec.needs)
    );

    out.body
        .push_str(&format!("{indent}phase({});\n", js_str(name)));
    out.body
        .push_str(&format!("{indent}const __b_{id} = spent();\n"));
    let fold_arg = if fold { "" } else { ", false" };
    out.body.push_str(&format!(
        "{indent}const __sh_{id} = shards({src}, {pool_size}{fold_arg});\n"
    ));
    out.body.push_str(&format!(
        "{indent}log({} + __sh_{id}.length + \" shard(s): \" + __sh_{id}.join(\" | \"));\n",
        js_str(&format!("fan-out `{name}`: "))
    ));
    let single_expr = dispatch_expr(&prompt, &spec.needs, &agent_opts(spec), retries);
    out.body.push_str(&format!(
        "{indent}const __pl_{id} = pool({pool_size}, {floor});\n"
    ));
    out.body
        .push_str(&format!("{indent}if (__sh_{id}.length === 1) {{\n"));
    out.body.push_str(&format!(
        "{indent}  results[{n}] = await {single_expr};\n",
        n = js_str(name),
    ));
    out.body.push_str(&format!("{indent}}} else {{\n"));
    out.body.push_str(&format!(
        "{indent}  const __fo_{id} = await parallel(__sh_{id}.map((__s, __i) => () => __pl_{id}.run(() => {expr})));\n",
        expr = dispatch_expr_raw(&shard_arg, &shard_opts, retries),
    ));
    if continue_on_failure {
        out.body.push_str(&format!(
            "{indent}  for (const __r of __fo_{id}) {{ if (isPause(__r, {soft_arg})) log({n} + \": shard not completed (on_item_failure: continue, status=\" + (__r ? __r.status : \"null\") + \")\"); }}\n",
            n = js_str(name),
        ));
    } else {
        out.body.push_str(&format!(
            "{indent}  for (const __r of __fo_{id}) {{ if (isPause(__r, {soft_arg})) return pauseInfo({n}, __r); }}\n",
            n = js_str(name),
        ));
    }
    out.body.push_str(&format!(
        "{indent}  results[{n}] = await {expr};\n",
        n = js_str(name),
        expr = dispatch_expr_raw(&merge_arg, &agent_opts(spec), retries),
    ));
    out.body.push_str(&format!("{indent}}}\n"));
    out.body.push_str(&format!(
        "{indent}withTokens(results[{n}], spent() - __b_{id});\n",
        n = js_str(name)
    ));
    out.body.push_str(&pause_check(spec, indent));
    Ok(())
}

/// The post-dispatch checkpoint for one stage.
fn pause_check(spec: &StageSpec, indent: &str) -> String {
    pause_check_named(&spec.name, indent, spec.soft_pause)
}

fn pause_check_named(name: &str, indent: &str, soft: bool) -> String {
    format!(
        "{indent}if (isPause(results[{n}], {soft})) return pauseInfo({n}, results[{n}]);\n",
        n = js_str(name),
    )
}

/// A stage declaring `merge_findings:` must have produced its own `outputs.findings`.
fn merge_findings_guard(spec: &StageSpec, indent: &str) -> String {
    if !spec.contracts.contains(&ContractFlag::MergeFindings) {
        return String::new();
    }
    let n = js_str(&spec.name);
    format!("{indent}if (!results[{n}].outputs || !results[{n}].outputs.findings) return pauseInfo({n}, results[{n}]);\n")
}

/// Pauses a stage that reports unevaluated checks, unless a correction loop consumes it.
fn harness_gap_guard(spec: &StageSpec, indent: &str) -> String {
    if !spec.contracts.contains(&ContractFlag::HarnessGapPause) {
        return String::new();
    }
    let n = js_str(&spec.name);
    format!("{indent}if (sigNum({n}, \"unevaluated_checks\") > 0) return pauseInfo({n}, results[{n}]);\n")
}

/// Renders the pass-ending return of a plan-driven stage whose plan is not on disk yet.
fn render_awaiting_plan(
    spec: &StageSpec,
    producer: &str,
    plan_path: &str,
    indent: &str,
    out: &mut EmitOut,
) {
    let name = &spec.name;
    out.body.push_str(&format!(
        "{indent}log({});\n",
        js_str(&format!(
            "plan-driven stage `{name}`: `{producer}`'s task graph is not on disk yet ({plan_path}) — re-run `aidp workflow emit` to compile this stage and everything after it, then resume"
        ))
    ));
    out.body.push_str(&format!(
        "{indent}return {{ status: \"{status}\", at: {n}, producer: {p}, plan_path: {pp}, completed: Object.keys(results), results: results, guard_findings: __guardFindings }};\n",
        status = WorkflowStatus::AwaitingPlan.as_str(),
        n = js_str(name),
        p = js_str(producer),
        pp = js_str(plan_path),
    ));
}

/// Logs the plan tasks no stage of the pipeline claims, once, on the first plan-driven stage.
fn render_orphan_kinds_log(
    ctx: &EmitCtx,
    spec: &StageSpec,
    doc: &plan::Plan,
    indent: &str,
    out: &mut EmitOut,
) {
    let first_plan_stage = ctx
        .pipe
        .stages
        .iter()
        .find(|s| s.plan_tasks.is_some())
        .map(|s| s.name.as_str());
    if first_plan_stage != Some(spec.name.as_str()) {
        return;
    }
    let Some(claimed) = claimed_kinds(ctx.pipe) else {
        return;
    };
    let orphans: Vec<String> = doc
        .tasks
        .iter()
        .filter(|t| !claimed.contains(&t.kind))
        .map(|t| format!("{} ({})", t.id, t.kind))
        .collect();
    if orphans.is_empty() {
        return;
    }
    out.body.push_str(&format!(
        "{indent}log({});\n",
        js_str(&format!(
            "plan warning: {} task(s) will NOT run — no stage of this pipeline claims their kind (claimed: {}): {}",
            orphans.len(),
            claimed.iter().cloned().collect::<Vec<_>>().join(", "),
            orphans.join(", ")
        ))
    ));
}

/// Render a plan-driven stage.
fn render_plan_stage(
    ctx: &EmitCtx,
    spec: &StageSpec,
    request: &PlanRequest,
    indent: &str,
    out: &mut EmitOut,
) -> anyhow::Result<()> {
    let pipe = ctx.pipe;
    let retries = spec.retries;
    let name = &spec.name;
    let id = ident(name);
    let (producer, _) = parse_plan_from(name, request.from)?;
    let path = PathBuf::from(request.plan_path);

    let Some(doc) = ctx
        .plans
        .load(&path)
        .map_err(|e| anyhow::anyhow!("stage `{name}`: {e}"))?
    else {
        render_awaiting_plan(spec, &producer, request.plan_path, indent, out);
        return Ok(());
    };

    let sched = plan::schedule(&doc, request.max, request.kinds)
        .map_err(|e| anyhow::anyhow!("stage `{name}`: {e}"))?;
    let base = stage_prompt(ctx, name)?;

    out.body
        .push_str(&format!("{indent}phase({});\n", js_str(name)));
    out.body
        .push_str(&format!("{indent}const __base_{id} = {};\n", js_str(&base)));
    out.body
        .push_str(&format!("{indent}const __b_{id} = spent();\n"));
    out.body.push_str(&format!(
        "{indent}log({});\n",
        js_str(&format!(
            "plan `{name}`: {t} task(s) from {file} — {w} wave(s), up to {c} concurrent, {s} ordering(s) inserted by aidp",
            t = sched.tasks.len(),
            file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            w = sched.waves.len(),
            c = sched.effective_parallel(),
            s = sched.serializations.len(),
        ))
    ));
    for ser in &sched.serializations {
        out.body.push_str(&format!(
            "{indent}log({});\n",
            js_str(&format!(
                "plan `{name}`: `{}` runs after `{}` (both write {})",
                ser.after,
                ser.before,
                ser.files.join(", ")
            ))
        ));
    }
    for w in &sched.warnings {
        out.body.push_str(&format!(
            "{indent}log({});\n",
            js_str(&format!("plan `{name}` warning: {w}"))
        ));
    }
    render_orphan_kinds_log(ctx, spec, &doc, indent, out);

    let by_kind = request.skills_by_kind;
    let mut kind_bases: BTreeMap<String, String> = BTreeMap::new();
    if !by_kind.is_empty() {
        let all_skills: BTreeSet<&str> = spec.skills.iter().map(String::as_str).collect();
        let kinds_present: BTreeSet<&str> =
            sched.tasks.iter().map(|t| t.task.kind.as_str()).collect();
        for kind in kinds_present {
            let keep = restrict_skills_for_kind(&spec.skills, by_kind, kind);
            if keep.len() != all_skills.len() {
                let text = filtered_base(&base, &keep, &all_skills);
                let cname = format!("__base_{id}_{}", ident(kind));
                out.body
                    .push_str(&format!("{indent}const {cname} = {};\n", js_str(&text)));
                kind_bases.insert(kind.to_string(), cname);
            }
        }
    }
    let base_for = |kind: &str| -> String {
        kind_bases
            .get(kind)
            .cloned()
            .unwrap_or_else(|| format!("__base_{id}"))
    };
    let skills_for = |kind: &str| task_skills(&spec.skills, by_kind, kind);

    if sched.tasks.len() == 1 {
        let t = &sched.tasks[0];
        let suffix = task_section(&sched, t, true);
        let arg = format!(
            "{base_ref} + {suf} + up({needs})",
            base_ref = base_for(&t.task.kind),
            suf = js_str(&suffix),
            needs = js_str_array(&spec.needs)
        );
        out.body.push_str(&format!(
            "{indent}results[{n}] = await {expr};\n",
            n = js_str(name),
            expr = dispatch_expr_raw(
                &arg,
                &agent_opts_with_skills(
                    spec,
                    &task_label(name, &t.task),
                    &skills_for(&t.task.kind)
                ),
                retries
            ),
        ));
        out.body.push_str(&format!(
            "{indent}results[{n} + \"#\" + {tid}] = results[{n}];\n",
            n = js_str(name),
            tid = js_str(&t.task.id),
        ));
        out.body.push_str(&format!(
            "{indent}logTasks({n}, [{{ id: {tid}, result: results[{n}] }}]);\n",
            n = js_str(name),
            tid = js_str(&t.task.id),
        ));
    } else {
        out.body
            .push_str(&format!("{indent}const __tk_{id} = {{}};\n"));
        out.body.push_str(&format!(
            "{indent}const __sm_{id} = pool({}, {});\n",
            sched.effective_parallel(),
            pipe.settings.budget_floor_tokens
        ));
        for t in &sched.tasks {
            let suffix = task_section(&sched, t, false);
            let arg = format!(
                "{base_ref} + {suf} + up({needs})",
                base_ref = base_for(&t.task.kind),
                suf = js_str(&suffix),
                needs = js_str_array(&spec.needs)
            );
            out.body.push_str(&format!(
                "{indent}__tk_{id}[{tid}] = runTask({tid}, {deps}, {mutex}, __tk_{id}, __sm_{id}, {arg}, {opts}, {retries});\n",
                tid = js_str(&t.task.id),
                deps = js_str_array(&t.deps),
                mutex = js_str_array(&t.mutex_after),
                opts = agent_opts_with_skills(
                    spec,
                    &task_label(name, &t.task),
                    &skills_for(&t.task.kind)
                ),
            ));
        }
        let ids: Vec<String> = sched.tasks.iter().map(|t| t.task.id.clone()).collect();
        out.body.push_str(&format!(
            "{indent}const __tr_{id} = await settle(__tk_{id}, {});\n",
            js_str_array(&ids)
        ));
        out.body.push_str(&format!(
            "{indent}for (const __t of __tr_{id}) results[{n} + \"#\" + __t.id] = __t.result;\n",
            n = js_str(name)
        ));
        out.body.push_str(&format!(
            "{indent}logTasks({n}, __tr_{id});\n",
            n = js_str(name)
        ));
        if pipe.settings.on_item_failure != ItemFailurePolicy::Continue {
            out.body.push_str(&format!(
                "{indent}for (const __t of __tr_{id}) {{ if (isPause(__t.result, true)) return pauseInfo({n} + \"#\" + __t.id, __t.result); }}\n",
                n = js_str(name)
            ));
        }
        let merge_suffix = merge_section(request.merge_hint, &sched);
        let merge_arg = format!(
            "__base_{id} + {suf} + up({needs}) + taskin(__tr_{id})",
            suf = js_str(&merge_suffix),
            needs = js_str_array(&spec.needs)
        );
        out.body.push_str(&format!(
            "{indent}results[{n}] = await {expr};\n",
            n = js_str(name),
            expr = dispatch_expr_raw(
                &merge_arg,
                &agent_opts_with_label(spec, &js_str(&format!("{name} · merge"))),
                retries
            ),
        ));
    }
    out.body.push_str(&format!(
        "{indent}withTokens(results[{n}], spent() - __b_{id});\n",
        n = js_str(name)
    ));
    out.body.push_str(&pause_check(spec, indent));
    out.schedules.push(sched.to_state(name, &path));
    Ok(())
}

/// Render a `parallel()` block for adjacent, independent stages.
fn render_parallel_group(
    ctx: &EmitCtx,
    group: &[StageSpec],
    out: &mut EmitOut,
) -> anyhow::Result<()> {
    out.body.push_str(&format!(
        "  log({});\n",
        js_str(&format!(
            "parallel: {}",
            group
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    ));
    out.body.push_str("  {\n");
    out.body.push_str("    const __pb = spent();\n");
    out.body.push_str(&format!(
        "    const __pl = pool({}, {});\n",
        group.len().max(1),
        ctx.pipe.settings.budget_floor_tokens
    ));
    out.body.push_str("    const __pg = await parallel([\n");
    for g in group {
        let prompt = stage_prompt(ctx, &g.name)?;
        out.body.push_str(&format!(
            "      () => __pl.run(() => {expr}),\n",
            expr = dispatch_expr(&prompt, &g.needs, &agent_opts(g), g.retries),
        ));
    }
    out.body.push_str("    ]);\n");
    out.body
        .push_str("    const __pt = Math.max(0, spent() - __pb);\n");
    out.body
        .push_str(&format!("    const __pn = {} || 1;\n", group.len()));
    for (idx, g) in group.iter().enumerate() {
        out.body.push_str(&format!(
            "    results[{n}] = __pg[{idx}];\n",
            n = js_str(&g.name)
        ));
    }
    for g in group {
        out.body.push_str(&format!(
            "    withTokens(results[{n}], Math.round(__pt / __pn));\n",
            n = js_str(&g.name)
        ));
    }
    let continue_on_failure = ctx.pipe.settings.on_item_failure == ItemFailurePolicy::Continue;
    for g in group {
        if continue_on_failure {
            out.body.push_str(&format!(
                "    if (isPause(results[{n}], {soft_arg})) log({n} + \": not completed (on_item_failure: continue, status=\" + (results[{n}] ? results[{n}].status : \"null\") + \")\");\n",
                n = js_str(&g.name),
                soft_arg = g.soft_pause
            ));
        } else {
            out.body.push_str(&pause_check(g, NESTED_INDENT));
        }
    }
    out.body.push_str("  }\n");
    Ok(())
}

/// Render the body steps of a correction loop. A body stage never schedules a plan, so whatever
/// the enclosing pass already recorded is set aside while it renders.
fn render_loop_body(ctx: &EmitCtx, body: &[Step], out: &mut EmitOut) -> anyhow::Result<()> {
    let outer_schedules = std::mem::take(&mut out.schedules);
    for step in body {
        render_step(ctx, step, NESTED_INDENT, out)?;
    }
    out.schedules = outer_schedules;
    Ok(())
}

/// Render a correction loop: re-run its body while the `when:` signal holds, then pause if the
/// loop did not converge.
fn render_loop(ctx: &EmitCtx, looped: &LoopRequest, out: &mut EmitOut) -> anyhow::Result<()> {
    let (container, max) = (looped.container, looped.max);
    let cond = match looped.source {
        Some((stage, prop)) => format!("sig({}, {})", js_str(stage), js_str(prop)),
        None => "true".to_string(),
    };
    let itvar = format!("__it_{}", ident(container));

    out.body.push_str(&format!(
        "  let {itvar} = 0;
"
    ));
    out.body.push_str(&format!(
        "  while ({itvar} < {max} && {cond}) {{
"
    ));
    out.body.push_str(&format!(
        "    {itvar}++;
"
    ));
    out.body.push_str(&format!(
        "    log({lit} + \" iteration \" + {itvar});
",
        lit = js_str(container)
    ));
    render_loop_body(ctx, looped.body, out)?;
    let refreshed = looped.body_producer.zip(looped.source);
    if let Some((producer, (source, _))) = refreshed {
        out.body.push_str(&format!(
            "    results[{s}] = results[{b}];
",
            s = js_str(source),
            b = js_str(producer)
        ));
    }
    out.body.push_str(
        "  }
",
    );
    if let Some((_, (source, _))) = refreshed {
        out.body.push_str(&format!(
            "  if ({cond}) {{
"
        ));
        out.body.push_str(&format!(
            "    log({lit} + \" did not converge after \" + {itvar} + \" iteration(s) — pausing on the remaining blocking findings\");
",
            lit = js_str(container)
        ));
        out.body.push_str(&format!(
            "    return pauseInfo({n}, results[{s}]);
",
            n = js_str(container),
            s = js_str(source)
        ));
        out.body.push_str(
            "  }
",
        );
    }
    Ok(())
}

/// The reply-schema constant a dispatch answers with, given whether its prompt names skills.
fn schema_const(kind: ReplySchemaKind, names_skills: bool) -> &'static str {
    let reviewer = matches!(
        kind,
        ReplySchemaKind::Reviewer | ReplySchemaKind::ReviewerWithSkills
    );
    match (reviewer, names_skills) {
        (true, true) => SCHEMA_REVIEWER_SKILLS,
        (true, false) => SCHEMA_REVIEWER,
        (false, true) => SCHEMA_STANDARD_SKILLS,
        (false, false) => SCHEMA_STANDARD,
    }
}

/// Build the `agent()` opts object literal (schema + label + phase + agentType,
/// model, and optional worktree isolation).
pub fn agent_opts(spec: &StageSpec) -> String {
    agent_opts_with_label(spec, &js_str(&spec.name))
}

/// Like `agent_opts` but with a caller-provided JS *expression* for `label` (fan-out shards compute
/// their label at runtime: `"code [2/3] <shard>"`).
fn agent_opts_with_label(spec: &StageSpec, label_expr: &str) -> String {
    agent_opts_with_skills(spec, label_expr, &spec.skills)
}

/// Like `agent_opts_with_label`, for a dispatch whose prompt names `skills` (a plan task may name
/// fewer than its stage).
fn agent_opts_with_skills(spec: &StageSpec, label_expr: &str, skills: &[String]) -> String {
    let mut opts = format!(
        "{{ label: {label_expr}, phase: {n}, schema: {sc}, agentType: {a}",
        n = js_str(&spec.name),
        sc = schema_const(spec.reply_schema, !skills.is_empty()),
        a = js_str(&spec.agent_type)
    );
    opts.push_str(&format!(", model: {}", js_str(spec.model.as_str())));
    if let Some(effort) = &spec.effort {
        opts.push_str(&format!(", effort: {}", js_str(effort.as_str())));
    }
    if spec.isolation == Some(Isolation::Worktree) {
        opts.push_str(", isolation: \"worktree\"");
    }
    if !skills.is_empty() {
        opts.push_str(&format!(", skills: {}", js_str_array(skills)));
        if spec.agent == AgentRole::Executor {
            opts.push_str(", skillsRetry: false");
        }
    }
    opts.push_str(" }");
    opts
}

/// Wrap the rendered stage bodies into the complete Workflow script.
fn assemble(ctx: &EmitCtx, plan: &ExecPlan, out: &EmitOut) -> String {
    let (pipe, state) = (ctx.pipe, ctx.state);
    let awaiting_plan = matches!(plan.end, Some(PassEnd::AwaitingPlan { .. }));
    let awaiting_composite = matches!(plan.end, Some(PassEnd::AwaitingComposite { .. }));
    let name = sanitize_name(&format!("aidp-v7-{}-{}", pipe.id, state.run_id));
    let desc = format!(
        "ai-dev-pipeline v7 — {} (run {}){}",
        pipe.id,
        state.run_id,
        if awaiting_plan {
            " [pass 1: tail compiled from the plan]"
        } else if awaiting_composite {
            " [pass stops before a composite — re-emit after crossing it]"
        } else {
            ""
        }
    );
    let phases_js = out
        .phases
        .iter()
        .map(|p| format!("{{ title: {} }}", js_str(p)))
        .collect::<Vec<_>>()
        .join(", ");

    let schema = reply_schema_json();
    let schema_reviewer = reply_schema_reviewer_json();

    let mut s = String::new();
    s.push_str(&format!(
        "export const meta = {{\n  name: {name},\n  description: {desc},\n  phases: [{phases}],\n}};\n\n",
        name = js_str(&name),
        desc = js_str(&desc),
        phases = phases_js,
    ));
    s.push_str(&format!("const REPLY = {schema};\n"));
    s.push_str(&format!("const REPLY_REVIEWER = {schema_reviewer};\n"));
    if any_stage(&pipe.stages, &|st| !st.skills.is_empty()) {
        s.push_str(&format!(
            "const REPLY_SKILLS = {};\n",
            with_skills_loaded(&reply_schema_json())
        ));
        s.push_str(&format!(
            "const REPLY_REVIEWER_SKILLS = {};\n",
            with_skills_loaded(&schema_reviewer)
        ));
    }
    s.push('\n');
    s.push_str(&format!(
        "const __approvedBaked = {};\n",
        js_str_array(&crate::run::approved_gates(state))
    ));
    s.push_str("const __approvedArgs = (typeof args === 'object' && args && Array.isArray(args.approved_gates)) ? args.approved_gates : [];\n");
    s.push_str(
        "const __approved = Array.from(new Set([...__approvedBaked, ...__approvedArgs]));\n",
    );
    s.push_str("const results = {};\nconst __guardFindings = [];\n");
    let reviewed: Vec<String> = plan.reviewed_stages.iter().cloned().collect();
    s.push_str(&format!(
        "const __REVIEWED = new Set({});\n\n",
        js_str_array(&reviewed)
    ));
    s.push_str(HELPERS);
    s.push_str(&guard::emit_js());
    if pipe.settings.max_stage_retries > 0 {
        s.push_str(RETRY_HELPERS);
    }
    let has_fan_out = pipeline::flat_stages(pipe)
        .iter()
        .any(|st| st.fan_out.is_some());
    if has_fan_out {
        s.push_str(FANOUT_HELPERS);
    }
    let has_plan_tasks = pipe.stages.iter().any(|st| st.plan_tasks.is_some());
    if has_plan_tasks {
        s.push_str(PLAN_HELPERS);
    }
    if has_fan_out || has_plan_tasks || out.has_parallel_group {
        s.push_str(POOL_HELPERS);
    }
    s.push('\n');
    s.push_str(&out.body);
    s.push_str(
        "\n  return { status: \"done\", completed: Object.keys(results), results: results, guard_findings: __guardFindings, self_answered_questions: selfAnswered() };\n",
    );
    s
}

/// True when `pred` holds for a stage at any depth of loop bodies.
fn any_stage(stages: &[Stage], pred: &dyn Fn(&Stage) -> bool) -> bool {
    stages
        .iter()
        .any(|st| pred(st) || any_stage(&st.body, pred))
}

/// JSON-encode a string into a valid JS string literal (double-quoted, escaped).
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// A JS array literal of strings, e.g. `["a","b"]`.
fn js_str_array(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| js_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn ident(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/workflow_emit_golden_tests.rs"]
mod golden_tests;
#[cfg(test)]
#[path = "../tests/unit/workflow_emit_tests.rs"]
mod tests;
