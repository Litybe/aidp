use crate::error::{FlowError, RejectReason};
use crate::expr::{self, Context};
use crate::model::{
    FlowDef, FlowState, FlowStatus, Frame, FrameKind, OnError, Step, StepKind, StepState,
    StepStatus, CI_BLOCK, CI_STEP_PREFIX, PUBLISH_BLOCK, PUBLISH_STEP_PREFIX,
};
use crate::models::constants::entry::FORCED_DETAIL_PREFIX;
use crate::models::constants::human::{REJECTED_DETAIL, REJECTED_SKIP_REFUSAL};
use crate::models::constants::manual::{PUBLISH_ACTION, PUBLISH_SUFFIX};
use crate::models::constants::resource;
pub use crate::models::enums::{
    ActionOutcome, EffectKind, EffectName, Event, FlowEvent, HumanDecision, ManualRequest,
    PipelineRunStatus, StepReason,
};
pub use crate::models::Effect;
use crate::template::render_with;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Prefix of the label given to effects requested by hand.
pub const MANUAL_PREFIX: &str = "manual-";
/// Run-wide label of journal and emit effect ids.
pub const FLOW_LABEL: &str = "flow";

const OUTPUT_DECISION: &str = "decision";
const OUTPUT_ACTOR: &str = "actor";
const OUTPUT_AT: &str = "at";
/// Output of a human step holding the path of its stored report.
pub const OUTPUT_REPORT: &str = "report";
const OUTPUT_OK: &str = "ok";
const OUTPUT_MARKED_PASSED: &str = "marked_passed";

const CTX_STEPS: &str = "steps";
const CTX_SETTINGS: &str = "settings";
const CTX_INPUTS: &str = "inputs";
const CTX_TICKET: &str = "ticket";
const CTX_STATUS: &str = "status";
const CTX_OUTPUTS: &str = "outputs";
const CTX_ITERATION: &str = "iteration";
const TICKET_SOURCE: &str = "source";

/// Builds the deterministic id of a step effect.
pub fn effect_id(step: &str, iteration: u32, kind: &str) -> String {
    format!("{step}:{iteration}:{kind}")
}

/// Applies one event; pure. A refusal returns `FlowError::Rejected`
/// and leaves the state unchanged.
pub fn advance(
    def: &FlowDef,
    state: FlowState,
    event: Event,
) -> Result<(FlowState, Vec<Effect>), FlowError> {
    let mut run = Run {
        def,
        at: event.at().to_string(),
        state,
        effects: Vec::new(),
    };
    run.apply(event)?;
    run.refresh_status();
    Ok((run.state, run.effects))
}

struct Run<'a> {
    def: &'a FlowDef,
    state: FlowState,
    effects: Vec<Effect>,
    at: String,
}

fn defines(steps: &[Step], id: &str) -> bool {
    steps
        .iter()
        .any(|step| step.id == id || defines(&step.body, id))
}

fn rejected(reason: RejectReason, detail: impl Into<String>) -> FlowError {
    FlowError::rejected(reason, detail)
}

