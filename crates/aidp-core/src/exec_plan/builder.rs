use super::refs::{
    is_reviewer, parse_fanout_from, parse_plan_from, plan_artifact_path, strip_handlebars,
};
use crate::compose;
use crate::expr;
use crate::models::build_ctx::BuildCtx;
use crate::models::build_out::BuildOut;
use crate::models::constants::pass::DEFAULT_MAX_STAGES_PER_PASS;
use crate::models::constants::refs::{
    LOOP_REF_MIN_PARTS, REF_OUTPUTS, REF_SEPARATOR, REF_STAGES, RUNTIME_REF_MARKER,
};
use crate::models::constants::review::AXIS_INPUT;
use crate::models::constants::scope::MODE_STAGE_DIR_ONLY;
use crate::models::enums::contract_flag::ContractFlag;
use crate::models::enums::fanout_items::FanoutItems;
use crate::models::enums::flow::Flow;
use crate::models::enums::loop_producer::LoopProducer;
use crate::models::enums::pass_end::PassEnd;
use crate::models::enums::reply_schema_kind::ReplySchemaKind;
use crate::models::enums::skip_reason::SkipReason;
use crate::models::enums::step::Step;
use crate::models::enums::{AgentRole, HarnessGapPolicy, LoopExhaustedPolicy, ReviewAxis};
use crate::models::exec_plan::ExecPlan;
use crate::models::gate_spec::GateSpec;
use crate::models::loop_signal::LoopSignal;
use crate::models::pipeline::{FanOutSpec, PlanTasksSpec};
use crate::models::port::PlanSource;
use crate::models::stage_spec::StageSpec;
use crate::pipeline::{self, Pipeline, Stage};
use crate::plan;
use crate::run::{self, RunState};
use crate::toolset;
use std::collections::{BTreeMap, BTreeSet};

/// Compiles a pipeline and run state into the typed plan of one Workflow pass. Reads plan
/// artifacts through `plans` and touches nothing else.
pub fn compile_exec_plan(
    pipe: &Pipeline,
    state: &RunState,
    plans: &dyn PlanSource,
) -> anyhow::Result<ExecPlan> {
    let ctx = BuildCtx::new(pipe, state, plans)?;
    let mut out = BuildOut::default();
    let mut index = 0usize;
    while index < pipe.stages.len() {
        match route_stage(&ctx, index, &mut out)? {
            Flow::Next(next) => index = next,
            Flow::Stop => break,
        }
    }
    Ok(ExecPlan {
        run_id: state.run_id.clone(),
        template: pipe.id.clone(),
        steps: out.steps,
        warnings: out.warnings,
        gates: out.gates,
        end: out.end,
        reviewed_stages: ctx.loop_reviewed,
    })
}

impl<'a> BuildCtx<'a> {
    fn new(
        pipe: &'a Pipeline,
        state: &'a RunState,
        plans: &'a dyn PlanSource,
    ) -> anyhow::Result<Self> {
        let loop_linked = loop_linked_stages(pipe)?;
        let mut soft = loop_linked.clone();
        for stage in pipeline::flat_stages(pipe) {
            if is_reviewer(stage) {
                soft.insert(stage.name.clone());
            }
        }
        let loop_reviewed = loop_reviewed_stages(pipe)?;
        soft.extend(loop_reviewed.iter().cloned());
        Ok(Self {
            pipe,
            state,
            plans,
            soft,
            loop_linked,
            loop_reviewed,
            fanout_producers: fanout_producers(pipe)?,
            plan_producers: plan_producers(pipe)?,
            approved: run::approved_gates(state),
            budget: pipe
                .settings
                .max_stages_per_pass
                .unwrap_or(DEFAULT_MAX_STAGES_PER_PASS),
        })
    }
}

/// The stages whose prompt carries a fan-out contract for a downstream consumer.
fn fanout_producers(pipe: &Pipeline) -> anyhow::Result<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    for stage in pipeline::flat_stages(pipe) {
        let Some(from) = stage.fan_out.as_ref().and_then(|f| f.from.as_ref()) else {
            continue;
        };
        set.insert(parse_fanout_from(&stage.name, from)?.0);
    }
    Ok(set)
}

/// The stages whose prompt carries the plan contract for a downstream `plan_tasks:` stage.
fn plan_producers(pipe: &Pipeline) -> anyhow::Result<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    for stage in &pipe.stages {
        if let Some(spec) = &stage.plan_tasks {
            set.insert(parse_plan_from(&stage.name, &spec.from)?.0);
        }
    }
    Ok(set)
}

