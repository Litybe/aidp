//! The executable plan: `plan.json` → a scheduled task graph.
//!
//! v7 up to now had the planner write prose (`plan.md`) plus an opaque
//! `signals.shards` array of labels, and the executor stage fanned out over
//! those labels. Two things were wrong with that: the *shape* of the work was
//! decided by a sentence ("handlers + validators for X"), and the parallelism
//! was decided by the model, not by `aidp`.
//!
//! Here the planner instead emits a **task graph**: one object per task, each
//! carrying its own prompt/instructions plus the two facts a scheduler needs —
//! `needs` (real data dependencies) and `files` (the task's write set). This
//! module loads it, validates it (schema + structure + acyclicity) and produces
//! a [`Schedule`]:
//!
//!   * declared `needs` become edges;
//!   * any two otherwise-concurrent tasks whose write sets intersect get an
//!     extra edge in plan order, so a shared working tree can never be raced
//!     (recorded in [`Serialization`] so the decision is auditable, never
//!     silent);
//!   * `waves` are reported for humans; the emitter dispatches each task as
//!     soon as *its own* dependencies resolve (no wave barrier).
//!
//! [`crate::workflow_emit`] turns a `Schedule` into the tail of the Workflow
//! script; [`ScheduleState`] is the slice persisted into run-state so
//! `aidp run summary` and `aidp workflow plan-status` can tell the truth about
//! what ran.

use crate::clock::{Clock, SystemClock};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Contract marker written by the planner (and validated when present).
pub const PLAN_SCHEMA_ID: &str = "aidp-plan.v1";

/// Task kinds a `plan_tasks:` stage may claim.
pub const KINDS: [&str; 5] = ["code", "tests", "docs", "infra", "chore"];

const SCHEMA: &str = include_str!("../../../schema/plan.schema.json");

// ---------------------------------------------------------------------------
// The plan as written by the planner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub jira_keys: Vec<String>,
    /// Files no task may write: the merge sub-agent applies every change to them.
    #[serde(default)]
    pub shared_files: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub prompt: String,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    /// The write set: every repo-relative file this task creates or modifies.
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub verification: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    // No per-task `agent`/`model`/`skills`: the role, the model and the skills a
    // task runs with are the *stage's* — the template owns that decision, not the
    // plan. A task only says what work to do and which files it owns.
    #[serde(default)]
    pub jira_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

fn default_kind() -> String {
    "code".to_string()
}

// ---------------------------------------------------------------------------
// Loading + validation
// ---------------------------------------------------------------------------

/// Read and validate `plan.json`. Errors are phrased for the *planner*: the file
/// is a machine contract produced by a sub-agent, so a violation means that
/// stage must be re-run, and the message has to say exactly what to fix.
/// Largest `plan.json` `load` accepts (LOC-27): refused before deserialization.
pub const MAX_PLAN_BYTES: u64 = 1024 * 1024;

pub fn load(path: &Path) -> anyhow::Result<Plan> {
    use anyhow::Context;
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_PLAN_BYTES + 1).read_to_string(&mut text))
        .with_context(|| format!("failed to read plan {}", path.display()))?;
    if text.len() as u64 > MAX_PLAN_BYTES {
        anyhow::bail!(
            "plan {} exceeds the {MAX_PLAN_BYTES}-byte limit",
            path.display()
        );
    }
    parse(&text).map_err(|e| anyhow::anyhow!("{} — {e}", path.display()))
}

/// Same as [`load`] on an in-memory document (used by tests and by callers that
/// already hold the bytes).
pub fn parse(text: &str) -> anyhow::Result<Plan> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("plan is not valid JSON: {e}"))?;
    validate_json(&json)?;
    let plan: Plan = serde_json::from_value(json)
        .map_err(|e| anyhow::anyhow!("plan does not match the aidp-plan.v1 shape: {e}"))?;
    validate_structure(&plan)?;
    Ok(plan)
}