impl Run<'_> {
    fn apply(&mut self, event: Event) -> Result<(), FlowError> {
        self.guard(&event)?;
        match event {
            Event::Start { ticket, entry, .. } => self.start(ticket, entry),
            Event::ActionFinished {
                effect_id, outcome, ..
            } => self.action_finished(&effect_id, outcome),
            Event::PipelineFinished {
                effect_id,
                run_status,
                outputs,
                ..
            } => self.pipeline_finished(&effect_id, &run_status, outputs),
            Event::PipelineOrphaned { effect_id, .. } => self.pipeline_orphaned(&effect_id),
            Event::BudgetInsufficient { step, .. } => self.budget_insufficient(&step),
            Event::ResourceBusy { step, resource, .. } => self.resource_busy(&step, &resource),
            Event::TicketBudgetExceeded { step, detail, .. } => {
                self.ticket_budget_exceeded(&step, &detail)
            }
            Event::HumanDecision {
                step,
                decision,
                actor,
                report,
                ..
            } => self.human_decision(&step, &decision, &actor, report),
            Event::WaitSatisfied {
                effect_id, outputs, ..
            } => self.wait_satisfied(&effect_id, outputs),
            Event::Pause { .. } => self.pause(),
            Event::Resume { .. } => self.resume(),
            Event::Abort { .. } => self.abort(),
            Event::Retry { step, .. } => self.retry(&step),
            Event::Skip { step, .. } => self.skip(&step),
            Event::MarkPassed { step, .. } => self.mark_passed(&step),
            Event::Manual { request, .. } => self.manual(request),
        }
    }

    fn guard(&self, event: &Event) -> Result<(), FlowError> {
        let status = self.state.status;
        let closed = matches!(status, FlowStatus::Done | FlowStatus::Aborted);
        match event {
            Event::Start { .. } => {
                if self.state.is_fresh() {
                    Ok(())
                } else {
                    Err(rejected(
                        RejectReason::AlreadyStarted,
                        "run already started",
                    ))
                }
            }
            _ if self.state.is_fresh() => {
                Err(rejected(RejectReason::NotStarted, "run not started"))
            }
            Event::Manual { .. } => Ok(()),
            Event::PipelineFinished { effect_id, .. } | Event::ActionFinished { effect_id, .. }
                if effect_id.starts_with(MANUAL_PREFIX) =>
            {
                Ok(())
            }
            Event::Retry { .. }
            | Event::Skip { .. }
            | Event::MarkPassed { .. }
            | Event::Abort { .. }
                if !closed =>
            {
                Ok(())
            }
            _ if !status.is_terminal() => Ok(()),
            _ => Err(rejected(
                RejectReason::NotActive,
                format!("run is {}", status.as_wire()),
            )),
        }
    }

    fn start(&mut self, ticket: Value, entry: Option<String>) -> Result<(), FlowError> {
        if let Some(name) = entry {
            self.seed_entry(&name)?;
        }
        self.state.ticket = ticket;
        self.state.status = FlowStatus::Running;
        self.state.started_at = Some(self.at.clone());
        self.state.stack.push(Frame {
            kind: FrameKind::Root,
            prefix: String::new(),
            owner: None,
            steps: self.def.steps.clone(),
            index: 0,
        });
        self.journal("flow started".to_string());
        self.emit(FlowEvent::Started, None, None);
        self.drive();
        Ok(())
    }

    /// Records the entry the run starts from and stores the outputs it assumes for skipped steps.
    fn seed_entry(&mut self, name: &str) -> Result<(), FlowError> {
        let Some(entry) = self.def.entries.get(name) else {
            return Err(rejected(
                RejectReason::UnknownEntry,
                format!("flow `{}` has no entry `{name}`", self.def.id),
            ));
        };
        for (rid, outputs) in &entry.outputs {
            self.state.steps.insert(
                rid.clone(),
                StepState {
                    status: StepStatus::Skipped,
                    reason: Some(StepReason::Forced.as_str().to_string()),
                    detail: Some(format!("{FORCED_DETAIL_PREFIX}{name}")),
                    outputs: outputs.clone(),
                    finished_at: Some(self.at.clone()),
                    ..StepState::default()
                },
            );
        }
        self.state.entry = Some(name.to_string());
        Ok(())
    }

    /// The detail of a top-level step skipped because the run started from an entry past it.
    fn forced_skip(&self, step: &Step, prefix: &str) -> Option<String> {
        let name = self.state.entry.as_deref()?;
        let entry = self.def.entries.get(name)?;
        let at_root = prefix.is_empty() && self.state.stack.len() == 1;
        let reached = step.id == entry.step || self.state.steps.contains_key(&entry.step);
        if !at_root || reached || entry.keep.contains(&step.id) {
            return None;
        }
        Some(format!("{FORCED_DETAIL_PREFIX}{name}"))
    }

    fn push_effect(&mut self, id: String, kind: EffectKind) {
        self.effects.push(Effect { id, kind });
    }

    fn next_sequence_id(&mut self, kind_name: &str) -> String {
        self.state.seq += 1;
        format!("{FLOW_LABEL}:{}:{kind_name}", self.state.seq)
    }

    fn journal(&mut self, line: String) {
        let id = self.next_sequence_id(EffectName::Journal.as_str());
        let at = self.at.clone();
        self.push_effect(id, EffectKind::Journal { at, line });
    }

    fn emit(&mut self, event: FlowEvent, step: Option<&str>, detail: Option<String>) {
        let id = self.next_sequence_id(EffectName::Emit.as_str());
        let step = step.map(str::to_string);
        self.push_effect(
            id,
            EffectKind::Emit {
                event,
                step,
                detail,
            },
        );
    }

    fn step_effect(&mut self, rid: &str, kind_name: &str, kind: EffectKind) {
        let iteration = self.state.steps.get(rid).map_or(0, |step| step.iteration);
        self.push_effect(effect_id(rid, iteration, kind_name), kind);
    }

    fn context(&self, prefix: &str) -> Context {
        let mut steps = Map::new();
        for (rid, step_state) in &self.state.steps {
            let entry = json!({
                CTX_STATUS: step_state.status.as_wire(),
                CTX_OUTPUTS: step_state.outputs,
                CTX_ITERATION: step_state.iteration,
            });
            if let Some(short) = rid.strip_prefix(prefix).filter(|_| !prefix.is_empty()) {
                steps.insert(short.to_string(), entry.clone());
            }
            steps.insert(rid.clone(), entry);
        }
        Context::new(json!({
            CTX_STEPS: steps,
            CTX_SETTINGS: self.state.settings,
            CTX_INPUTS: self.state.inputs,
            CTX_TICKET: self.state.ticket,
        }))
    }

    fn rendered_with(&self, step: &Step) -> BTreeMap<String, Value> {
        render_with(&step.with, &self.context(&self.top_prefix()))
    }

    fn top_step(&self) -> Option<Step> {
        let frame = self.state.stack.last()?;
        frame.steps.get(frame.index).cloned()
    }

    fn top_prefix(&self) -> String {
        self.state
            .stack
            .last()
            .map(|frame| frame.prefix.clone())
            .unwrap_or_default()
    }

    fn leave(&mut self) {
        self.state.current = None;
        if let Some(frame) = self.state.stack.last_mut() {
            frame.index += 1;
        }
    }

    fn drive(&mut self) {
        loop {
            let blocked = self.state.status.is_terminal()
                || self.state.pause_requested
                || self.state.current.is_some();
            if blocked {
                return;
            }
            let Some(frame) = self.state.stack.last() else {
                self.finish_flow(FlowStatus::Done);
                return;
            };
            let Some(step) = frame.steps.get(frame.index).cloned() else {
                self.close_frame();
                continue;
            };
            let prefix = frame.prefix.clone();
            self.enter_step(step, prefix);
        }
    }

    fn close_frame(&mut self) {
        let Some(frame) = self.state.stack.last() else {
            return;
        };
        let (kind, owner) = (frame.kind, frame.owner.clone());
        match kind {
            FrameKind::Root => {
                self.state.stack.pop();
            }
            FrameKind::Block => {
                self.state.stack.pop();
                self.state.current = owner;
                self.finish_ok(BTreeMap::new());
            }
            FrameKind::LoopBody => self.end_loop_iteration(owner),
        }
    }

    fn enter_step(&mut self, step: Step, prefix: String) {
        let rid = format!("{prefix}{}", step.id);
        if let Some(detail) = self.forced_skip(&step, &prefix) {
            let seeded = self.state.steps.remove(&rid).map(|state| state.outputs);
            self.state.steps.insert(
                rid.clone(),
                StepState {
                    outputs: seeded.unwrap_or_default(),
                    phase: step.phase.clone(),
                    ..StepState::default()
                },
            );
            self.mark_skipped(&rid, StepReason::Forced, detail);
            self.leave();
            return;
        }
        let iteration = self
            .state
            .steps
            .get(&rid)
            .map_or(0, |existing| existing.iteration + 1);
        self.state.steps.insert(
            rid.clone(),
            StepState {
                iteration,
                phase: step.phase.clone(),
                ..StepState::default()
            },
        );
        match self.skip_decision(&step, &prefix) {
            Ok(Some((reason, detail))) => {
                self.mark_skipped(&rid, reason, detail);
                self.leave();
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.state.current = Some(rid);
                self.fail_current(
                    StepStatus::Failed,
                    StepReason::ExpressionError,
                    error.to_string(),
                );
                return;
            }
        }
        match step.kind {
            StepKind::Action => self.enter_action(&step, rid),
            StepKind::Pipeline => self.enter_pipeline(&step, rid),
            StepKind::Wait => self.enter_wait(&step, rid),
            StepKind::Human => self.enter_human(&step, rid),
            StepKind::Loop => self.enter_loop(&step, rid, prefix),
        }
    }

    fn skip_decision(
        &self,
        step: &Step,
        prefix: &str,
    ) -> Result<Option<(StepReason, String)>, FlowError> {
        let source = self
            .state
            .ticket
            .get(TICKET_SOURCE)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(tag) = step.skip_on.iter().find(|tag| tag.as_str() == source) {
            return Ok(Some((StepReason::SkipOn, format!("skip_on {tag}"))));
        }
        let Some(source_text) = &step.when else {
            return Ok(None);
        };
        let condition = expr::parse(source_text)?.eval_bool(&self.context(prefix))?;
        if condition {
            Ok(None)
        } else {
            Ok(Some((
                StepReason::WhenFalse,
                format!("when `{source_text}` is false"),
            )))
        }
    }

    fn mark_skipped(&mut self, rid: &str, reason: StepReason, detail: String) {
        if let Some(step_state) = self.state.steps.get_mut(rid) {
            step_state.status = StepStatus::Skipped;
            step_state.reason = Some(reason.as_str().to_string());
            step_state.detail = Some(detail.clone());
            step_state.finished_at = Some(self.at.clone());
        }
        self.journal(format!("step {rid} skipped: {detail}"));
        self.emit(
            FlowEvent::StepChanged,
            Some(rid),
            Some(StepStatus::Skipped.as_wire().to_string()),
        );
    }

    fn begin_leaf(&mut self, rid: &str, status: StepStatus) {
        if let Some(step_state) = self.state.steps.get_mut(rid) {
            step_state.status = status;
            step_state.started_at = Some(self.at.clone());
        }
        self.state.current = Some(rid.to_string());
        self.emit(
            FlowEvent::StepChanged,
            Some(rid),
            Some(status.as_wire().to_string()),
        );
    }

    fn enter_action(&mut self, step: &Step, rid: String) {
        let name = step.use_.clone().unwrap_or_default();
        if let Some(block) = self.def.blocks.get(&name) {
            let steps = block.clone();
            self.enter_container(&rid, FrameKind::Block, format!("{rid}/"), steps);
            return;
        }
        self.begin_leaf(&rid, StepStatus::Running);
        let kind = EffectKind::RunAction {
            step: rid.clone(),
            name,
            with: self.rendered_with(step),
        };
        self.step_effect(&rid, EffectName::RunAction.as_str(), kind);
    }

    fn enter_pipeline(&mut self, step: &Step, rid: String) {
        self.begin_leaf(&rid, StepStatus::Running);
        let kind = EffectKind::StartPipeline {
            step: rid.clone(),
            template: step.use_.clone().unwrap_or_default(),
            with: self.rendered_with(step),
        };
        self.step_effect(&rid, EffectName::StartPipeline.as_str(), kind);
    }

    fn enter_wait(&mut self, step: &Step, rid: String) {
        self.begin_leaf(&rid, StepStatus::WaitingExternal);
        let kind = EffectKind::StartWait {
            step: rid.clone(),
            probe: step.use_.clone().unwrap_or_default(),
            with: self.rendered_with(step),
        };
        self.step_effect(&rid, EffectName::StartWait.as_str(), kind);
    }

    fn enter_human(&mut self, step: &Step, rid: String) {
        self.begin_leaf(&rid, StepStatus::WaitingHuman);
        let kind = EffectKind::CreateInboxEntry {
            step: rid.clone(),
            prompt: step.prompt.clone(),
            choices: step.choices.clone(),
        };
        self.step_effect(&rid, EffectName::CreateInbox.as_str(), kind);
        self.emit(FlowEvent::WaitingYou, Some(&rid), None);
    }

    fn enter_loop(&mut self, step: &Step, rid: String, prefix: String) {
        if let Some(step_state) = self.state.steps.get_mut(&rid) {
            step_state.iteration = 1;
        }
        self.enter_container(&rid, FrameKind::LoopBody, prefix, step.body.clone());
    }

    fn enter_container(&mut self, rid: &str, kind: FrameKind, prefix: String, steps: Vec<Step>) {
        if let Some(step_state) = self.state.steps.get_mut(rid) {
            step_state.status = StepStatus::Running;
            step_state.started_at = Some(self.at.clone());
        }
        self.emit(
            FlowEvent::StepChanged,
            Some(rid),
            Some(StepStatus::Running.as_wire().to_string()),
        );
        self.state.stack.push(Frame {
            kind,
            prefix,
            owner: Some(rid.to_string()),
            steps,
            index: 0,
        });
    }

    fn finish_ok(&mut self, outputs: BTreeMap<String, Value>) {
        let Some(rid) = self.state.current.clone() else {
            return;
        };
        if let Some(step_state) = self.state.steps.get_mut(&rid) {
            step_state.status = StepStatus::Done;
            step_state.outputs = outputs;
            step_state.finished_at = Some(self.at.clone());
        }
        self.journal(format!("step {rid} done"));
        self.emit(
            FlowEvent::StepChanged,
            Some(&rid),
            Some(StepStatus::Done.as_wire().to_string()),
        );
        if let Some(step) = self.top_step().filter(|step| step.pushes) {
            self.insert_after_push(&step);
        }
        self.leave();
    }

    /// Queues the `publish` block, then the `ci` block, after the pushing step: each insertion lands
    /// right after the step, so `ci` goes in first.
    fn insert_after_push(&mut self, pushing_step: &Step) {
        if let Some(runtime_id) = self.insert_block(pushing_step, CI_BLOCK, CI_STEP_PREFIX) {
            if !self.state.inserted_ci.contains(&runtime_id) {
                self.state.inserted_ci.push(runtime_id);
            }
        }
        self.insert_block(pushing_step, PUBLISH_BLOCK, PUBLISH_STEP_PREFIX);
    }

    /// Inserts a reference to `block` right after the current step; its runtime id, when defined.
    fn insert_block(&mut self, pushing_step: &Step, block: &str, prefix: &str) -> Option<String> {
        if !self.def.blocks.contains_key(block) {
            return None;
        }
        let reference = Step {
            id: format!("{prefix}{}", pushing_step.id),
            kind: StepKind::Action,
            use_: Some(block.to_string()),
            ..Step::default()
        };
        let frame = self.state.stack.last_mut()?;
        let runtime_id = format!("{}{}", frame.prefix, reference.id);
        frame.steps.insert(frame.index + 1, reference);
        Some(runtime_id)
    }

    /// Parks the current step on an external condition until it is retried.
    fn wait_current(&mut self, reason: StepReason, detail: String) {
        let Some(rid) = self.state.current.clone() else {
            return;
        };
        if let Some(step_state) = self.state.steps.get_mut(&rid) {
            step_state.status = StepStatus::WaitingExternal;
            step_state.reason = Some(reason.as_str().to_string());
            step_state.detail = Some(detail.clone());
        }
        self.journal(format!("step {rid} waiting: {detail}"));
        self.emit(
            FlowEvent::StepChanged,
            Some(&rid),
            Some(StepStatus::WaitingExternal.as_wire().to_string()),
        );
        self.refresh_status();
    }

    fn fail_current(&mut self, status: StepStatus, reason: StepReason, detail: String) {
        let Some(rid) = self.state.current.clone() else {
            return;
        };
        let warn_only = reason != StepReason::Rejected
            && self
                .top_step()
                .is_some_and(|step| step.on_error == OnError::Warn);
        if let Some(step_state) = self.state.steps.get_mut(&rid) {
            step_state.status = status;
            step_state.reason = Some(reason.as_str().to_string());
            step_state.detail = Some(detail.clone());
            step_state.finished_at = Some(self.at.clone());
            if warn_only {
                step_state.warnings.push(detail.clone());
            }
        }
        self.journal(format!("step {rid} {}: {detail}", status.as_wire()));
        self.emit(
            FlowEvent::StepChanged,
            Some(&rid),
            Some(status.as_wire().to_string()),
        );
        if warn_only {
            self.leave();
            return;
        }
        let flow_status = if status == StepStatus::Escalated {
            FlowStatus::Escalated
        } else {
            FlowStatus::Failed
        };
        self.finish_flow(flow_status);
    }

    fn finish_flow(&mut self, status: FlowStatus) {
        self.state.status = status;
        self.state.ended_at = Some(self.at.clone());
        self.journal(format!("flow {}", status.as_wire()));
        self.emit(FlowEvent::Ended, None, Some(status.as_wire().to_string()));
    }

    fn end_loop_iteration(&mut self, owner: Option<String>) {
        let Some(owner) = owner else {
            self.state.stack.pop();
            return;
        };
        let parent_step = self
            .state
            .stack
            .len()
            .checked_sub(2)
            .and_then(|index| self.state.stack.get(index))
            .and_then(|frame| frame.steps.get(frame.index).cloned());
        let Some(loop_step) = parent_step else {
            self.state.stack.pop();
            return;
        };
        let iteration = self.state.steps.get(&owner).map_or(1, |s| s.iteration);
        let prefix = self.top_prefix();
        let satisfied = match &loop_step.until {
            None => Ok(true),
            Some(source) => {
                expr::parse(source).and_then(|parsed| parsed.eval_bool(&self.context(&prefix)))
            }
        };
        match satisfied {
            Ok(false) if loop_step.max.is_none_or(|max| iteration < max) => {
                self.restart_loop(&owner, iteration + 1);
            }
            Ok(false) => {
                self.state.stack.pop();
                self.state.current = Some(owner);
                let detail = format!("max {} iterations reached", iteration);
                self.fail_current(StepStatus::Escalated, StepReason::MaxIterations, detail);
            }
            Ok(true) => {
                self.state.stack.pop();
                self.state.current = Some(owner);
                self.finish_ok(BTreeMap::new());
            }
            Err(error) => {
                self.state.stack.pop();
                self.state.current = Some(owner);
                self.fail_current(
                    StepStatus::Failed,
                    StepReason::ExpressionError,
                    error.to_string(),
                );
            }
        }
    }

    fn restart_loop(&mut self, owner: &str, iteration: u32) {
        if let Some(step_state) = self.state.steps.get_mut(owner) {
            step_state.iteration = iteration;
        }
        if let Some(frame) = self.state.stack.last_mut() {
            frame.steps.retain(|step| {
                !step.id.starts_with(CI_STEP_PREFIX) && !step.id.starts_with(PUBLISH_STEP_PREFIX)
            });
            frame.index = 0;
        }
        self.journal(format!("loop {owner} iteration {iteration}"));
    }

    fn expect_effect(
        &self,
        given_id: &str,
        kind_name: &str,
        allowed: &[StepStatus],
    ) -> Result<String, FlowError> {
        let stale = || {
            rejected(
                RejectReason::StaleEffect,
                format!("effect `{given_id}` is not pending"),
            )
        };
        let rid = self.state.current.clone().ok_or_else(stale)?;
        let step_state = self.state.steps.get(&rid).ok_or_else(stale)?;
        let expected = effect_id(&rid, step_state.iteration, kind_name);
        if given_id == expected && allowed.contains(&step_state.status) {
            Ok(rid)
        } else {
            Err(stale())
        }
    }

    fn action_finished(&mut self, given_id: &str, outcome: ActionOutcome) -> Result<(), FlowError> {
        if given_id.starts_with(MANUAL_PREFIX) {
            let line = match outcome {
                ActionOutcome::Ok(_) => format!("manual effect {given_id} finished"),
                ActionOutcome::TechnicalError(reason) | ActionOutcome::BusinessError(reason) => {
                    format!("manual effect {given_id} failed: {reason}")
                }
                ActionOutcome::Skipped { reason } => {
                    format!("manual effect {given_id} skipped: {reason}")
                }
                ActionOutcome::WaitingExternal { reason, .. } => {
                    format!("manual effect {given_id} waiting: {reason}")
                }
            };
            self.journal(line);
            return Ok(());
        }
        self.expect_effect(
            given_id,
            EffectName::RunAction.as_str(),
            &[StepStatus::Running, StepStatus::WaitingExternal],
        )?;
        match outcome {
            ActionOutcome::Ok(outputs) => self.finish_ok(outputs),
            ActionOutcome::Skipped { reason } => {
                if let Some(rid) = self.state.current.clone() {
                    self.mark_skipped(&rid, StepReason::ActionSkipped, reason);
                    self.leave();
                }
            }
            ActionOutcome::WaitingExternal { reason, outputs } => {
                if let Some(state) = self
                    .state
                    .current
                    .as_ref()
                    .and_then(|step| self.state.steps.get_mut(step))
                {
                    state.outputs = outputs;
                }
                self.wait_current(StepReason::ActionWaiting, reason);
                return Ok(());
            }
            ActionOutcome::TechnicalError(message) => {
                self.fail_current(StepStatus::Failed, StepReason::TechnicalError, message)
            }
            ActionOutcome::BusinessError(message) => {
                self.fail_current(StepStatus::Escalated, StepReason::BusinessError, message)
            }
        }
        self.drive();
        Ok(())
    }

    fn pipeline_finished(
        &mut self,
        given_id: &str,
        run_status: &str,
        outputs: BTreeMap<String, Value>,
    ) -> Result<(), FlowError> {
        if given_id.starts_with(MANUAL_PREFIX) {
            self.journal(format!("manual pipeline {given_id} ended {run_status}"));
            let label = given_id.split(':').next().unwrap_or(given_id);
            let publish = self.state.manual_publish.remove(label);
            if publish && PipelineRunStatus::parse(run_status) == Some(PipelineRunStatus::Done) {
                let step = format!("{label}{PUBLISH_SUFFIX}");
                self.journal(format!("manual action {PUBLISH_ACTION} ({step})"));
                self.push_effect(
                    effect_id(&step, 0, EffectName::RunAction.as_str()),
                    EffectKind::RunAction {
                        step,
                        name: PUBLISH_ACTION.to_string(),
                        with: BTreeMap::new(),
                    },
                );
            }
            return Ok(());
        }
        self.expect_effect(
            given_id,
            EffectName::StartPipeline.as_str(),
            &[StepStatus::Running],
        )?;
        match PipelineRunStatus::parse(run_status) {
            Some(PipelineRunStatus::Done) => self.finish_ok(outputs),
            Some(PipelineRunStatus::Failed) => self.fail_current(
                StepStatus::Failed,
                StepReason::PipelineFailed,
                format!("pipeline run {run_status}"),
            ),
            _ => self.fail_current(
                StepStatus::Escalated,
                StepReason::RunStatus,
                format!("pipeline run ended {run_status}"),
            ),
        }
        self.drive();
        Ok(())
    }

    fn pipeline_orphaned(&mut self, given_id: &str) -> Result<(), FlowError> {
        self.expect_effect(
            given_id,
            EffectName::StartPipeline.as_str(),
            &[StepStatus::Running],
        )?;
        self.fail_current(
            StepStatus::Failed,
            StepReason::Orphaned,
            "session lost while the daemon was down".to_string(),
        );
        self.drive();
        Ok(())
    }

    fn wait_satisfied(
        &mut self,
        given_id: &str,
        outputs: BTreeMap<String, Value>,
    ) -> Result<(), FlowError> {
        self.expect_effect(
            given_id,
            EffectName::StartWait.as_str(),
            &[StepStatus::WaitingExternal],
        )?;
        self.finish_ok(outputs);
        self.drive();
        Ok(())
    }

    fn budget_insufficient(&mut self, step: &str) -> Result<(), FlowError> {
        let running_pipeline = self.state.current.as_deref() == Some(step)
            && self.state.steps.get(step).map(|s| s.status) == Some(StepStatus::Running)
            && self
                .top_step()
                .is_some_and(|top| top.kind == StepKind::Pipeline);
        if !running_pipeline {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` is not a starting pipeline"),
            ));
        }
        if let Some(step_state) = self.state.steps.get_mut(step) {
            step_state.status = StepStatus::WaitingExternal;
            step_state.reason = Some(StepReason::Budget.as_str().to_string());
        }
        self.journal(format!("step {step} waiting for budget"));
        self.emit(
            FlowEvent::StepChanged,
            Some(step),
            Some(StepStatus::WaitingExternal.as_wire().to_string()),
        );
        Ok(())
    }

    fn resource_busy(&mut self, step: &str, resource: &str) -> Result<(), FlowError> {
        let running_pipeline = self.state.current.as_deref() == Some(step)
            && self.state.steps.get(step).map(|s| s.status) == Some(StepStatus::Running)
            && self
                .top_step()
                .is_some_and(|top| top.kind == StepKind::Pipeline);
        if !running_pipeline {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` is not a starting pipeline"),
            ));
        }
        if let Some(step_state) = self.state.steps.get_mut(step) {
            step_state.status = StepStatus::WaitingExternal;
            step_state.reason = Some(format!("{}{resource}", resource::REASON_PREFIX));
        }
        self.journal(format!("step {step} waiting for resource {resource}"));
        self.emit(
            FlowEvent::StepChanged,
            Some(step),
            Some(StepStatus::WaitingExternal.as_wire().to_string()),
        );
        Ok(())
    }

    /// A pipeline step about to start waits for a human: its ticket reached the token ceiling.
    fn ticket_budget_exceeded(&mut self, step: &str, detail: &str) -> Result<(), FlowError> {
        let starting_pipeline = self.state.current.as_deref() == Some(step)
            && self.state.steps.get(step).map(|s| s.status) == Some(StepStatus::Running)
            && self
                .top_step()
                .is_some_and(|top| top.kind == StepKind::Pipeline);
        if !starting_pipeline {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` is not a starting pipeline"),
            ));
        }
        if let Some(step_state) = self.state.steps.get_mut(step) {
            step_state.status = StepStatus::WaitingHuman;
            step_state.reason = Some(StepReason::TicketBudget.as_str().to_string());
            step_state.detail = Some(detail.to_string());
        }
        self.journal(format!("step {step} waiting for a human: {detail}"));
        self.emit(
            FlowEvent::StepChanged,
            Some(step),
            Some(StepStatus::WaitingHuman.as_wire().to_string()),
        );
        self.emit(FlowEvent::WaitingYou, Some(step), Some(detail.to_string()));
        Ok(())
    }

    /// Decision on a step held by the ticket ceiling: approve starts the pipeline, reject escalates.
    fn ticket_budget_decision(
        &mut self,
        step: &str,
        decision: &str,
        actor: &str,
    ) -> Result<(), FlowError> {
        self.journal(format!(
            "ticket budget decision `{decision}` on {step} by {actor}"
        ));
        self.step_effect(
            step,
            EffectName::ResolveInbox.as_str(),
            EffectKind::ResolveInboxEntry {
                step: step.to_string(),
            },
        );
        if HumanDecision::parse(decision).is_some_and(HumanDecision::is_ok) {
            if let Some(top) = self.top_step() {
                let prefix = self.top_prefix();
                self.enter_step(top, prefix);
            }
            self.drive();
        } else {
            self.fail_current(
                StepStatus::Escalated,
                StepReason::TicketBudget,
                format!("{REJECTED_DETAIL} {actor}"),
            );
        }
        Ok(())
    }

    fn human_decision(
        &mut self,
        step: &str,
        decision: &str,
        actor: &str,
        report: Option<String>,
    ) -> Result<(), FlowError> {
        if !self.state.steps.contains_key(step) && !self.defines_step(step) {
            return Err(rejected(
                RejectReason::UnknownStep,
                format!("unknown step `{step}`"),
            ));
        }
        let waiting = self.state.current.as_deref() == Some(step)
            && self.current_status() == Some(StepStatus::WaitingHuman);
        if !waiting {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` is not waiting for a human"),
            ));
        }
        let held_by_ceiling = self.state.steps.get(step).and_then(|s| s.reason.as_deref())
            == Some(StepReason::TicketBudget.as_str());
        if held_by_ceiling {
            if HumanDecision::parse(decision).is_none() {
                return Err(rejected(
                    RejectReason::InvalidChoice,
                    format!("`{decision}` is not a decision"),
                ));
            }
            return self.ticket_budget_decision(step, decision, actor);
        }
        let choices = self.top_step().map(|top| top.choices).unwrap_or_default();
        if !choices.is_empty() && !choices.iter().any(|choice| choice == decision) {
            return Err(rejected(
                RejectReason::InvalidChoice,
                format!("`{decision}` is not a choice of step `{step}`"),
            ));
        }
        let at = self.at.clone();
        self.journal(format!(
            "human decision `{decision}` on {step} by {actor} at {at}"
        ));
        self.step_effect(
            step,
            EffectName::ResolveInbox.as_str(),
            EffectKind::ResolveInboxEntry {
                step: step.to_string(),
            },
        );
        let mut outputs = BTreeMap::new();
        outputs.insert(OUTPUT_DECISION.to_string(), json!(decision));
        outputs.insert(OUTPUT_ACTOR.to_string(), json!(actor));
        outputs.insert(OUTPUT_AT.to_string(), json!(at));
        outputs.insert(OUTPUT_REPORT.to_string(), json!(report));
        outputs.insert(
            OUTPUT_OK.to_string(),
            json!(HumanDecision::parse(decision).is_some_and(HumanDecision::is_ok)),
        );
        if HumanDecision::parse(decision) == Some(HumanDecision::Reject) {
            if let Some(step_state) = self.state.steps.get_mut(step) {
                step_state.outputs = outputs;
            }
            self.fail_current(
                StepStatus::Escalated,
                StepReason::Rejected,
                format!("{REJECTED_DETAIL} {actor}"),
            );
            return Ok(());
        }
        self.finish_ok(outputs);
        self.drive();
        Ok(())
    }

    fn pause(&mut self) -> Result<(), FlowError> {
        self.state.pause_requested = true;
        self.journal("pause requested".to_string());
        Ok(())
    }

    fn resume(&mut self) -> Result<(), FlowError> {
        if !self.state.pause_requested {
            return Err(rejected(RejectReason::NotPaused, "run is not paused"));
        }
        self.state.pause_requested = false;
        self.journal("resumed".to_string());
        self.drive();
        Ok(())
    }

    fn abort(&mut self) -> Result<(), FlowError> {
        let active = matches!(
            self.current_status(),
            Some(StepStatus::Running | StepStatus::WaitingHuman | StepStatus::WaitingExternal)
        );
        if let (Some(rid), Some(step), true) = (self.state.current.clone(), self.top_step(), active)
        {
            let status = self.current_status();
            match (step.kind, status) {
                (StepKind::Pipeline, Some(StepStatus::Running)) => self.step_effect(
                    &rid,
                    EffectName::CancelSession.as_str(),
                    EffectKind::CancelSession { step: rid.clone() },
                ),
                (StepKind::Human | StepKind::Pipeline, Some(StepStatus::WaitingHuman)) => self
                    .step_effect(
                        &rid,
                        EffectName::ResolveInbox.as_str(),
                        EffectKind::ResolveInboxEntry { step: rid.clone() },
                    ),
                _ => {}
            }
            if let Some(step_state) = self.state.steps.get_mut(&rid) {
                step_state.status = StepStatus::Failed;
                step_state.reason = Some(StepReason::Aborted.as_str().to_string());
                step_state.finished_at = Some(self.at.clone());
            }
        }
        self.finish_flow(FlowStatus::Aborted);
        Ok(())
    }

    fn defines_step(&self, rid: &str) -> bool {
        let id = rid.rsplit('/').next().unwrap_or(rid);
        defines(&self.def.steps, id) || self.def.blocks.values().any(|block| defines(block, id))
    }

    fn current_status(&self) -> Option<StepStatus> {
        let rid = self.state.current.as_ref()?;
        self.state
            .steps
            .get(rid)
            .map(|step_state| step_state.status)
    }

    fn require_retryable(&self, step: &str) -> Result<(), FlowError> {
        if !self.state.steps.contains_key(step) {
            return Err(rejected(
                RejectReason::UnknownStep,
                format!("unknown step `{step}`"),
            ));
        }
        let step_state = &self.state.steps[step];
        let waiting_budget = step_state.status == StepStatus::WaitingExternal
            && matches!(
                step_state.reason.as_deref(),
                Some(reason) if reason == StepReason::Budget.as_str()
                    || reason == StepReason::ActionWaiting.as_str()
                    || reason.starts_with(resource::REASON_PREFIX)
            );
        let retryable = matches!(
            step_state.status,
            StepStatus::Failed | StepStatus::Escalated
        ) || waiting_budget;
        if self.state.current.as_deref() == Some(step) && retryable {
            Ok(())
        } else {
            Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` cannot be retried or skipped now"),
            ))
        }
    }

    fn retry(&mut self, step: &str) -> Result<(), FlowError> {
        self.require_retryable(step)?;
        let waiting_outputs = (self.state.steps[step].status == StepStatus::WaitingExternal)
            .then(|| self.state.steps[step].outputs.clone());
        self.journal(format!("retry step {step}"));
        self.state.status = FlowStatus::Running;
        self.state.ended_at = None;
        if let Some(top) = self.top_step() {
            let prefix = self.top_prefix();
            self.enter_step(top, prefix);
            if let Some(outputs) = waiting_outputs {
                if let Some(state) = self.state.steps.get_mut(step) {
                    state.outputs = outputs;
                }
            }
        }
        self.drive();
        Ok(())
    }

    fn skip(&mut self, step: &str) -> Result<(), FlowError> {
        self.require_retryable(step)?;
        if self.state.steps[step].reason.as_deref() == Some(StepReason::Rejected.as_str()) {
            return Err(rejected(RejectReason::WrongState, REJECTED_SKIP_REFUSAL));
        }
        self.state.status = FlowStatus::Running;
        self.state.ended_at = None;
        self.mark_skipped(step, StepReason::ManualSkip, "skipped by hand".to_string());
        self.leave();
        self.drive();
        Ok(())
    }

    /// A failed or escalated current step that is not a human step counts as passed.
    fn mark_passed(&mut self, step: &str) -> Result<(), FlowError> {
        let failed = self.state.current.as_deref() == Some(step)
            && matches!(
                self.current_status(),
                Some(StepStatus::Failed | StepStatus::Escalated)
            );
        if !failed {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` has not failed: it cannot be marked passed"),
            ));
        }
        if self
            .top_step()
            .is_some_and(|top| top.kind == StepKind::Human)
        {
            return Err(rejected(
                RejectReason::WrongState,
                format!("step `{step}` is a human step: it is never marked passed"),
            ));
        }
        self.state.status = FlowStatus::Running;
        self.state.ended_at = None;
        self.journal(format!("step {step} marked passed"));
        let mut outputs = self.state.steps[step].outputs.clone();
        outputs.insert(OUTPUT_OK.to_string(), json!(true));
        outputs.insert(OUTPUT_MARKED_PASSED.to_string(), json!(true));
        self.finish_ok(outputs);
        self.drive();
        Ok(())
    }

    fn manual(&mut self, request: ManualRequest) -> Result<(), FlowError> {
        let index = self.state.manual_count;
        self.state.manual_count += 1;
        let label = format!("{MANUAL_PREFIX}{index}");
        let (kind_name, kind, line) = match request {
            ManualRequest::Action { name, with } => (
                EffectName::RunAction.as_str(),
                EffectKind::RunAction {
                    step: label.clone(),
                    name: name.clone(),
                    with,
                },
                format!("manual action {name}"),
            ),
            ManualRequest::Pipeline {
                template,
                with,
                publish,
            } => {
                if publish {
                    self.state.manual_publish.insert(label.clone());
                }
                (
                    EffectName::StartPipeline.as_str(),
                    EffectKind::StartPipeline {
                        step: label.clone(),
                        template: template.clone(),
                        with,
                    },
                    format!("manual pipeline {template}"),
                )
            }
        };
        self.journal(line);
        self.push_effect(format!("{label}:0:{kind_name}"), kind);
        Ok(())
    }

    fn refresh_status(&mut self) {
        if self.state.status.is_terminal() {
            return;
        }
        let active = self.current_status();
        self.state.status = if self.state.pause_requested {
            match active {
                Some(StepStatus::Running | StepStatus::WaitingExternal) => FlowStatus::Pausing,
                _ => FlowStatus::Paused,
            }
        } else {
            match active {
                Some(StepStatus::WaitingHuman) => FlowStatus::WaitingHuman,
                Some(StepStatus::WaitingExternal) => FlowStatus::WaitingExternal,
                _ => FlowStatus::Running,
            }
        };
    }
}

#[cfg(test)]
#[path = "../tests/unit/machine_tests.rs"]
mod tests;