/// How many of `stages[start..]` belong to the composite named `composite`.
fn composite_span_len(stages: &[Stage], start: usize, composite: &str) -> usize {
    let prefix = format!("{composite}{}", compose::SEP);
    stages[start..]
        .iter()
        .take_while(|s| s.name.starts_with(&prefix))
        .count()
}

/// Whether the pass must stop before entering `composite`, given the stages already compiled.
fn crosses_pass_budget(ctx: &BuildCtx, at: usize, composite: &str, emitted: usize) -> bool {
    let crossed = ctx.state.crossed_boundaries.iter().any(|c| c == composite);
    !crossed && emitted + composite_span_len(&ctx.pipe.stages, at, composite) > ctx.budget
}

/// Compiles the stage at `index` and tells the caller where to resume.
fn route_stage(ctx: &BuildCtx, index: usize, out: &mut BuildOut) -> anyhow::Result<Flow> {
    let st = &ctx.pipe.stages[index];
    let next = Flow::Next(index + 1);

    if st.is_loop() {
        if composite_skip(st, ctx, &mut out.warnings) {
            push_skipped(out, st, SkipReason::CompositeWhen);
            return Ok(next);
        }
        compile_loop(ctx, st, out)?;
        return Ok(next);
    }
    if st.is_composite() {
        return Ok(next);
    }
    if let Some(reason) = static_skip(st, ctx, &mut out.warnings) {
        push_skipped(out, st, reason);
        return Ok(next);
    }
    if let Some(composite) = &st.composite_entry {
        if crosses_pass_budget(ctx, index, composite, out.emitted) {
            out.steps.push(Step::CompositeBoundary {
                stage: st.name.clone(),
                composite: composite.clone(),
            });
            out.end = Some(PassEnd::AwaitingComposite {
                stage: st.name.clone(),
                composite: composite.clone(),
            });
            return Ok(Flow::Stop);
        }
    }
    if is_gate(st, ctx, &mut out.warnings) {
        out.gates.push(st.name.clone());
        return push_dispatched(ctx, st, true, next, out);
    }
    if st.plan_tasks.is_some() || st.fan_out.is_some() {
        return push_dispatched(ctx, st, false, next, out);
    }

    let group = independent_group(ctx, index, out);
    if group.len() == 1 {
        out.steps.push(Step::Stage(stage_spec(ctx, st, false)));
        out.emitted += 1;
        return Ok(next);
    }
    let specs = group.iter().map(|g| stage_spec(ctx, g, false)).collect();
    out.steps.push(Step::Parallel { stages: specs });
    out.emitted += group.len();
    Ok(Flow::Next(index + group.len()))
}

fn push_skipped(out: &mut BuildOut, st: &Stage, reason: SkipReason) {
    out.steps.push(Step::Skipped {
        stage: st.name.clone(),
        reason,
    });
}

/// Compiles a plan, fan-out or plain stage, stopping the pass when its plan is missing.
fn push_dispatched(
    ctx: &BuildCtx,
    st: &Stage,
    gated: bool,
    next: Flow,
    out: &mut BuildOut,
) -> anyhow::Result<Flow> {
    let (step, awaiting) = stage_step(ctx, st, gated)?;
    out.steps.push(step);
    out.emitted += 1;
    match awaiting {
        Some(end) => {
            out.end = Some(end);
            Ok(Flow::Stop)
        }
        None => Ok(next),
    }
}

/// The plain stages from `start` onward that can share one parallel block.
fn independent_group<'a>(ctx: &BuildCtx<'a>, start: usize, out: &mut BuildOut) -> Vec<&'a Stage> {
    let stages = &ctx.pipe.stages;
    let mut group: Vec<&Stage> = vec![&stages[start]];
    for (offset, nxt) in stages[start + 1..].iter().enumerate() {
        let at = start + 1 + offset;
        if nxt.is_loop() || nxt.is_composite() {
            break;
        }
        if nxt.fan_out.is_some() || nxt.plan_tasks.is_some() {
            break;
        }
        if let Some(composite) = &nxt.composite_entry {
            if crosses_pass_budget(ctx, at, composite, out.emitted + group.len()) {
                break;
            }
        }
        if static_skip(nxt, ctx, &mut out.warnings).is_some() {
            break;
        }
        if is_gate(nxt, ctx, &mut out.warnings) {
            break;
        }
        let group_names: Vec<&str> = group.iter().map(|g| g.name.as_str()).collect();
        let nxt_needs_group = nxt.needs.iter().any(|d| group_names.contains(&d.as_str()));
        let group_needs_nxt = group.iter().any(|g| g.needs.iter().any(|d| d == &nxt.name));
        if nxt_needs_group || group_needs_nxt {
            break;
        }
        group.push(nxt);
    }
    group
}