fn validate_json(instance: &serde_json::Value) -> anyhow::Result<()> {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA)?;
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .map_err(|e| anyhow::anyhow!("plan schema compile: {e}"))?;
    if let Err(errors) = compiled.validate(instance) {
        let msgs: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        anyhow::bail!(
            "plan violates schema/plan.schema.json:\n  {}",
            msgs.join("\n  ")
        );
    }
    Ok(())
}

/// Structural rules the JSON Schema cannot express: unique ids, resolvable
/// `needs`, no self-dependency, no cycle.
fn validate_structure(plan: &Plan) -> anyhow::Result<()> {
    if let Some(s) = &plan.schema {
        if s != PLAN_SCHEMA_ID {
            anyhow::bail!("plan `schema` is `{s}`, expected `{PLAN_SCHEMA_ID}`");
        }
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for t in &plan.tasks {
        if !seen.insert(t.id.as_str()) {
            anyhow::bail!("duplicate task id `{}` — ids must be unique", t.id);
        }
    }
    for t in &plan.tasks {
        for d in &t.needs {
            if d == &t.id {
                anyhow::bail!("task `{}` depends on itself", t.id);
            }
            if !seen.contains(d.as_str()) {
                anyhow::bail!(
                    "task `{}` needs unknown task `{d}` — every `needs` entry must be another task's id",
                    t.id
                );
            }
        }
        if !KINDS.contains(&t.kind.as_str()) {
            anyhow::bail!(
                "task `{}` has kind `{}` — expected one of {:?}",
                t.id,
                t.kind,
                KINDS
            );
        }
    }
    // Cycle detection over the declared edges only (implicit conflict edges are
    // added later, and by construction cannot introduce a cycle).
    let idx: BTreeMap<&str, usize> = plan
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let mut indeg = vec![0usize; plan.tasks.len()];
    for t in &plan.tasks {
        indeg[idx[t.id.as_str()]] += t.needs.len();
    }
    let mut queue: Vec<usize> = (0..plan.tasks.len()).filter(|i| indeg[*i] == 0).collect();
    let mut placed = 0usize;
    while let Some(i) = queue.pop() {
        placed += 1;
        for (j, t) in plan.tasks.iter().enumerate() {
            if t.needs.iter().any(|d| idx[d.as_str()] == i) {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    queue.push(j);
                }
            }
        }
    }
    if placed != plan.tasks.len() {
        let stuck: Vec<&str> = plan
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| indeg[*i] > 0)
            .map(|(_, t)| t.id.as_str())
            .collect();
        anyhow::bail!("cycle in task `needs` among: {stuck:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

/// One compiled task: the plan's task plus the wave it lands in.
///
/// `deps` and `mutex_after` are kept apart on purpose (C9, Lot 3
/// Capitalisation): `deps` is the planner's semantic `needs` graph — a real
/// data dependency whose failure must propagate to this task. `mutex_after`
/// is an ordering `aidp` inserted on its own because this task and an
/// earlier one write the same file; execution still waits for it (a shared
/// working tree can never be raced), but a failed mutex predecessor must
/// NOT cancel this task — it only deserves a warning (see `runTask` in
/// `workflow_emit.rs`). Both sets still drive the *scheduler*'s topological
/// order and waves identically to before this split (see `schedule`'s
/// internal `effective_deps`) — only what gets published here changed.
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task: Task,
    /// Declared, semantic dependencies only (`needs`, resolved within this
    /// stage's claimed tasks). A `blocked`/`failed` entry here must cancel
    /// this task.
    pub deps: Vec<String>,
    /// Tasks this one is serialized after purely because they write a file
    /// this task also writes. Never a semantic dependency: a failed entry
    /// here must NOT cancel this task, only warn it.
    pub mutex_after: Vec<String>,
    pub wave: usize,
}

/// An ordering `aidp` inserted because two concurrent tasks wrote the same file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Serialization {
    pub before: String,
    pub after: String,
    pub files: Vec<String>,
}

