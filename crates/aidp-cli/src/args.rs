//! The whole clap declaration: `Cli`, `Cmd` and every CLI-owned `…Op` enum.

use crate::models::FormatArg;
use clap::{ArgAction, ArgGroup, Parser, Subcommand};
use std::path::PathBuf;

pub mod flow;
pub mod gateway;
pub mod gh;
pub mod memory;
pub mod project;
pub mod tickets;

/// Exit-code table appended to `aidp --help`, pointing at the full reference.
const EXIT_CODES_AFTER_HELP: &str = "Exit codes: 0 success, 1 internal, 2 hook wake, 3 not found, 4 policy refusal, 5 upstream failure, 6 negative verdict, 7 usage. See the \"Exit codes\" section of README.md.";

/// Lower bound of `git outcomes --overlap`.
const OVERLAP_MIN: f32 = 0.0;
/// Upper bound of `git outcomes --overlap`.
const OVERLAP_MAX: f32 = 1.0;
/// Smallest accepted `git outcomes --window`.
const WINDOW_MIN: usize = 1;
/// Name of the `validate` argument group requiring a pipeline or `--all`.
const VALIDATE_TARGET_GROUP: &str = "validate_target";
/// Separator between the key and the value of an `--input` pair.
const KEY_VALUE_SEPARATOR: char = '=';

/// Parses `--overlap` as a fraction in `0..=1`, naming the range on failure.
pub fn parse_overlap(raw: &str) -> Result<f32, String> {
    let value: f32 = raw.parse().map_err(|_| {
        format!("`{raw}` is not a number; expected a value in {OVERLAP_MIN}..={OVERLAP_MAX}")
    })?;
    if (OVERLAP_MIN..=OVERLAP_MAX).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "`{raw}` is out of range; expected a value in {OVERLAP_MIN}..={OVERLAP_MAX}"
        ))
    }
}

/// Parses `--window` as a whole number of at least 1.
pub fn parse_window(raw: &str) -> Result<usize, String> {
    let value: usize = raw.parse().map_err(|_| {
        format!("`{raw}` is not a whole number; expected a value of at least {WINDOW_MIN}")
    })?;
    if value >= WINDOW_MIN {
        Ok(value)
    } else {
        Err(format!(
            "`{raw}` is too small; expected a value of at least {WINDOW_MIN}"
        ))
    }
}

/// Parses one `key=value` pair, refusing an empty key. The value may be empty
/// and may itself contain `=`.
pub fn parse_key_value(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once(KEY_VALUE_SEPARATOR)
        .ok_or_else(|| format!("`{raw}` must be key=value"))?;
    if key.trim().is_empty() {
        return Err(format!("`{raw}` has an empty key; expected key=value"));
    }
    Ok((key.to_string(), value.to_string()))
}