/// Routes a stage to its step: plan-driven, fan-out or plain. The second value is set when a
/// plan-driven stage has no plan yet.
fn stage_step(ctx: &BuildCtx, st: &Stage, gated: bool) -> anyhow::Result<(Step, Option<PassEnd>)> {
    if let Some(spec) = &st.plan_tasks {
        return plan_step(ctx, st, spec, gated);
    }
    if let Some(spec) = &st.fan_out {
        return Ok((fanout_step(ctx, st, spec, gated)?, None));
    }
    Ok((Step::Stage(stage_spec(ctx, st, gated)), None))
}

fn fanout_step(ctx: &BuildCtx, st: &Stage, spec: &FanOutSpec, gated: bool) -> anyhow::Result<Step> {
    let items = match &spec.from {
        Some(from) => {
            parse_fanout_from(&st.name, from)?;
            FanoutItems::FromSignal(strip_handlebars(from).to_string())
        }
        None => FanoutItems::Static(spec.items.clone()),
    };
    Ok(Step::Fanout {
        stage: stage_spec(ctx, st, gated),
        items,
        max: spec.max,
        fold: spec.fold,
        merge_hint: spec.merge_hint.clone(),
    })
}

fn plan_step(
    ctx: &BuildCtx,
    st: &Stage,
    spec: &PlanTasksSpec,
    gated: bool,
) -> anyhow::Result<(Step, Option<PassEnd>)> {
    let (producer, output) = parse_plan_from(&st.name, &spec.from)?;
    let path = plan_artifact_path(ctx.state, ctx.pipe, &producer, &output);
    let loaded = ctx
        .plans
        .load(&path)
        .map_err(|e| anyhow::anyhow!("stage `{}`: {e}", st.name))?;
    let awaiting = match loaded {
        Some(doc) => {
            plan::schedule(&doc, spec.max, &spec.kinds)
                .map_err(|e| anyhow::anyhow!("stage `{}`: {e}", st.name))?;
            None
        }
        None => Some(PassEnd::AwaitingPlan {
            stage: st.name.clone(),
            producer,
            plan_path: path.display().to_string(),
        }),
    };
    let step = Step::PlanTasks {
        stage: stage_spec(ctx, st, gated),
        from: strip_handlebars(&spec.from).to_string(),
        plan_path: path.display().to_string(),
        max: spec.max,
        kinds: spec.kinds.clone(),
        merge_hint: spec.merge_hint.clone(),
        skills_by_kind: spec.skills_by_kind.clone(),
    };
    Ok((step, awaiting))
}

/// Compiles a correction loop and its body. A body stage never schedules a plan.
fn compile_loop(ctx: &BuildCtx, container: &Stage, out: &mut BuildOut) -> anyhow::Result<()> {
    if container.body.is_empty() {
        return Ok(());
    }
    let max = expr::eval_max_iterations(container, ctx.state, ctx.pipe);
    let signal = loop_signal(container)?;
    let mut body = Vec::new();
    for stage in &container.body {
        body.push(stage_step(ctx, stage, false)?.0);
        out.emitted += 1;
    }
    let (source_stage, body_producer) = match signal.producer {
        LoopProducer::None => (None, None),
        LoopProducer::Source(stage_ref) => (Some(stage_ref), None),
        LoopProducer::Found {
            stage_ref,
            producer,
        } => (Some(stage_ref), Some(producer)),
    };
    out.steps.push(Step::Loop {
        container: container.name.clone(),
        condition: signal.condition,
        source_stage,
        signal: signal.property,
        body_producer,
        max,
        continue_when_exhausted: container.on_exhausted == Some(LoopExhaustedPolicy::Continue),
        body,
    });
    Ok(())
}