/// The compiled schedule for one `plan_tasks:` stage.
#[derive(Debug, Clone)]
pub struct Schedule {
    /// Tasks in topological order — the emitter relies on this so a dependency's
    /// promise always exists before its dependents reference it.
    pub tasks: Vec<ScheduledTask>,
    /// Wave membership (task ids), for reporting only. Execution is
    /// dependency-driven, not wave-gated.
    pub waves: Vec<Vec<String>>,
    pub serializations: Vec<Serialization>,
    /// Non-fatal contract slips worth surfacing (e.g. a `shared_files` entry
    /// that a task also claims as its own).
    pub warnings: Vec<String>,
    pub max_parallel: u32,
    /// Widest wave — the most tasks that could ever run at once.
    pub width: usize,
    pub shared_files: Vec<String>,
}

impl Schedule {
    /// Concurrency actually achievable: the cap, bounded by the graph's width.
    pub fn effective_parallel(&self) -> usize {
        (self.max_parallel as usize).min(self.width.max(1))
    }

    pub fn task(&self, id: &str) -> Option<&ScheduledTask> {
        self.tasks.iter().find(|t| t.task.id == id)
    }

    /// The persistable slice of this schedule (run-state / `plan-status`).
    pub fn to_state(&self, stage: &str, plan_path: &Path) -> ScheduleState {
        self.to_state_with(stage, plan_path, &SystemClock)
    }

    /// [`Schedule::to_state`], taking its `compiled_at` timestamp from `clock`
    /// instead of reading the wall clock directly.
    pub fn to_state_with(&self, stage: &str, plan_path: &Path, clock: &dyn Clock) -> ScheduleState {
        ScheduleState {
            stage: stage.to_string(),
            plan_path: plan_path.display().to_string(),
            compiled_at: clock.now_rfc3339(),
            max_parallel: self.max_parallel,
            width: self.width,
            waves: self.waves.clone(),
            serializations: self.serializations.clone(),
            warnings: self.warnings.clone(),
            tasks: self
                .tasks
                .iter()
                .map(|t| TaskState {
                    id: t.task.id.clone(),
                    title: t.task.title.clone(),
                    kind: t.task.kind.clone(),
                    wave: t.wave,
                    deps: t.deps.clone(),
                    mutex_after: t.mutex_after.clone(),
                    files: t.task.files.clone(),
                    status: TaskStatus::Pending,
                    tokens_used: None,
                })
                .collect(),
        }
    }
}