#[derive(Parser)]
#[command(
    name = "aidp",
    version,
    about = "ai-dev-pipeline v7 helper: compiles pipelines into Workflow scripts and manages runs, the vault, tickets and flows",
    after_help = EXIT_CODES_AFTER_HELP
)]
pub struct Cli {
    /// Workspace to act on instead of the current directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub workspace_root: Option<PathBuf>,
    /// Obsidian vault to use instead of `AIDP_VAULT`, `~/.aidp/config.json` or auto-detection.
    #[arg(long, global = true, value_name = "PATH")]
    pub vault: Option<PathBuf>,
    /// Ignore any vault and keep artefacts in the repo (`.aidp/pipelines/`).
    #[arg(long, global = true, conflicts_with = "vault")]
    pub no_vault: bool,
    /// Leave the compile-time `## Knowledge` block out of the stage prompts.
    #[arg(long, global = true)]
    pub no_knowledge: bool,
    /// Output format. Each command keeps its own default when omitted.
    #[arg(long, global = true, value_enum, value_name = "FORMAT")]
    pub format: Option<FormatArg>,
    /// More diagnostics on stderr; repeat for more (`-vv`).
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
    /// The command to run.
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Validate a pipeline YAML (schema + composition: refs, cycles, inputs).
    #[command(group(ArgGroup::new(VALIDATE_TARGET_GROUP).required(true).args(["pipeline", "all"])))]
    Validate {
        /// Pipeline id or path to validate.
        #[arg(value_name = "ID_OR_PATH")]
        pipeline: Option<String>,
        /// Validate every discoverable pipeline.
        #[arg(long, conflicts_with = "inputs")]
        all: bool,
        /// Cold-validate a JSON inputs object against the pipeline's `inputs:`
        /// contract (required/values/default), without starting a run.
        #[arg(long)]
        inputs: Option<String>,
    },
    /// Discover pipelines: `list` them, `show` one, or `expand` its compositions.
    Pipeline {
        /// The pipeline operation.
        #[command(subcommand)]
        op: PipelineOp,
    },
    /// Run lifecycle: init, start, state, summary, reconcile, resume anchors.
    Run {
        /// The run operation.
        #[command(subcommand)]
        op: RunOp,
    },
    /// Scope checking.
    Scope {
        /// The scope operation.
        #[command(subcommand)]
        op: ScopeOp,
    },
    /// Discover registered agents.
    Agents {
        /// The agents operation.
        #[command(subcommand)]
        op: AgentsOp,
    },
    /// Resolve a skill name.
    Skills {
        /// The skills operation.
        #[command(subcommand)]
        op: SkillsOp,
    },
    /// Check the stage diff against the rules of the loaded skills.
    SkillRules {
        /// The skill-rules operation.
        #[command(subcommand)]
        op: SkillRulesOp,
    },
    /// Topo-sort a pipeline.
    Topo {
        /// Pipeline id or path to sort.
        #[arg(long)]
        pipeline: String,
    },
    /// Render run progress (markdown).
    Progress {
        /// Id of the run to render.
        #[arg(long)]
        run: String,
    },
    /// Gate lifecycle: approve/abort/pending. Persists the decision then hands
    /// back exactly what the driver needs to act on it.
    Gate {
        /// The gate operation.
        #[command(subcommand)]
        op: GateOp,
    },
    /// GitHub verbs (pull requests, checks, review threads, workflow runs) through the `gh` CLI.
    Gh {
        /// The GitHub operation.
        #[command(subcommand)]
        op: gh::Op,
    },
    /// Facts about a project directory (stack detection).
    Project {
        /// The project operation.
        #[command(subcommand)]
        op: project::ProjectOp,
    },
    /// Run the local mock HTTP server.
    Mock {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8765")]
        addr: String,
        /// Directory holding the mock fixtures.
        #[arg(long, default_value = "fixtures/mocks")]
        data: PathBuf,
    },
    /// v7 engine: compile a run into a Claude Code Workflow script.
    Workflow {
        /// The workflow operation.
        #[command(subcommand)]
        op: WorkflowOp,
    },
    /// v7: the Obsidian vault that holds everything pipelines produce.
    Vault {
        /// The vault operation.
        #[command(subcommand)]
        op: VaultOp,
    },
    /// v7: the per-project knowledge base — its two layers, and the
    /// create-or-update decision for a feature note.
    Knowledge {
        /// The knowledge operation.
        #[command(subcommand)]
        op: KnowledgeOp,
    },
    /// The resident control plane: headless claude sessions, automations
    /// (cron / webhook / event), HTTP API + SSE for the viewer.
    Gateway {
        /// The gateway operation.
        #[command(subcommand)]
        op: gateway::Op,
    },
    /// The local ticket backlog under `<vault>/TODO/`, only ever written by
    /// `aidp`: an agent must never hand-edit a ticket's status or frontmatter.
    Ticket {
        /// The ticket operation.
        #[command(subcommand)]
        op: tickets::Op,
    },
    /// Ticket flows: validate a definition, start and drive a run through the daemon.
    Flow {
        /// The flow operation.
        #[command(subcommand)]
        op: flow::Op,
    },
    /// RAG over the project corpus: index and search BIBLE, docs, C# code, runs and tickets.
    Rag {
        /// The rag operation.
        #[command(subcommand)]
        op: memory::RagOp,
    },
    /// Project memory: lessons and decisions injected into pipeline stages.
    Memory {
        /// The memory operation.
        #[command(subcommand)]
        op: memory::MemoryOp,
    },
    /// Self-check: binary, templates, agents, hooks, schemas, vault.
    Doctor,
    /// List runs found in the vault, and under `.aidp/pipelines/` in the current workspace.
    Status,
    /// Setup helpers (cross-platform JSON merge for hooks, etc.).
    Setup {
        /// The setup operation.
        #[command(subcommand)]
        op: SetupOp,
    },
    /// Cost measurement: per-run breakdown, JSONL ledger, counterfactual and
    /// anomaly detection. Never changes a `model:` or makes a routing decision.
    Cost {
        /// The cost operation.
        #[command(subcommand)]
        op: CostOp,
    },
    /// Git history as a differed signal: commit outcomes (revert, hotfix,
    /// success) derived from history, never from the reply envelope.
    Git {
        /// The git operation.
        #[command(subcommand)]
        op: GitOp,
    },
    /// Deterministic findings.json merge: fold N axis reviews into
    /// one canonical verdict mechanically instead of via a synthesis model.
    Findings {
        /// The findings operation.
        #[command(subcommand)]
        op: FindingsOp,
    },
}