/// Parses a loop's `when:`, recognising `stages.<S>.outputs.<NAME>.<PROP>`.
fn loop_signal(container: &Stage) -> anyhow::Result<LoopSignal> {
    let raw = match &container.when {
        None => {
            return Ok(LoopSignal {
                condition: None,
                property: None,
                producer: LoopProducer::None,
            })
        }
        Some(serde_yaml::Value::String(s)) => s,
        Some(other) => anyhow::bail!(
            "loop `{}`: `when:` must be a string of shape `stages.<stage>.outputs.<name>.<prop>`, got a non-string value ({:?})",
            container.name,
            other
        ),
    };
    let inner = strip_handlebars(raw);
    let parts: Vec<&str> = inner.split(REF_SEPARATOR).map(str::trim).collect();
    if parts.len() >= LOOP_REF_MIN_PARTS && parts[0] == REF_STAGES && parts[2] == REF_OUTPUTS {
        let stage_ref = parts[1].to_string();
        let output_name = parts[3];
        let producer = container
            .body
            .iter()
            .rev()
            .find(|b| b.output_names().iter().any(|o| o == output_name))
            .map(|b| b.name.clone());
        let producer = match producer {
            Some(producer) => LoopProducer::Found {
                stage_ref,
                producer,
            },
            None => LoopProducer::Source(stage_ref),
        };
        return Ok(LoopSignal {
            condition: Some(inner.to_string()),
            property: Some(parts[4].to_string()),
            producer,
        });
    }
    anyhow::bail!(
        "loop `{}`: unrecognized `when:` shape `{}` — it must reference a reviewer signal of the form `stages.<stage>.outputs.<name>.<prop>` (handlebars-wrapped). The emitter will not silently disable the loop; fix the template `when:`.",
        container.name,
        raw.trim()
    )
}

/// The stages a loop's `when:` reads, plus the body stage that re-produces that output.
fn loop_linked_stages(pipe: &Pipeline) -> anyhow::Result<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    for st in pipeline::flat_stages(pipe) {
        if !st.is_loop() || st.body.is_empty() {
            continue;
        }
        match loop_signal(st)?.producer {
            LoopProducer::None => {}
            LoopProducer::Source(stage_ref) => {
                set.insert(stage_ref);
            }
            LoopProducer::Found {
                stage_ref,
                producer,
            } => {
                set.insert(stage_ref);
                set.insert(producer);
            }
        }
    }
    Ok(set)
}