/// Compile a plan into a schedule.
///
/// `kinds` filters which tasks this stage claims (empty = all). `max_parallel`
/// is the stage's concurrency cap — it never changes the graph, only how many
/// ready tasks the emitted script lets run at once.
pub fn schedule(plan: &Plan, max_parallel: u32, kinds: &[String]) -> anyhow::Result<Schedule> {
    let claimed: Vec<&Task> = plan
        .tasks
        .iter()
        .filter(|t| kinds.is_empty() || kinds.iter().any(|k| k == &t.kind))
        .collect();
    if claimed.is_empty() {
        anyhow::bail!(
            "no plan task matches kinds {:?} (plan has {} task(s): {})",
            kinds,
            plan.tasks.len(),
            plan.tasks
                .iter()
                .map(|t| format!("{}:{}", t.id, t.kind))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let n = claimed.len();
    let idx: BTreeMap<&str, usize> = claimed
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();

    let mut warnings: Vec<String> = Vec::new();

    // Declared edges — the planner's semantic `needs` graph, and *only* that.
    // A `needs` pointing at a task this stage does not claim (e.g. a `tests`
    // task depending on a `code` task claimed by another stage) cannot be
    // enforced here — surface it instead of dropping it silently.
    let mut declared_edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n]; // successors of i
    for (j, t) in claimed.iter().enumerate() {
        for d in &t.needs {
            match idx.get(d.as_str()) {
                Some(&i) => {
                    declared_edges[i].insert(j);
                }
                None => warnings.push(format!(
                    "task `{}` needs `{d}`, which this stage does not claim (different `kind`) — that ordering is enforced by the stage dependency, not inside this stage",
                    t.id
                )),
            }
        }
    }

    // Reachability closure over the declared edges only.
    let mut reach = closure(&declared_edges, n);

    // Write sets, normalized for comparison. A file the plan declared shared is
    // no task's to own: drop it from the write set (and say so) so the merge
    // step stays the single writer instead of every claimant being serialized.
    let shared: BTreeSet<String> = plan
        .shared_files
        .iter()
        .map(|f| normalize_plan_path(f))
        .collect();
    let mut writes: Vec<BTreeMap<String, String>> = Vec::with_capacity(n); // norm → display
    for t in claimed.iter() {
        let mut m = BTreeMap::new();
        for f in &t.files {
            let k = normalize_plan_path(f);
            if k.is_empty() {
                continue;
            }
            if shared.contains(&k) {
                warnings.push(format!(
                    "task `{}` claims `{f}`, which the plan also lists in `shared_files` — left to the merge step, not owned by this task",
                    t.id
                ));
                continue;
            }
            m.insert(k, f.clone());
        }
        if m.is_empty() {
            warnings.push(format!(
                "task `{}` declares no owned file — nothing anchors it, so it cannot be serialized against a conflicting task",
                t.id
            ));
        }
        writes.push(m);
    }

    // Conflict serialization: any two tasks that could run concurrently and
    // write the same file are ordered in plan order. Checking `reach` first
    // means an already-ordered pair costs nothing, and orienting the new edge
    // only when neither direction is reachable keeps the graph acyclic.
    //
    // `mutex_edges` records *only* these inserted orderings, kept apart from
    // `declared_edges` (C9, Lot 3 Capitalisation) — see `ScheduledTask`'s doc
    // comment for why the split matters downstream. `effective_edges` is the
    // union of both: it is what the scheduler itself (indeg / waves / topo
    // order below) is computed over, so the *execution order* this function
    // produces is byte-for-byte identical to before the split. Only the two
    // vectors published on `ScheduledTask` (`deps`, `mutex_after`) differ.
    let mut mutex_edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut effective_edges: Vec<BTreeSet<usize>> = declared_edges.clone();
    let mut serializations: Vec<Serialization> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            if reach[i][j] || reach[j][i] {
                continue;
            }
            let shared_files: Vec<String> = writes[i]
                .iter()
                .filter(|(k, _)| writes[j].contains_key(*k))
                .map(|(_, disp)| disp.clone())
                .collect();
            if shared_files.is_empty() {
                continue;
            }
            effective_edges[i].insert(j);
            mutex_edges[i].insert(j);
            link(&mut reach, i, j, n);
            serializations.push(Serialization {
                before: claimed[i].id.clone(),
                after: claimed[j].id.clone(),
                files: shared_files,
            });
        }
    }

    // Published `deps`: declared semantics only — a real `needs` edge, whose
    // failure must propagate.
    let mut deps: Vec<Vec<String>> = vec![Vec::new(); n];
    for i in 0..n {
        for &j in &declared_edges[i] {
            deps[j].push(claimed[i].id.clone());
        }
    }
    // Published `mutex_after`: file-lock orderings only — waited on, but a
    // failure here must never cancel the successor (see `runTask`).
    let mut mutex_after: Vec<Vec<String>> = vec![Vec::new(); n];
    for i in 0..n {
        for &j in &mutex_edges[i] {
            mutex_after[j].push(claimed[i].id.clone());
        }
    }

    // Effective deps per task (declared ∪ mutex) — internal only. `indeg`,
    // `level` and the topological order below MUST be derived from this, not
    // from the published `deps`, or restricting `deps` to declared-only would
    // silently reorder execution and reopen the very race this scheduler
    // exists to prevent.
    let mut effective_deps: Vec<Vec<String>> = vec![Vec::new(); n];
    for i in 0..n {
        for &j in &effective_edges[i] {
            effective_deps[j].push(claimed[i].id.clone());
        }
    }

    // Topological order, plan order as the tie-break so emission is stable.
    let mut indeg: Vec<usize> = effective_deps.iter().map(|d| d.len()).collect();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut ready: Vec<usize> = (0..n).filter(|i| indeg[*i] == 0).collect();
    while !ready.is_empty() {
        ready.sort_unstable();
        let i = ready.remove(0);
        order.push(i);
        for &j in &effective_edges[i] {
            indeg[j] -= 1;
            if indeg[j] == 0 {
                ready.push(j);
            }
        }
    }
    if order.len() != n {
        anyhow::bail!("internal: task graph not acyclic after conflict serialization");
    }

    // Waves: level = 1 + max(level(effective deps)).
    let mut level = vec![0usize; n];
    for &i in &order {
        let l = effective_deps[i]
            .iter()
            .filter_map(|d| idx.get(d.as_str()).map(|&k| level[k] + 1))
            .max()
            .unwrap_or(0);
        level[i] = l;
    }
    let depth = level.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut waves: Vec<Vec<String>> = vec![Vec::new(); depth];
    for i in 0..n {
        waves[level[i]].push(claimed[i].id.clone());
    }
    let width = waves.iter().map(|w| w.len()).max().unwrap_or(0);

    let tasks: Vec<ScheduledTask> = order
        .iter()
        .map(|&i| ScheduledTask {
            task: claimed[i].clone(),
            deps: deps[i].clone(),
            mutex_after: mutex_after[i].clone(),
            wave: level[i],
        })
        .collect();

    Ok(Schedule {
        tasks,
        waves,
        serializations,
        warnings,
        max_parallel: max_parallel.max(1),
        width,
        shared_files: plan.shared_files.clone(),
    })
}