#[derive(Subcommand)]
pub enum CostOp {
    /// Cost of one run by stage and model plus the total, always paired with
    /// its provenance (`jsonl` | `budget` | `unavailable`), never a silent 0.
    Show {
        /// Id of the run to price.
        #[arg(long)]
        run: String,
    },
    /// Append one numeric JSONL line per sub-agent to the durable routing ledger
    /// (default `<vault>/Knowledge/<repo>/_routing/decisions.jsonl`); write failures are swallowed.
    Ledger {
        /// Id of the run to record.
        #[arg(long)]
        run: String,
        /// Ledger file to append to instead of the default.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Replay this run's observed token volumes at an alternate tier's prices.
    /// Cost only: says nothing about the alternate model's quality.
    Counterfactual {
        /// Id of the run to replay.
        #[arg(long)]
        run: String,
        /// Tier to price the run at, or `all`.
        #[arg(long, default_value = "all")]
        baseline: String,
    },
    /// Token cost of one ticket from the daemon (flows, steps, stages, models, window share);
    /// `--format footer` prints the one-line report footer instead.
    Ticket {
        /// Ticket key (the `origin.ref` of its flow sessions).
        key: String,
        /// Daemon URL; defaults to `AIDP_GATEWAY_URL` or the local daemon.
        #[arg(long)]
        url: Option<String>,
    },
    /// Flag cost outliers via the Iglewicz-Hoaglin modified z-score (median + MAD).
    /// Exits 6 (negative verdict) if an outlier is found.
    Anomaly {
        /// Repository whose runs are compared.
        #[arg(long)]
        repo: String,
        /// Only compare runs of this pipeline template.
        #[arg(long)]
        template: Option<String>,
        /// Only compare the last N runs.
        #[arg(long)]
        last: Option<usize>,
    },
}

#[derive(Subcommand)]
pub enum VaultOp {
    /// Show which vault resolves for this workspace, and how it was found.
    Status,
    /// Create the vault skeleton and record it in `~/.aidp/config.json`.
    /// The path is `--vault`, else whatever `status` resolves.
    Init {
        /// Do not write `~/.aidp/config.json`.
        #[arg(long)]
        no_config: bool,
    },
    /// Regenerate every generated note (`_*.md`) for every run in the vault.
    /// Use after upgrading `aidp`, or if notes were edited/deleted by hand.
    Reindex,
}

/// `aidp knowledge ...`: read side of `<vault>/Knowledge/<repo>/`, mainly for
/// `resolve`, which computes the create-or-update decision instead of asking an agent.
#[derive(Subcommand)]
pub enum KnowledgeOp {
    /// List the base: transverse notes, then one line per feature note.
    List {
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Decide whether a feature has a note (`update`) or needs one (`create`, with
    /// the path to write); `ambiguous` means several notes are close: read them.
    Resolve {
        /// The feature this run is about. Repeatable, and Jira keys count — a
        /// note's `tickets:` is matched too. `--name "A" --name PIXEL-1234`.
        #[arg(long = "name", required = true)]
        names: Vec<String>,
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Regenerate the knowledge index notes (`_<repo>.md`, `Features/_Features.md`,
    /// `_Knowledge.md`) for every documented project. Run it after hand-editing.
    Index,
    /// Print the resolved paths of both layers.
    Path {
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Base-wide health: feature/root-note counts, size, oversized notes,
    /// colliding handles, and the adoption count for `files:`.
    Stats {
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Mark a feature note superseded by upserting `status`, `valid_until`,
    /// `superseded_by` and optional `supersede_reason` in its frontmatter; never deletes it.
    Supersede {
        /// Slug of the note being superseded.
        #[arg(long)]
        slug: String,
        /// Slug of the note that replaces it.
        #[arg(long = "by")]
        by: String,
        /// Why the note is superseded.
        #[arg(long)]
        reason: Option<String>,
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Date every feature note's `files:` against git history, cache the result in
    /// `<knowledge_dir>/.aidp-stale.json` and print the report; never fails.
    Stale {
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
    /// Read-only report on forks, dangling `superseded_by` pointers and notes whose
    /// `files:` are all gone. Always exits 0.
    Gc {
        /// Where the base is located.
        #[command(flatten)]
        loc: KnowledgeLoc,
    },
}

/// How the knowledge base is located, in precedence order.
/// The vault and workspace come from the global `--vault` and `--workspace-root`.
#[derive(clap::Args, Clone)]
pub struct KnowledgeLoc {
    /// The base directory itself. Wins over everything else.
    #[arg(long)]
    pub knowledge_dir: Option<PathBuf>,
    /// Take it from a run's state. Falls back to `$AIDP_RUN`, which every stage
    /// prompt exports — so inside a stage this needs no flag at all.
    #[arg(long)]
    pub run: Option<String>,
}

/// How the ticket vault is located, filled from the global `--vault` and
/// `--workspace-root` flags before a ticket command runs.
#[derive(Clone)]
pub struct TicketLoc {
    /// The global `--vault` value.
    pub vault: Option<PathBuf>,
    /// The global `--workspace-root` value.
    pub workspace_root: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum SetupOp {
    /// Merge hooks from an example file into target settings.json. Idempotent.
    MergeHooks {
        /// Example settings file holding the hooks to add.
        #[arg(long)]
        example: PathBuf,
        /// Settings file to merge the hooks into.
        #[arg(long)]
        target: PathBuf,
        /// Skip writing — print what would change.
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate the restricted agent personas (`aidp-<role>-ro.md`) that
    /// `settings.tool_enforcement: strict` needs, into the Claude Code agent registry.
    Agents {
        /// Registry directory to write into; defaults to `~/.claude/agents/`.
        #[arg(long)]
        dest: Option<PathBuf>,
        /// Overwrite an existing registry file; without it an existing target
        /// refuses the whole command.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum PipelineOp {
    /// List discovered pipelines.
    List,
    /// Print one pipeline, resolved from an id or a path.
    Show {
        /// Pipeline id or path.
        id_or_path: String,
    },
    /// Print the flattened effective pipeline (composites inlined, namespaced).
    Expand {
        /// Pipeline id or path to expand.
        #[arg(long)]
        pipeline: String,
    },
}

#[derive(Subcommand)]
pub enum RunOp {
    /// Create a run: allocate its id and directory, write its state and print the id.
    Init {
        /// Pipeline id or path.
        #[arg(long)]
        pipeline: String,
        /// The pipeline target (ticket key, PR number, path, ...).
        #[arg(long)]
        target: Option<String>,
        /// JSON object of pipeline inputs.
        #[arg(long)]
        inputs: Option<String>,
    },
    /// `run init` + `workflow emit` in one call. Same flags as `init`, plus
    /// repeatable `--input k=v` (a repeated key accumulates into a JSON array).
    Start {
        /// Pipeline id or path.
        #[arg(long)]
        pipeline: String,
        /// The pipeline target (ticket key, PR number, path, ...).
        #[arg(long)]
        target: Option<String>,
        /// JSON object of pipeline inputs.
        #[arg(long)]
        inputs: Option<String>,
        /// Repeatable `key=value`. A key given more than once accumulates into
        /// a JSON array for that key. The key must not be empty.
        #[arg(long, value_parser = parse_key_value)]
        input: Vec<(String, String)>,
        /// Pre-approve a gate before the first `workflow emit`; repeatable, `*` means
        /// every gated stage. Falls back to `AIDP_AUTO_APPROVE_GATES`.
        #[arg(long = "approve-gate")]
        approve_gate: Vec<String>,
        /// Read the inputs JSON object from this file instead of `--inputs`,
        /// immune to shell quoting of accented text.
        #[arg(long, conflicts_with = "inputs")]
        inputs_file: Option<PathBuf>,
        /// Pin a stage to a model tier, `STAGE=TIER` (haiku|sonnet|opus); repeatable.
        /// Wins over the vault's `models:` policy and over the template.
        #[arg(long = "stage-model", value_name = "STAGE=TIER")]
        stage_model: Vec<String>,
    },
    /// Run a template natively: the daemon (or `--foreground`, in-process) drives every
    /// stage through agent sessions, with no Workflow tool involved.
    Exec {
        /// Pipeline template id or path.
        template: String,
        /// The pipeline target (ticket key, PR number, path, ...).
        #[arg(long)]
        target: Option<String>,
        /// Repeatable `key=value`; a key given more than once accumulates into a JSON array.
        #[arg(long, value_parser = parse_key_value)]
        input: Vec<(String, String)>,
        /// Read the inputs JSON object from this file.
        #[arg(long)]
        inputs_file: Option<PathBuf>,
        /// Agent provider id used by every stage without its own.
        #[arg(long)]
        provider: Option<String>,
        /// Cross this gate without asking; repeatable, `*` means every gate.
        #[arg(long = "approve-gate")]
        approve_gate: Vec<String>,
        /// Drive the run in this process and exit with its verdict instead of asking the daemon.
        #[arg(long)]
        foreground: bool,
        /// Daemon URL; defaults to `AIDP_GATEWAY_URL` or the local daemon.
        #[arg(long)]
        url: Option<String>,
    },
    /// Print a run's state (text by default; `--format json-compact` for JSON).
    State {
        /// Id of the run.
        #[arg(long)]
        run: String,
    },
    /// Print a human-readable summary of a run.
    Summary {
        /// Omit to summarise the run in flight (the `run-in-flight` marker), as the
        /// Stop hook does.
        #[arg(long)]
        run: Option<String>,
    },
    /// Fold a Workflow result file (`--from`) back into run-state. Exits 0 even
    /// with scope_violations; exits 4 (policy refusal) on a vault leak.
    Reconcile {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// A file path, or `-` to read the result JSON from stdin.
        #[arg(long)]
        from: String,
    },
    /// Write the typed execution plan of a run to `execution-plan.json` and print its path.
    PlanExec {
        /// Id of the run.
        #[arg(long)]
        run: String,
    },
    /// Persist resume state: Workflow runId, approved gates, `--pending-gate`,
    /// `--crossed-boundary`, or `--from-result`; explicit flags win over the file.
    RecordResume {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// The Workflow `runId` to resume from.
        #[arg(long)]
        workflow_run_id: Option<String>,
        /// A gate that was approved; repeatable.
        #[arg(long = "approved-gate")]
        approved_gate: Vec<String>,
        /// The gate awaiting a decision.
        #[arg(long = "pending-gate")]
        pending_gate: Option<String>,
        /// The composite boundary the last Workflow pass stopped at.
        #[arg(long = "crossed-boundary")]
        crossed_boundary: Option<String>,
        /// Rebuild the anchor from a saved `workflow-result.json`.
        #[arg(long = "from-result")]
        from_result: Option<PathBuf>,
    },
    /// Read a sub-agent's token usage from the Claude projects directory.
    StageTokens {
        /// Claude session id.
        #[arg(long)]
        session_id: String,
        /// Sub-agent id within the session.
        #[arg(long)]
        agent_id: String,
        /// Claude projects directory; the default location when omitted.
        #[arg(long)]
        claude_projects_dir: Option<PathBuf>,
    },
    /// Replay verify_outputs over a run's declared outputs: annotation only,
    /// exits 0. `--stage` restricts to one stage.
    Verify {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Only verify this stage.
        #[arg(long)]
        stage: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum GateOp {
    /// Record the approval, re-emit `workflow.js` (the only writer of that
    /// file), and print the exact object to hand the `Workflow` tool to resume.
    Approve {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Name of the gated stage being approved.
        #[arg(long)]
        gate: String,
        /// The Workflow `runId` to resume from.
        #[arg(long)]
        workflow_run_id: Option<String>,
    },
    /// Record that this gate was rejected — no re-emit, the run stays paused.
    Abort {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Name of the gated stage being rejected.
        #[arg(long)]
        gate: String,
    },
    /// Record that this gate is awaiting a decision, *before* asking the user,
    /// so a resume can re-propose the same gate after a lost transcript.
    Pending {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Name of the gated stage awaiting a decision.
        #[arg(long)]
        gate: String,
        /// The Workflow `runId` to resume from.
        #[arg(long)]
        workflow_run_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ScopeOp {
    /// Print the write scope a stage declares.
    Resolve {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Name of the stage.
        #[arg(long)]
        stage: String,
    },
    /// Check one path against a stage's declared scope.
    Check {
        /// Path to check.
        #[arg(long)]
        path: PathBuf,
        /// Id of the run; falls back to `$AIDP_RUN`.
        #[arg(long)]
        run: Option<String>,
        /// Name of the stage; falls back to `$AIDP_STAGE`.
        #[arg(long)]
        stage: Option<String>,
    },
    /// Verify git history against each successful stage's declared scope. Exits 6
    /// on scope violations, and 4 (checked first) on a vault leak.
    Verify {
        /// Id of the run.
        #[arg(long)]
        run: String,
    },
}

#[derive(Subcommand)]
pub enum GitOp {
    /// Commit-by-commit verdicts (success | hotfixed | reverted) over the
    /// last 200 commits, via `aidp_core::gitlog`.
    Outcomes {
        /// Only consider commits since this date/ref (passed to `git log --since`).
        #[arg(long)]
        since: Option<String>,
        /// Number of following commits examined for a revert or hotfix (at least 1).
        #[arg(long, default_value = "20", value_parser = parse_window)]
        window: usize,
        /// Share of touched files that must overlap for a follow-up to count, in 0..=1.
        #[arg(long, default_value = "0.5", value_parser = parse_overlap)]
        overlap: f32,
    },
}

#[derive(Subcommand)]
pub enum FindingsOp {
    /// Merge N axis findings.json files into one: severity_counts recomputed,
    /// checks merged by id (worst wins), findings deduped; a bad axis is degraded.
    Merge {
        /// One findings.json per axis; the label defaults to the file stem.
        /// Repeat per input; prefer --label since every axis is named `findings.json`.
        #[arg(long = "input")]
        inputs: Vec<PathBuf>,
        /// `axis=path` — an explicitly-labelled input, in addition to --input.
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Where to write the merged findings.json.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum AgentsOp {
    /// List the registered agents.
    List,
}

#[derive(Subcommand)]
pub enum SkillsOp {
    /// Resolve a skill name to its SKILL.md.
    Resolve {
        /// Skill name.
        name: String,
    },
    /// List the discoverable skills.
    List,
}

#[derive(Subcommand)]
pub enum SkillRulesOp {
    /// Scan the added lines of a diff for skill rule violations and write a findings.json.
    /// Exits 0 either way; violations show as `has_blocking_findings` in the file.
    Check {
        /// Base ref whose merge-base with HEAD the diff starts from.
        #[arg(long, default_value = "master")]
        base: String,
        /// Restrict the check to the rules of this skill; repeat per skill (default: all).
        #[arg(long)]
        skill: Vec<String>,
        /// Repository to diff (default: the current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Read a unified diff from this file instead of running git.
        #[arg(long)]
        diff_file: Option<PathBuf>,
        /// Where to write the findings.json.
        #[arg(long)]
        out: PathBuf,
        /// Run id recorded in the findings.json.
        #[arg(long)]
        run_id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum WorkflowOp {
    /// Compile the run into a Workflow JS script (`<run_dir>/workflow.js` or
    /// `--out`), printing its path. A `plan_tasks:` stage needs a second emit after planning.
    Emit {
        /// Id of the run.
        #[arg(long)]
        run: String,
        /// Where to write the script instead of `<run_dir>/workflow.js`.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Report plan-driven stages: whether `plan.json` is compiled and the
    /// scheduled task graph (waves, concurrency, inserted orderings).
    PlanStatus {
        /// Id of the run.
        #[arg(long)]
        run: String,
    },
}

#[cfg(test)]
#[path = "../tests/unit/args_tests.rs"]
mod args_parsing;