/// The non-reviewer stages whose leftover problems a correction loop exists to fix.
fn loop_reviewed_stages(pipe: &Pipeline) -> anyhow::Result<BTreeSet<String>> {
    let flat = pipeline::flat_stages(pipe);
    let by_name: BTreeMap<&str, &Stage> = flat.iter().map(|s| (s.name.as_str(), *s)).collect();
    let mut set = BTreeSet::new();
    for st in &flat {
        if !st.is_loop() || st.body.is_empty() {
            continue;
        }
        let LoopProducer::Found { stage_ref, .. } = loop_signal(st)?.producer else {
            continue;
        };
        for b in st.body.iter().filter(|b| !is_reviewer(b)) {
            set.insert(b.name.clone());
        }
        let mut stack = vec![stage_ref];
        let mut seen = BTreeSet::new();
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(stage) = by_name.get(name.as_str()) else {
                continue;
            };
            if !is_reviewer(stage) {
                continue;
            }
            for dep in &stage.needs {
                match by_name.get(dep.as_str()) {
                    Some(ds) if is_reviewer(ds) => stack.push(dep.clone()),
                    Some(ds) if !ds.is_loop() && !ds.is_composite() => {
                        set.insert(dep.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(set)
}

/// A stage's dispatch data with every default resolved.
fn stage_spec(ctx: &BuildCtx, st: &Stage, gated: bool) -> StageSpec {
    let agent = st.agent_role();
    let agent_name = st.agent.as_deref().unwrap_or(AgentRole::Executor.as_str());
    let scope_mode = st
        .scope
        .as_ref()
        .and_then(|s| s.mode.as_deref())
        .unwrap_or(MODE_STAGE_DIR_ONLY);
    let agent_type =
        toolset::agent_type_for(agent_name, scope_mode, ctx.pipe.settings.tool_enforcement);
    let model = ctx
        .state
        .model_overrides
        .get(&st.name)
        .copied()
        .or(st.model)
        .unwrap_or_else(|| agent.default_model());
    let effort = st.effort.or_else(|| {
        ctx.pipe
            .settings
            .emit_effort
            .then(|| agent.default_effort())
    });
    let reply_schema = match (agent == AgentRole::Reviewer, st.skills.is_empty()) {
        (true, true) => ReplySchemaKind::Reviewer,
        (true, false) => ReplySchemaKind::ReviewerWithSkills,
        (false, true) => ReplySchemaKind::Standard,
        (false, false) => ReplySchemaKind::StandardWithSkills,
    };
    StageSpec {
        name: st.name.clone(),
        agent_type,
        model,
        effort,
        skills: st.skills.clone(),
        needs: st.needs.clone(),
        outputs: st.output_names(),
        scope: st.scope.clone(),
        isolation: st.isolation,
        gate: gated.then(|| GateSpec {
            approved: ctx.approved.contains(&st.name),
        }),
        when: runtime_when(st),
        retries: ctx.pipe.settings.max_stage_retries,
        soft_pause: ctx.soft.contains(&st.name),
        loop_linked: ctx.loop_linked.contains(&st.name),
        reply_schema,
        contracts: contract_flags(ctx, st),
        agent,
    }
}

/// The raw `when:` text when it reads a stage output and so cannot be decided at compile time.
fn runtime_when(st: &Stage) -> Option<String> {
    match &st.when {
        Some(serde_yaml::Value::String(text)) if text.contains(RUNTIME_REF_MARKER) => {
            Some(text.clone())
        }
        _ => None,
    }
}

/// Names the prompt contracts and runtime guards that apply to a stage.
fn contract_flags(ctx: &BuildCtx, st: &Stage) -> Vec<ContractFlag> {
    let mut flags = Vec::new();
    if ctx.fanout_producers.contains(&st.name) {
        flags.push(ContractFlag::FanoutProducer);
    }
    if ctx.plan_producers.contains(&st.name) {
        flags.push(ContractFlag::PlanProducer);
    }
    if st.merge_findings.is_some() {
        flags.push(ContractFlag::MergeFindings);
    }
    let axis = st
        .inputs
        .get(AXIS_INPUT)
        .and_then(|v| v.as_str())
        .and_then(ReviewAxis::parse);
    match axis {
        Some(ReviewAxis::Synthesis) => flags.push(ContractFlag::ReviewSynthesis),
        Some(dimension) if dimension.is_dimension() => flags.push(ContractFlag::ReviewDimension),
        _ => {}
    }
    if st.on_harness_gap == Some(HarnessGapPolicy::Pause) && !ctx.loop_linked.contains(&st.name) {
        flags.push(ContractFlag::HarnessGapPause);
    }
    if let Some(layer) = st.knowledge.as_ref().and_then(|k| k.layer()) {
        flags.push(ContractFlag::Knowledge(layer));
    }
    flags
}

/// Why a stage is statically skipped, `None` when it runs. Both conditions are always evaluated so
/// each can report an unparseable expression.
fn static_skip(st: &Stage, ctx: &BuildCtx, warnings: &mut Vec<String>) -> Option<SkipReason> {
    let composite = composite_skip(st, ctx, warnings);
    let own = own_static_skip(st, ctx, warnings);
    if composite {
        Some(SkipReason::CompositeWhen)
    } else if own {
        Some(SkipReason::StaticWhen)
    } else {
        None
    }
}

/// An enclosing composite's `when:` that is statically false. Runtime-dependent ones fail open.
fn composite_skip(st: &Stage, ctx: &BuildCtx, warnings: &mut Vec<String>) -> bool {
    let eval = expr::EvalCtx::new(ctx.state, ctx.pipe);
    st.composite_when.iter().any(|w| {
        if refs_runtime(w) {
            return false;
        }
        match expr::eval_condition(w, &eval) {
            Some(value) => !value,
            None => {
                warnings.push(format!(
                    "warning: stage `{}` has an unparseable composite `when:` expression ({w:?}); defaulting to run",
                    st.name
                ));
                false
            }
        }
    })
}

fn own_static_skip(st: &Stage, ctx: &BuildCtx, warnings: &mut Vec<String>) -> bool {
    match &st.when {
        Some(w) if !refs_runtime(w) => !expr::eval_when_collect(st, ctx.state, ctx.pipe, warnings),
        _ => false,
    }
}

/// Whether the stage is a gate. Runtime-dependent expressions fail safe and count as a gate.
fn is_gate(st: &Stage, ctx: &BuildCtx, warnings: &mut Vec<String>) -> bool {
    match &st.gate {
        None | Some(serde_yaml::Value::Null) => false,
        Some(serde_yaml::Value::Bool(b)) => *b,
        Some(other) => {
            refs_runtime(other) || expr::eval_gate_collect(st, ctx.state, ctx.pipe, warnings)
        }
    }
}

/// True when a `when:`/`gate:` value references runtime stage outputs.
fn refs_runtime(v: &serde_yaml::Value) -> bool {
    matches!(v, serde_yaml::Value::String(s) if s.contains(RUNTIME_REF_MARKER))
}