/// Transitive closure of `edges` (Floyd–Warshall on booleans; task counts are
/// small — tens, not thousands).
fn closure(edges: &[BTreeSet<usize>], n: usize) -> Vec<Vec<bool>> {
    let mut r = vec![vec![false; n]; n];
    for (i, succ) in edges.iter().enumerate() {
        for &j in succ {
            r[i][j] = true;
        }
    }
    for k in 0..n {
        let row_k = r[k].clone();
        for row_i in r.iter_mut() {
            if row_i[k] {
                for (j, reachable) in row_k.iter().enumerate() {
                    if *reachable {
                        row_i[j] = true;
                    }
                }
            }
        }
    }
    r
}

/// Record `i → j` in the closure: every ancestor of `i` (and `i`) now reaches
/// every descendant of `j` (and `j`).
fn link(reach: &mut [Vec<bool>], i: usize, j: usize, n: usize) {
    let ancestors: Vec<usize> = (0..n).filter(|&a| a == i || reach[a][i]).collect();
    let descendants: Vec<usize> = (0..n).filter(|&b| b == j || reach[j][b]).collect();
    for &a in &ancestors {
        for &b in &descendants {
            reach[a][b] = true;
        }
    }
}

/// Normalizes a repo-relative path for comparison: `\` to `/`, drop `./`, trim slashes.
/// Lowercases the result, since Windows and macOS paths are case-insensitive.
pub fn normalize_plan_path(path: &str) -> String {
    let t = path.trim().replace('\\', "/");
    let mut t = t.as_str();
    while let Some(rest) = t.strip_prefix("./") {
        t = rest;
    }
    t.trim_matches('/').to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Persisted slice (run-state / plan-status)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleState {
    pub stage: String,
    pub plan_path: String,
    pub compiled_at: String,
    pub max_parallel: u32,
    #[serde(default)]
    pub width: usize,
    pub waves: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serializations: Vec<Serialization>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub tasks: Vec<TaskState>,
}

/// Outcome of a plan task; a value the runtime writes that is not listed round-trips in `Other`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TaskStatus {
    #[default]
    Pending,
    Success,
    Failed,
    Skipped,
    Cancelled,
    Other(String),
}

