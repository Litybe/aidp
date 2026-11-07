//! Pipeline YAML data models.

use crate::models::enums::{
    EffortLevel, HarnessGapPolicy, Isolation, ItemFailurePolicy, ModelTier, ToolEnforcement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_KIND_PIPELINE: &str = "pipeline";
const DEFAULT_INPUT_KIND: &str = "string";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pipeline {
    pub version: u32,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `pipeline` (default) = runnable on its own; `fragment` = a composition
    /// brick meant to be pulled in via `uses:`, never launched alone.
    #[serde(default = "default_kind_pipeline")]
    pub kind: String,
    /// Imperative rules for this pipeline as a whole.
    /// Capped at 10 rules by the schema.
    #[serde(default)]
    pub policy: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputSpec>,
    #[serde(default)]
    pub settings: Settings,
    pub stages: Vec<Stage>,
}
fn default_kind_pipeline() -> String {
    DEFAULT_KIND_PIPELINE.into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default = "default_max_iter")]
    pub max_correction_iterations: u32,
    /// Optional ceiling on flattened stages after composition expansion.
    /// Defaults to `compose::DEFAULT_MAX_TOTAL_STAGES` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_stages: Option<usize>,
    /// Ceiling on stages a single Workflow pass accumulates before stopping
    /// at a not-yet-crossed composite entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stages_per_pass: Option<usize>,
    /// Automatic retries of a stage whose sub-agent returns `failed`.
    /// 0 (default) emits the single `agent()` call.
    #[serde(default)]
    pub max_stage_retries: u32,
    /// When `true`, stages without `effort:` emit a per-role default effort.
    /// Defaults to `false`.
    #[serde(default)]
    pub emit_effort: bool,
    /// Optional workspace knowledge-graph (Graphify) config.
    /// When `enabled: true`, `run::init` builds it once (see `crate::kg`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_graph: Option<crate::kg::KnowledgeGraphSettings>,
    /// Whether `run reconcile` runs `scope::verify_run` at the end of a run.
    /// Defaults to `true`; a template opts out.
    #[serde(default = "default_verify_scope")]
    pub verify_scope: bool,
    /// Admission floor `pool(...)` reads before dispatching an item, in output
    /// tokens only. Below it the item comes back `cancelled`; not a cost figure.
    #[serde(default = "default_budget_floor_tokens")]
    pub budget_floor_tokens: u64,
    /// What a `cancelled` or failed item does to the run: `pause` (default)
    /// halts for a human, `continue` lets the stage reach its merge.
    #[serde(default)]
    pub on_item_failure: ItemFailurePolicy,
    /// `off` (default) emits the bare role; `strict` resolves a tool-restricted
    /// `agentType` for read-only roles. `executor` is never restricted.
    #[serde(default)]
    pub tool_enforcement: ToolEnforcement,
    /// Agent provider id per role (analyzer/planner/executor/reviewer).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}
fn default_max_iter() -> u32 {
    2
}
fn default_verify_scope() -> bool {
    true
}
fn default_budget_floor_tokens() -> u64 {
    50_000
}
/// Manual `Default` mirroring each field's serde default; a derived one
/// would fail schema validation for pipelines without a `settings:` block.
impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_correction_iterations: default_max_iter(),
            max_total_stages: None,
            max_stages_per_pass: None,
            max_stage_retries: 0,
            emit_effort: false,
            knowledge_graph: None,
            verify_scope: default_verify_scope(),
            budget_floor_tokens: default_budget_floor_tokens(),
            on_item_failure: ItemFailurePolicy::default(),
            tool_enforcement: ToolEnforcement::default(),
            providers: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InputSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
    #[serde(default)]
    pub description: String,
}
fn default_kind() -> String {
    DEFAULT_INPUT_KIND.into()
}

/// Per-value refinement of a stage's `skills:` list, keyed by the value of one
/// declared input (`input`). See [`SkillsBy::resolve`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillsBy {
    /// Name of the `inputs.*` entry whose value selects a bucket in `map`.
    /// Falls back to `default` when absent or unmapped.
    pub input: String,
    #[serde(default)]
    pub map: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub default: Vec<String>,
}