impl TaskStatus {
    /// Wire string of this status.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Other(raw) => raw,
        }
    }
}

impl From<&str> for TaskStatus {
    fn from(raw: &str) -> Self {
        match raw {
            "pending" => Self::Pending,
            "success" => Self::Success,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            "cancelled" => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TaskStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from(raw.as_str()))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskState {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub wave: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    /// Tasks this one was serialized after purely because of a shared write
    /// set (C9, Lot 3 Capitalisation) — never a semantic dependency. Absent
    /// on a run-state written before this field existed, hence the default
    /// so an old `run-state.json` still deserializes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutex_after: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Outcome folded in by `run::reconcile`.
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
}

#[cfg(test)]
#[path = "../tests/unit/plan_tests.rs"]
mod status_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn load_refuses_oversized_plan() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("plan.json");
        std::fs::write(&f, vec![b' '; (super::MAX_PLAN_BYTES + 1) as usize]).unwrap();
        let err = super::load(&f).unwrap_err().to_string();
        assert!(err.contains("1048576"), "{err}");
    }

    #[test]
    fn load_small_plan_is_not_refused_for_size() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("plan.json");
        std::fs::write(&f, "{}").unwrap();
        if let Err(e) = super::load(&f) {
            assert!(!e.to_string().contains("byte limit"), "{e}");
        }
    }

    use super::*;

    fn plan_json(tasks: &str) -> String {
        format!(r#"{{"schema":"aidp-plan.v1","summary":"s","tasks":[{tasks}]}}"#)
    }

    fn task(id: &str, kind: &str, needs: &[&str], files: &[&str]) -> String {
        format!(
            r#"{{"id":"{id}","title":"t {id}","kind":"{kind}","prompt":"do {id}","needs":{},"files":{}}}"#,
            serde_json::to_string(needs).unwrap(),
            serde_json::to_string(files).unwrap()
        )
    }

    #[test]
    fn parses_and_schedules_a_linear_chain() {
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "tests", &["T1"], &["tests/a.tests.cs"])
        ));
        let plan = parse(&text).unwrap();
        let s = schedule(&plan, 4, &[]).unwrap();
        assert_eq!(s.tasks.len(), 2);
        assert_eq!(
            s.waves,
            vec![vec!["T1".to_string()], vec!["T2".to_string()]]
        );
        assert_eq!(s.task("T2").unwrap().deps, vec!["T1".to_string()]);
        assert!(s.serializations.is_empty());
        assert_eq!(s.width, 1);
        assert_eq!(s.effective_parallel(), 1);
    }

    #[test]
    fn independent_tasks_share_a_wave() {
        let text = plan_json(&format!(
            "{},{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "code", &[], &["src/b.cs"]),
            task("T3", "code", &[], &["src/c.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        assert_eq!(s.waves.len(), 1, "no deps → one wave: {:?}", s.waves);
        assert_eq!(s.width, 3);
        assert_eq!(s.effective_parallel(), 3);
        // A cap below the width bounds concurrency without changing the graph.
        let s2 = schedule(&parse(&text).unwrap(), 2, &[]).unwrap();
        assert_eq!(s2.waves.len(), 1);
        assert_eq!(s2.effective_parallel(), 2);
    }

    // The whole point of `files`: two concurrent writers of one file are ordered
    // by aidp, and the decision is recorded rather than silently applied.
    #[test]
    fn overlapping_write_sets_are_serialized_and_recorded() {
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["src/Shared.cs", "src/a.cs"]),
            task("T2", "code", &[], &["src/shared.cs", "src/b.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        // C9 (Lot 3, Capitalisation): this ordering was inserted purely because
        // of the shared write set — neither task declared `needs` — so it must
        // land on `mutex_after`, never on the semantic `deps` a `blocked`/
        // `failed` upstream propagates through.
        assert!(
            s.task("T2").unwrap().deps.is_empty(),
            "no `needs` declared: deps must stay empty"
        );
        assert_eq!(
            s.task("T2").unwrap().mutex_after,
            vec!["T1".to_string()],
            "T2 must still wait for T1"
        );
        assert_eq!(s.waves.len(), 2, "the conflict costs one wave");
        assert_eq!(
            s.serializations,
            vec![Serialization {
                before: "T1".into(),
                after: "T2".into(),
                // Case-insensitive match, reported with the earlier task's spelling.
                files: vec!["src/Shared.cs".into()],
            }]
        );
    }

    // C9 (Lot 3, Capitalisation): the split between `deps` (declared) and
    // `mutex_after` (file-lock only) must not change what the scheduler
    // actually does — waves and topological order are byte-for-byte identical
    // to what this same two-task, no-`needs` scenario produced before the
    // split (see `overlapping_write_sets_are_serialized_and_recorded` above).
    // That identity, not just the new field's value, is the non-regression
    // guarantee this chantier exists to prove.
    #[test]
    fn mutex_edges_are_not_declared_dependencies() {
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["a.cs"]),
            task("T2", "code", &[], &["a.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        assert!(
            s.task("T1").unwrap().deps.is_empty(),
            "no declared needs → deps must be empty"
        );
        assert!(
            s.task("T2").unwrap().deps.is_empty(),
            "a file-lock ordering must never leak into `deps`: {:?}",
            s.task("T2").unwrap().deps
        );
        assert!(
            s.task("T1").unwrap().mutex_after.is_empty(),
            "T1 has no predecessor to wait for"
        );
        assert_eq!(
            s.task("T2").unwrap().mutex_after,
            vec!["T1".to_string()],
            "the write conflict must still be recorded, just on the right field"
        );
        // Non-regression: wave placement...
        assert_eq!(
            s.waves,
            vec![vec!["T1".to_string()], vec!["T2".to_string()]],
            "the split must not change wave placement"
        );
        // ...and the topological order the emitter walks tasks in.
        assert_eq!(
            s.tasks
                .iter()
                .map(|t| t.task.id.clone())
                .collect::<Vec<_>>(),
            vec!["T1".to_string(), "T2".to_string()],
            "the split must not change topological order"
        );
    }

    // An already-ordered pair pays nothing, and a conflict edge is never
    // inserted against an existing path (that would create a cycle).
    #[test]
    fn existing_order_is_respected_both_ways() {
        // T2 needs T1 and they share a file → no extra edge, no serialization.
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "code", &["T1"], &["src/a.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        assert!(
            s.serializations.is_empty(),
            "declared order already covers it"
        );
        assert_eq!(s.task("T2").unwrap().deps, vec!["T1".to_string()]);

        // Backwards declared order (T1 needs T2) + a shared file: the edge must
        // stay T2 → T1, never both.
        let back = plan_json(&format!(
            "{},{}",
            task("T1", "code", &["T2"], &["src/a.cs"]),
            task("T2", "code", &[], &["src/a.cs"])
        ));
        let s2 = schedule(&parse(&back).unwrap(), 4, &[]).unwrap();
        assert!(s2.serializations.is_empty());
        assert_eq!(s2.task("T1").unwrap().deps, vec!["T2".to_string()]);
        assert_eq!(s2.tasks[0].task.id, "T2", "topo order must put T2 first");
    }

    // Transitive orderings count as ordered: T1 → T2 → T3 means T1/T3 need no
    // conflict edge even when they write the same file.
    #[test]
    fn transitive_order_counts_as_ordered() {
        let text = plan_json(&format!(
            "{},{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "code", &["T1"], &["src/b.cs"]),
            task("T3", "code", &["T2"], &["src/a.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        assert!(
            s.serializations.is_empty(),
            "T1..T3 already ordered: {:?}",
            s.serializations
        );
        assert_eq!(s.waves.len(), 3);
    }

    // shared_files belong to the merge step: claiming one does not serialize
    // every claimant, it just warns and drops the claim.
    #[test]
    fn shared_files_are_not_owned_by_tasks() {
        let text = format!(
            r#"{{"schema":"aidp-plan.v1","shared_files":["src/Program.cs"],"tasks":[{},{}]}}"#,
            task("T1", "code", &[], &["src/Program.cs", "src/a.cs"]),
            task("T2", "code", &[], &["src/Program.cs", "src/b.cs"])
        );
        let s = schedule(&parse(&text).unwrap(), 4, &[]).unwrap();
        assert!(
            s.serializations.is_empty(),
            "a shared file must not serialize tasks"
        );
        assert_eq!(s.waves.len(), 1, "both tasks stay concurrent");
        assert_eq!(
            s.warnings.len(),
            2,
            "both claims are reported: {:?}",
            s.warnings
        );
        assert!(s.warnings[0].contains("shared_files"));
    }

    #[test]
    fn kinds_filter_claims_a_subset() {
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "tests", &["T1"], &["tests/a.cs"])
        ));
        let plan = parse(&text).unwrap();
        let code = schedule(&plan, 4, &["code".to_string()]).unwrap();
        assert_eq!(code.tasks.len(), 1);
        let tests = schedule(&plan, 4, &["tests".to_string()]).unwrap();
        assert_eq!(tests.tasks.len(), 1);
        // T2's cross-stage dep can't be enforced inside the tests stage — warned.
        assert!(
            tests.warnings.iter().any(|w| w.contains("does not claim")),
            "cross-kind dependency must be surfaced: {:?}",
            tests.warnings
        );
        // A filter that matches nothing is an error, never an empty stage.
        assert!(schedule(&plan, 4, &["docs".to_string()]).is_err());
    }

    #[test]
    fn rejects_bad_plans_with_actionable_errors() {
        // Missing required field (`files`).
        let e = parse(r#"{"tasks":[{"id":"T1","title":"t","kind":"code","prompt":"p"}]}"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("files"), "must name the missing field: {e}");

        // Empty task list.
        assert!(parse(r#"{"tasks":[]}"#).is_err());

        // Duplicate ids.
        let dup = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["a"]),
            task("T1", "code", &[], &["b"])
        ));
        let e = parse(&dup).unwrap_err().to_string();
        assert!(e.contains("duplicate task id"), "got: {e}");

        // Unknown dependency.
        let unknown = plan_json(&task("T1", "code", &["TX"], &["a"]));
        let e = parse(&unknown).unwrap_err().to_string();
        assert!(e.contains("unknown task `TX`"), "got: {e}");

        // Cycle.
        let cyc = plan_json(&format!(
            "{},{}",
            task("T1", "code", &["T2"], &["a"]),
            task("T2", "code", &["T1"], &["b"])
        ));
        let e = parse(&cyc).unwrap_err().to_string();
        assert!(e.contains("cycle"), "got: {e}");

        // Wrong schema marker.
        let e = parse(r#"{"schema":"aidp-plan.v9","tasks":[]}"#)
            .unwrap_err()
            .to_string();
        assert!(!e.is_empty());

        // Not JSON at all.
        assert!(parse("# Plan\n\n1. do the thing").is_err());
    }

    #[test]
    fn to_state_snapshots_the_schedule() {
        let text = plan_json(&format!(
            "{},{}",
            task("T1", "code", &[], &["src/a.cs"]),
            task("T2", "tests", &["T1"], &["tests/a.cs"])
        ));
        let s = schedule(&parse(&text).unwrap(), 3, &[]).unwrap();
        let st = s.to_state("implement", Path::new("/ws/plan/plan.json"));
        assert_eq!(st.stage, "implement");
        assert_eq!(st.max_parallel, 3);
        assert_eq!(st.tasks.len(), 2);
        assert!(st.tasks.iter().all(|t| t.status == TaskStatus::Pending));
        assert_eq!(st.waves.len(), 2);
    }
}