impl SkillsBy {
    /// Returns `base` followed by the list picked from `map` for `value`
    /// (else `default`), de-duplicated keeping the first occurrence in order.
    pub fn resolve(&self, base: &[String], value: Option<&str>) -> Vec<String> {
        let picked = value.and_then(|v| self.map.get(v)).unwrap_or(&self.default);
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for skill in base.iter().chain(picked.iter()) {
            if seen.insert(skill.clone()) {
                out.push(skill.clone());
            }
        }
        out
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Stage {
    pub name: String,
    /// Human-readable description of what this stage does.
    #[serde(default)]
    pub description: String,
    /// Imperative rules scoped to this stage.
    /// Capped at 10 rules by the schema.
    #[serde(default)]
    pub policy: Vec<String>,
    /// Kind of stage, e.g. `loop` or `pipeline`; skipped when `None` so re-validation passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// For a `type: pipeline` stage: id (resolved via discovery) or path of
    /// another pipeline template to inline here. Enables composition / reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<String>,
    /// Set by `compose::expand` on a composite's entry stage to the composite's
    /// own name; never authored. Forces a new Workflow pass at this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composite_entry: Option<String>,
    /// Never authored: the `when:` of every enclosing composite, stamped by
    /// `compose::expand` on every inlined stage. All of them must hold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub composite_when: Vec<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Override the Claude model family for this stage's sub-agent dispatch.
    /// Maps directly to the Task tool's `model` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelTier>,
    /// Effort level for this stage's sub-agent. Omitted, it inherits the session's effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortLevel>,
    /// Agent provider id running this stage; overrides `settings.providers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    /// Per-value refinement of `skills`, keyed by an `inputs.*` value.
    /// See [`SkillsBy::resolve`]; `None` leaves `skills` untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills_by: Option<SkillsBy>,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_yaml::Value>,
    #[serde(default)]
    pub outputs: Vec<serde_yaml::Value>,
    #[serde(default)]
    pub with: BTreeMap<String, serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeSpec>,
    /// When `Some(Isolation::Worktree)`, the stage's sub-agent runs in an isolated git
    /// worktree so its writes cannot collide with parallel stages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<Isolation>,
    /// Explicit `allow`/`deny` tool list overriding `crate::toolset`'s matrix
    /// under `tool_enforcement: strict`. Inert while enforcement is `off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsSpec>,
    /// Fans this stage out into parallel sub-agents plus a merge sub-agent.
    /// See [`FanOutSpec`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<FanOutSpec>,
    /// Merges upstream stages' `findings.json` into this stage's own via
    /// `aidp findings merge`. Only supported on a plain sequential agent stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_findings: Option<MergeFindingsSpec>,
    /// `pause` halts the run when the reply reports `signals.has_harness_gap`.
    /// Defaults to `continue`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_harness_gap: Option<HarnessGapPolicy>,
    /// Expands this stage from an upstream planner's `plan.json` task graph.
    /// See [`PlanTasksSpec`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_tasks: Option<PlanTasksSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<serde_yaml::Value>,
    /// Marks this stage as producing durable project knowledge under
    /// `<vault>/Knowledge/<repo>/`: `true`/`all`, `service` or `feature`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<KnowledgeSpec>,
    #[serde(default)]
    pub body: Vec<Stage>,
}

/// Intra-stage fan-out over `items` (static labels) or `from` (an upstream
/// array signal); exactly one shard source must be given.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FanOutSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Ceiling on concurrent shards; extras fold into the last shard or drop.
    #[serde(default = "default_fanout_max")]
    pub max: u32,
    /// `true` (default) folds items past `max` into the last shard; `false`
    /// truncates to `max` and `max` becomes the concurrency size.
    #[serde(default = "default_fanout_fold")]
    pub fold: bool,
    /// Extra instruction appended to the merge sub-agent's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_hint: Option<String>,
}
fn default_fanout_max() -> u32 {
    8
}
fn default_fanout_fold() -> bool {
    true
}

/// `merge_findings:` merges the `findings.json` of every `from` stage into
/// this stage's own `out` file via `aidp findings merge`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MergeFindingsSpec {
    /// Names of the producer stages, each declaring a `findings` output.
    pub from: Vec<String>,
    /// Filename, relative to this stage's dir, of the merged `findings.json`.
    pub out: String,
}

/// Plan-driven expansion: an upstream planner's `plan.json` task graph is
/// compiled into one sub-agent per task plus a merge sub-agent, in two passes.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanTasksSpec {
    /// Handlebars ref to the upstream plan artifact (a `.json` file).
    pub from: String,
    /// Ceiling on concurrently-running tasks; tasks are never folded together.
    #[serde(default = "default_plan_max")]
    pub max: u32,
    /// Task kinds this stage claims (empty = every task in the plan).
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Extra instruction appended to the merge sub-agent's prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_hint: Option<String>,
    /// Per-kind reduction of the stage's skill list: a task whose `kind` has an
    /// entry loads only those skills. The merge sub-agent keeps the full list.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skills_by_kind: BTreeMap<String, Vec<String>>,
}
fn default_plan_max() -> u32 {
    4
}

/// Which layer of the knowledge base a `knowledge:` stage curates.
/// Declared by the template, never picked by the agent at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeLayer {
    /// The transverse notes only (`architecture.md`, `domain.md`, …).
    Service,
    /// `Features/<slug>.md` only — one note per business feature.
    Feature,
    /// The whole base, both layers. What a bare `knowledge: true` means.
    All,
}

/// `knowledge:` accepts a boolean or a layer name.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum KnowledgeSpec {
    Flag(bool),
    Layer(KnowledgeLayer),
}

impl KnowledgeSpec {
    /// The layer this spec selects, or `None` when it is an explicit `false`.
    pub fn layer(&self) -> Option<KnowledgeLayer> {
        match self {
            Self::Flag(true) => Some(KnowledgeLayer::All),
            Self::Flag(false) => None,
            Self::Layer(l) => Some(*l),
        }
    }
}

/// Which directory scope a stage's artefacts belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// `<run_dir>/<stage>/` — disposable, one run only.
    Run,
    /// `<ticket_root>/<stage>/` — shared by every run of the same ticket.
    Ticket,
    /// `<vault>/Knowledge/<repo>/` — the transverse layer, shared by every run
    /// of the repo.
    Knowledge,
    /// `<vault>/Knowledge/<repo>/Features/` — the per-feature layer, kept apart
    /// so `stage_dir_only` grants `Features/` and nothing else.
    KnowledgeFeature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScopeSpec {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub reads: Vec<String>,
}

/// A stage's explicit `tools:` allow/deny lists, carried as data.
/// Parsed but not yet resolved into the emitted `agentType`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolsSpec {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[cfg(test)]
#[path = "../../tests/unit/pipeline_provider_fields_tests.rs"]
mod pipeline_provider_fields_tests;
