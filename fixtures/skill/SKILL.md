---
name: ai-dev-pipeline-v7
description: Workflow-native pipeline orchestrator (dev, study, ticket-create, discovery, tech/functional doc, audit) whose output lands in an Obsidian vault instead of the repo's .aidp folder. The Rust binary `aidp` compiles the pipeline into a deterministic Claude Code **Workflow** script; you run it with the `Workflow` tool. Sub-agents return schema-validated output (no reply parsing), independent stages run in parallel, executors can use git-worktree isolation, and gates pause/resume natively. Every artefact is written to `<vault>/Pipelines/<repo>/<feature>/<run_id>/` with generated index notes (frontmatter properties + wikilinks) linking runs, stages, features and repos. Triggers on /run-pipeline-v7, "lance la pipeline v7", "run pipeline v7", "ai-dev-pipeline-v7", and on questions about the AIDP vault.
---

# ai-dev-pipeline v7 — Workflow-native engine, Obsidian-native output

v7 keeps v6's engine — **`aidp` is the single source of truth, the agent does not improvise**, and the
native **`Workflow`** runtime executes the run — and changes **where everything a pipeline produces
lives**: an Obsidian vault instead of the target repo's `.aidp/` folder.

- `aidp workflow emit` compiles the run into a deterministic **JavaScript Workflow script**.
- The `Workflow` tool executes it: each stage is `agent(prompt, { schema, model, isolation })`, so
  replies are **schema-validated** (no `reply.md` free-text parsing), independent stages run via
  `parallel()`, and executor stages can run in isolated **git worktrees**.
- Gates pause the run; you ask the user, then **resume** the same script.
- Every artefact lands in `<vault>/Pipelines/<repo>/<feature>/<run_id>/`, and `aidp` maintains a set of
  **generated index notes** (`_*.md`) linking runs, stages, features and repos together.

You are still a thin driver: you only run a few `aidp` commands, invoke `Workflow`, and relay gates.

## Prerequisites

`aidp` must be built (with the `workflow` subcommand) and on PATH. Run `/pipeline-setup-v7` if needed, then
`/pipeline-doctor-v7`. v7 is **self-contained**: the binary is built from this skill's own `rust/`
workspace, then installed by `/pipeline-setup-v7` into `~/.aidp/bin/aidp` (override: `AIDP_INSTALL_DIR`),
the folder on PATH. No dependency on ai-dev-pipeline-v4. After changing or pulling the Rust code, re-run
`/pipeline-setup-v7` to update the installed copy, then open a new session. A cargo build alone does not
update it, and `cargo clean` no longer breaks a running session.
`aidp doctor`'s `aidp_on_path` check fails (exit 6) when the first `aidp` on PATH is absent, a
`target/debug` or worktree build, a different binary from the running one, or older than the sources
(slugs `absent`, `target_build`, `mismatch`, `older_than_running`, `older_than_sources`); the fix is
`/pipeline-setup-v7`. The gateway reports the same verdict as the `aidpBinary` block of `GET /api/health`.

Agent providers: `~/.aidp/config.json` may hold a `providers` key (default provider, per-provider
`program`/`args`, tier-to-model map). A stage picks its provider by: stage `provider:`, then template
`settings.providers.<role>`, then launch override, then `providers.default`, then `claude`; an unknown
provider is refused by name. `aidp doctor` has a `providers` check (binary present, `--version`) and
`GET /api/health` lists `providers[]`. See `docs/providers.md`.

**LOC-32:** `aidp-core` exposes four small ports so the rest of the workspace stops calling the system
directly: `clock.rs` (`Clock`/`SystemClock`/`FixedClock`), `id.rs` (`IdGen`/`SystemIdGen`/`FixedIdGen`),
`git.rs` (`Git`/`GitCli`/`RecordingGit`), and `process_runner.rs`
(`ProcessRunner`/`OsProcessRunner`/`ScriptedProcessRunner`). Pricing moved the same way: `cost.rs` reads
`AIDP_MODEL_PRICES` through a `PriceTable` built by `PriceTable::from_env_or_embedded`, replacing the
old process-global `OnceCell` — `AIDP_MODEL_PRICES` itself is unchanged, only how the grid is read.
`aidp-gateway` now depends on `aidp-core`, and its `claude` spawning lives in
`aidp-gateway/src/claude_runner.rs` behind `SessionManager::load_with_runner`. Every `aidp-integrations`
`gh`/`git` spawn resolves its program through `resolve_program`, so a `gh.cmd` shim on `PATHEXT` is
honored by every `aidp gh` verb, on Windows included.
`aidp gh pr-status [<pr>] [--repo owner/name]` reads a PR's merge gate (facts plus verdict); it is read-only and exits 0 whatever the verdict.
`aidp gh pr-comments <pr> [--repo owner/name]` splits a PR's review comments into points: a comment splits at its `##` to `####` headings, else at its top-level list items; a code block is never split. A point id is `<commentId>#<i>`. The `bot` flag is set for an author ending in `[bot]` or listed in `ci.bot_authors`. The comment carrying the marker `<!-- aidp:pr-comments-summary -->` is excluded.
`aidp gh pr-review-threads --all` also returns `comments[]`, `lastBy`, `addressed` (`direct`, `indirect` or `no`, advisory only), `waitingOnReviewer` and `startLine`.
Squad PR inbox: `aidp gh squad-repos` and `aidp gh squad-prs` (read-only), served by the daemon at `GET /api/prs`.
PR detail: `aidp gh pr-detail|pr-files|pr-file-diff` read a PR; `comment-create|edit|delete`, `review-comment-create|edit|delete`, `review-submit` and `run-rerun --failed` write to it; `aidp sonar report --pr <n> --envelope` reads its Sonar gate. The daemon keeps drafts and viewed marks in `~/.aidp/gateway/pr-drafts.json` and `pr-viewed.json`. See `docs/gateway-api.md`.

PR attention: the daemon raises deduplicated PR signals (new comment, changes requested, CI or Sonar red, conflict, mergeable, review requested) at `GET /api/prs/attention`; `POST /api/prs/attention/read` marks them read, and `notifications.pr` in `~/.aidp/config.json` toggles each one. See `docs/gateway-api.md`.
Selected threads only: `aidp gh pr-fix <repo> <n> --threads <ids> [--points <ids>] [--note-file -]` starts a free autonomous session through the daemon (`POST /api/prs/fix`) that fixes those threads, replies and pushes to the PR head branch. Use it for a quick fix of a few threads; use the `pr-comments` pipeline to classify, plan and review every comment.
PR actions: `aidp gh pr-ready`, `pr-update-branch`, `pr-merge`, `thread-reply` and `thread-resolve`; `aidp gh pr-create --draft` opens a draft PR (the `pr_draft` setting sets the default). See `docs/exit-codes.md`.

Test infrastructure notes (LOC-49), relevant when `cargo test --workspace` runs in a `verify` stage:
- `aidp-testkit`'s `ProcShim` installs a real `<name>.exe` on Windows (an argv-recorder dev binary that
  writes one argument per line into `ARGV_FILE`), not a `.bat`: `CreateProcessW` cannot run a `.bat`
  as a command.
- The `aidp-integrations` mock server shuts down deterministically (`MockHandle` stop/`Drop` returns
  promptly).

## Where the output goes (v7)

`aidp` resolves the vault on the first hit of: `--vault <path>` → `AIDP_VAULT` → `~/.aidp/config.json`
→ auto-detection (the nearest directory holding a `.obsidian/`, searched from the workspace outwards,
so a vault sitting *beside* the repos is found). No hit ⇒ the legacy `.aidp/pipelines/` layout, unchanged.

The gateway daemon resolves the vault with `aidp_core::vault::resolve_global` (`AIDP_VAULT`, then
`~/.aidp/config.json`; no auto-detection). It honours the same disable values as `aidp run init`
(`off`, `none`, `0`, `false`, `-`, case-insensitive): `/api/status` reports `vault: null`,
`/api/dispatcher/plan` answers mode `no-vault`, `/api/tickets` answers `503`, and no directory is created.

```
<vault>/Pipelines/_Pipelines.md                     dashboard: every repo, recent runs
<vault>/Pipelines/<repo>/_<repo>.md                 repo MOC: every feature
<vault>/Pipelines/<repo>/<feature>/_<feature>.md    feature MOC: every run + shared research
<vault>/Pipelines/<repo>/<feature>/<stage>/         shared research (ticket root, reused across runs)
<vault>/Pipelines/<repo>/<feature>/<run_id>/        one run: run-state.json, workflow.js, stages

<vault>/Knowledge/_Knowledge.md                     index: every documented project
<vault>/Knowledge/<repo>/_<repo>.md                 project knowledge base (one folder per project)
<vault>/Knowledge/<repo>/architecture.md            transverse layer — what the service IS
<vault>/Knowledge/<repo>/Features/_Features.md      index of the feature layer
<vault>/Knowledge/<repo>/Features/<slug>.md         feature layer — one note per business feature

<vault>/TODO/_TODO.md                               board: every open ticket across services
<vault>/TODO/<service>/<KEY>.md                     one note per local ticket
```

- **A stage's outputs go to the ABSOLUTE stage dir given in its prompt; a relative `Pipelines/...`
  write is a bug.** When a vault is configured, `aidp run reconcile` and `aidp scope verify` detect
  any repo-relative path whose leading component is `Pipelines/`, `Knowledge/` or `.aidp/pipelines/`
  — a *vault leak*, distinct from a scope violation — and both commands exit **4** on one. `aidp run
  init` also seeds those paths into the repo's `.git/info/exclude` (untracked, per-clone) the first
  time a vault-backed run touches the repo, so a stray artefact cannot be committed even if written.
  If you hit exit 4: move the stray file into the vault, or `git rm --cached` it if it was already
  committed, then re-run. See the README's "Exit codes" table for the full code list.
- **`Knowledge/` is a sibling of `Pipelines/`, not part of a run.** It exists to give AI agents a
  standing context on a service instead of making each run rediscover it. A stage declaring
  `knowledge:` writes into `Knowledge/<repo>/`, so every such stage curates **one** set of notes and
  updates it in place, and the documentation outlives the run that produced it.
- **`TODO/` is a third sibling root, written only by `aidp ticket`.** Unlike `Pipelines/` and
  `Knowledge/`, it is not created by `aidp vault init` — the first `aidp ticket` command creates it on
  demand. No pipeline stage calls `aidp ticket` directly, but the gateway's dispatcher does start
  sessions from the tickets in it, under a slot ceiling, a one-session-per-project rule, and a
  token-budget verdict. Four guardrails shape what it launches: a sprint-scoped default JQL, a
  `waiting-on:<KEY>` skip while Jira blockers are unfinished, adoption of an already-existing PR
  (`aidp ticket adopt <KEY> --pr <url|number>`, `POST /api/tickets/:key/adopt`), and an epic context
  block in the prompt. See `docs/tickets.md` for the format and the command surface.
  With `jira.write_back.on_release` on, the dispatcher also watches merged tickets for a release tag
  (`autopilot.release_watch_until`), writes it to Jira with `aidp jira issue-edit` and moves the
  ticket to the `status_map.released` status (QA Validation). The driver never edits `autopilot.*`
  by hand.
- **Two layers.** The transverse layer (base root: `overview/architecture/events/data/domain/
  operations.md`) answers *what is this service* — read cold, kept small. The feature layer
  (`Features/<slug>.md`, one note per **business feature**) answers *how does this business capability
  actually work here*: its rules and thresholds, its flow, the files that carry it, its edge cases.
  That second layer is the granularity an agent implementing a ticket needs, and the reason a single
  `domain.md` was not enough — it either stays vague or grows into an unreadable dump.
  A stage picks its layer: `knowledge: service` | `knowledge: feature` | `knowledge: true` (both).
  A `feature` stage's `stage_dir` **is** `Features/`, so `scope: stage_dir_only` keeps it out of the
  transverse notes.
- **Create-or-update is decided by `aidp`, not by the agent.** Told in prose to "update the note if
  the feature exists", an agent forks the base into `auto-exclusion.md` / `autoexclusion.md` /
  `auto-exclusion-v2.md`. So `aidp knowledge resolve --name "<feature>"` scores the run against every
  existing note (exact name/alias/slug → 1.0, a Jira key listed in `tickets:` → 0.95, else word
  overlap) and returns `update` with the file to edit, `create` with the exact path to write,
  `ambiguous` with the candidates to read, or `supersede` when the best match is itself superseded —
  its score is capped at `0.50` (still a candidate, never an update target) and the target becomes the
  note named by that note's `superseded_by`. Every `knowledge: feature` stage gets that decision
  **already computed** in its prompt. A run identified only by a ticket key is reported as `weak`:
  the agent is told to resolve again with the business name rather than assume the feature is new.
- **Bootstrapping a service**: `knowledge-init` (`/run-pipeline-v7 knowledge-init .`) creates a base
  from nothing — 6 parallel read-only scans (structure ‖ events ‖ data ‖ domain ‖ operations ‖
  business features), a gated `write_knowledge` for the transverse notes, `write_features` fanned out
  one sub-agent per feature, then `verify` (audits every claim against the code) → `correct`. Run it
  once per service; re-running it is safe.
- **The loop that keeps it true afterwards**, wired into `dev-*`, `study`, `tech-doc` and `func-doc`:
  1. `knowledge-read` (sub-pipeline, `uses: knowledge-read`) runs *first* in discovery and distils the
     base into the run's `knowledge_context.md` — starting with the **feature note** of the work at
     hand — and ends with an explicit "what the base does NOT answer" list, which is what the Jira /
     Confluence / code reads then go and find.
  2. every research and planning stage also gets the base injected into its prompt: the transverse
     notes as a file list, the feature notes with their names and aliases, and the one feature note
     matching this run named outright — "read this FIRST, investigate only what it does not answer".
  3. `knowledge-update` (`uses: knowledge-update`) runs *last* in discovery, as **two** stages:
     `knowledge_update` (`knowledge: service`) folds service-level findings into the transverse notes,
     then `knowledge_feature_update` (`knowledge: feature`) creates or updates the note of the feature
     the run actually worked on. Both leave the base unchanged when the run uncovered nothing new.
- The base is hand-editable: nothing there is ever overwritten by generation (only `_*.md` index notes
  are generated), so a correction you type by hand benefits every later run. Filling a feature note's
  `aliases:` is the highest-value hand edit there is — it is what stops a future run duplicating it.
  After editing by hand, `aidp knowledge index` refreshes the index notes.

- `<repo>` comes from the git `origin` remote, so two checkouts of one repo group together.
- **Notes prefixed `_` are generated** and rewritten on every `aidp` call that touches the run —
  never hand-edit them, and never tell a sub-agent to write one.
- **Nothing aidp writes stays in the repo.** Per-repo state that belongs to no single run — the CI
  autopilot's `run-in-flight` marker and its `ci-autopilot.state.json` ledger — lives in
  `<vault>/Pipelines/<repo>/.state/`, dot-prefixed so neither Obsidian nor the note generators see it.
  A copy left in `<repo>/.aidp/` by an older binary is moved out on first touch.
- What stays in the repo is what **you** put there and aidp only reads: `.aidp/dev-target.json` and
  `.aidp/sonar.json` (connection config), `.aidp/ci-autopilot.json` (per-repo autopilot config) and
  `.aidp/pipeline/flow/` (project-local templates). Those are inputs, not output.
- Pre-v7 runs stay in `.aidp/pipelines/` and stay loadable, resumable and listed by `aidp status`.
  Nothing was migrated.

Useful commands: `aidp vault status --format json-pretty` (where it resolves and why),
`aidp vault init --vault <path>` (create the skeleton + pin it in `~/.aidp/config.json`),
`aidp vault reindex` (regenerate every `_*.md`). Per-run escape hatch: `aidp run init --no-vault`.
Knowledge base: `aidp knowledge list` (both layers), `aidp knowledge resolve --name "<feature>"`
(create-or-update decision), `aidp knowledge index` (refresh the index notes after a hand edit),
`aidp knowledge path`, `aidp knowledge stats` (note counts, oversized notes, handle collisions),
`aidp knowledge supersede --slug <s> --by <s> [--reason <r>]` (the only write on the `knowledge`
surface — upserts `status`/`valid_until`/`superseded_by`/`supersede_reason` on the named slug,
preserving every other frontmatter key verbatim, position included), `aidp knowledge stale` (dates
notes against `git log` on their `files:` key; never exits non-zero over a stale note), `aidp knowledge
gc` (reports forks, dead `superseded_by` pointers and notes whose `files:` all vanished — it never
writes, moves or deletes a note; it only prints the command a human can choose to run). Tunable via
`settings.knowledge.prompt_budget_chars` and `settings.knowledge.max_note_bytes`; a misspelled
`settings:` key is not rejected — `settings.additionalProperties` stays `true` on purpose. Inside a
stage they need no flags — `$AIDP_RUN` locates the base.

## When to invoke

- `/run-pipeline-v7 <template> [target]`
- "lance la pipeline v7 …", "run pipeline v7 …"
- Any v4-style request where the user explicitly wants the v7 (Workflow) engine — e.g. for benchmarking
  v4 vs v7.

## CI autopilot — pipelines that start themselves

Claude Code has no "CI failed" hook: hooks fire on **Claude's** lifecycle, not GitHub's. `aidp ci` is
the bridge. It probes the PR in flight, fingerprints each problem so it fires exactly once, and speaks
the hook protocol (JSON on stdin, JSON on stdout, exit 2) so `settings.json` holds one-liners.

| signal | source | pipeline |
|---|---|---|
| `gha` | failing GitHub Actions checks on the PR head sha | `gha-fix` |
| `sonar` | SonarQube quality gate red, or new-code issues | `sonar-fix` |
| `comments` | unresolved PR review threads | `pr-comments` |

Three hooks (`hooks/ci-events.example.json`, installed by `/pipeline-setup-v7`):

- **`PostToolUse` (Bash), `asyncRewake: true`** — the one that makes it feel automatic. After a
  `git push` / `gh pr create`, it stays in the background for up to 30 minutes polling the checks, and
  **wakes the session on exit 2** the moment the build goes red. You push, you carry on, Claude comes
  back to you with the failure.
- **`Stop`** — at the end of a turn, exits 2 with the directive if anything is pending, so the fix run
  starts without you typing. Guarded against loops by `stop_hook_active`, the `max_notifications` cap
  (default 1) and a hard skip while the repo's `run-in-flight` marker exists (never interrupt a
  running pipeline to start another).
- **`UserPromptSubmit`** — cache-only, never touches the network; its stdout becomes context, so a
  session that was not woken still knows the PR has three unresolved comments and a red gate.

When woken, you invoke the **`ci-autopilot`** skill. It reads `aidp ci dispatch`, runs the pipelines
one at a time via the normal v7 driver protocol above, and acks each signal. **The autopilot decides
whether to start a run; it never decides whether to push.** Every one of those pipelines gates its
`push`/`reply` stage, so nothing reaches GitHub without the user approving it. The dispatch plan also
carries the PR branch's ticket key as a `feature` input when the branch names one
(`feat/PIXEL-5694` → `PIXEL-5694`), so the run is filed under that ticket instead of a PR-numbered
folder — `target` still stays the PR number.

**Which PR it watches.** First the one of the branch you are on; if that yields nothing usable —
no PR, a draft, a merged one, an ignored branch — it falls back to your own open PRs
(`gh pr list --author @me`) and to the ones it has seen before. That fallback exists because a branch
is routinely *not* checked out: Claude Code runs isolated stages in throwaway worktrees under
`<repo>/.claude/worktrees/`, and people switch branches. When a signal is recovered that way, the
directive says so and names the branch, because **any stage that edits or pushes code must run on the
PR's branch**.

**Where its files live.** Always the **main** worktree (`git rev-parse --git-common-dir`), never a
linked one — otherwise the config you set in the main checkout is invisible to a worktree, and
everything the autopilot learns dies when that worktree is cleaned up.

**Draft PRs** are skipped by default (`ignore_draft_prs`) — work in progress is allowed to be red. The
PR is still remembered, so the moment it is marked ready the next probe picks it up.

Control surface (`/ci-autopilot-v7`, or directly):

```
aidp ci status --refresh --format json-pretty   # PR, checks, Sonar, threads, derived signals
aidp ci dispatch                                # which pipeline, what target (PR), what inputs (incl. feature ticket key)
aidp ci ack --all --note "handled"              # silence the current state (re-fires on a new sha)
aidp ci config --enable false --save            # off for this repo
aidp ci config --auto-dispatch false --save     # report but never start a run unprompted
aidp ci reset                                   # forget every dispatch record
```

Config lives in `.aidp/ci-autopilot.json` (per repo) or `~/.aidp/config.json` → `"ci"` — config is
yours, so it stays in the (gitignored) repo `.aidp/`. State lives in the vault, at
`<vault>/Pipelines/<repo>/.state/ci-autopilot.state.json` (falling back to the legacy
`.aidp/ci-autopilot.state.json` only when no vault is configured) — state is ours, so it does not sit
in the repo. The repo file still wins for ordinary config fields, but `auto_dispatch` needs the user
config: set `ci.auto_dispatch: true` in `~/.aidp/config.json` (LOC-27), so a cloned repo cannot start
pipelines by itself.

Graphify: `settings.knowledge_graph.graphify_bin` must be a bare program name resolved on `PATH`;
an explicit path only via the user env var `AIDP_GRAPHIFY_BIN` (LOC-27).

Sonar needs a host and a token: `aidp sonar config --host https://sonarqube.pixelhood.net --save` plus
`SONAR_TOKEN` in the environment (never in the repo). Without them the `sonar` signal degrades to
nothing and the other two keep working — `aidp ci status` reports the reason under `errors.sonar`.
Likewise, a missing or unauthenticated `gh`, or a rate-limited GitHub API, degrades only the PR signal
and is reported under `pr_error` — a key distinct from `skipped`, `checks_error`, `threads_error` and
`sonar_error`.

**Flow-owned tickets.** When the daemon runs with `dispatcher.launch: flow`, it delivers a ticket through the `ticket-delivery` flow and runs `gha-fix`, `sonar-fix` and `pr-comments` itself. `aidp ci dispatch` then reports the skip reason `owned-by-flow` for that PR: do not start those pipelines by hand, the flow owns the PR.

## Procedure (do EXACTLY this)

**Set `AIDP_SKILL_ROOT` to this skill's directory for every `aidp` call below** so v7 templates (not v4)
are discovered. On this machine that is `~/.claude/skills/ai-dev-pipeline-v7`. (PowerShell:
`$env:AIDP_SKILL_ROOT = "<v7 skill root>"`; bash: prefix each call with `AIDP_SKILL_ROOT=<v7 skill root>`.)

`aidp` no longer falls back to the source tree it was built from. A binary that was moved or installed
elsewhere, with no discoverable skill root, fails with an error naming `AIDP_SKILL_ROOT`: set it and retry.

Flag contract: `--workspace-root`, `--vault`, `--no-vault`, `--format` and `-v` are global, accepted before
or after the verb. `--vault` also works on the read commands (`run state|summary|reconcile|verify|record-resume`,
`progress`, `gate ...`, `scope ...`, `workflow emit|plan-status`, `cost ...`, `status`). `--format` takes
`json-compact`, `json-pretty`, `ndjson` (true one-line-per-element NDJSON) or `text`. `run summary` and
`run state` default to `text` (Markdown) and also accept `--format json-*`. Invalid enumerated values
fail at parse time with exit 2. Details: `docs/migration-LOC-39.md`.

`--workspace-root` is honoured when a pipeline id is resolved (`validate`, `pipeline expand`, `run init`,
`run start`); the process cwd no longer decides which workspace's templates are searched.

Stdout and stderr: read stdout only. It carries the machine contract (bare `run_id`, bare script path,
JSON). When you need the phase that `aidp workflow emit` prints on stderr, read stderr line by line and pick
the line whose JSON has a `phase` key; ignore every other line, and never `JSON.parse` stderr as a whole.
`RUST_LOG`, `AIDP_LOG` and `-v`/`-vv`/`-vvv` only add log lines to stderr and are for debugging.

Two behaviours to expect. When the run has no knowledge graph, `aidp workflow emit` drops the
`graphify-context` skill from every stage and prints one compile warning per stage; this is not an error.
Review findings may carry a `skill_rules` axis: `aidp skill-rules check` ran over the diff and its
`blocking` findings (file and line) are merged like any other axis. You never run it yourself.

### Templates `dev-flow` and `correction-after-test-dev`

Before `aidp run init`, run `aidp project stack <workspace>` and offer its `stack` value as the answer to the `stack` enum through AskUserQuestion; the user may pick another value. `unknown` is a refusal, not a choice: run init rejects it, so escalate to the user instead of proceeding. `correction-after-test-dev` never opens a PR; `dev-flow` takes `pr` as `draft`, `ready` or `none`.

### Exit 127: `aidp` not found mid-run

Sessions launched by `aidp gateway` receive `AIDP_BIN` (absolute path of the `aidp` binary) and a PATH
prefixed with its directory. The binary can still vanish or the PATH break after launch. If any
`aidp ...` call exits 127 (or the shell says `aidp: command not found`):

1. **Retry the exact same call through `AIDP_BIN`**, keeping `AIDP_SKILL_ROOT` set: bash
   `"$AIDP_BIN" <args>`; PowerShell `& $env:AIDP_BIN <args>`. If it works, use `$AIDP_BIN` for the rest
   of the run.
2. **If `AIDP_BIN` is unset or also fails, STOP and tell the user — never silently.** Report:
   - the run dir (`run_dir` printed by `aidp run init` / `run start`, i.e.
     `<vault>/Pipelines/<repo>/<feature>/<run_id>/`), the `run_id`, and the Workflow `runId` if one exists;
   - the last completed step;
   - the exact command to replay once `aidp` is reachable again:
     `aidp run record-resume --run <run_id> --workflow-run-id <runId> [...]`, with the
     `--pending-gate` / `--approved-gate` / `--crossed-boundary` flag the current branch required,
     then `/pipeline-resume-v7 <run_id>`.

Never abandon a run without saying where it stands.

0. **Check the vault** — `aidp vault status --format json-compact`. If `configured` is `false`, tell the
   user their artefacts will land in the repo's `.aidp/pipelines/` and offer `aidp vault init --vault <path>`;
   if `ambiguity` is set, surface it (several vaults were in reach) before starting a run. One call, then
   move on — do not block the run on it.

1. **Pick template** — `aidp pipeline list --format json-compact`. Use `AskUserQuestion` to let the user
   choose. Never infer.

2. **Collect inputs** — inspect the chosen pipeline's `inputs:` (call `aidp pipeline show <id>` if needed).
   For every `kind: enum` input with a `values` list, use `AskUserQuestion` (default first/recommended).
   For a `kind: list` input (e.g. `jira_keys` — several unitary US of one feature), ask the user for the
   keys and pass each with a repeated `--input jira_keys=<key>` (accumulates into an array — no
   hand-assembled JSON needed); a single ticket is just one `--input`. If the pipeline also declares an
   optional `feature` string (epic/label), pass it when the user names one — it becomes the run's
   grouping folder and the `feat/<feature>` branch; otherwise both are derived from the ticket list.

   **Dual-mode demand (`jira_keys` + `prompt`).** A template declaring BOTH takes its demand either way,
   and you must pass exactly one:
   - the user named tickets → `--input jira_keys=PIXEL-1 [--input jira_keys=PIXEL-2 ...]`;
   - the user described the work in prose, with no ticket → `--input prompt="<their request, verbatim>"
     --input feature=<short-kebab-slug>`. Pass the request **verbatim** — do not summarise or restructure
     it; the `discover_prompt` stage is what turns it into requirements, and it quotes the original.
     `feature` is **required** here: with no ticket list, nothing else can name the branch, the vault
     folder or the PR. Derive the slug from the request and confirm it with the user in the same
     `AskUserQuestion`.
   If the user gave neither, ask which mode they want before starting the run — a run with an empty
   demand compiles fine and then wastes a full pipeline. `aidp run start` (next step) validates every
   input against the pipeline's contract and **fails, naming the missing key**, rather than guessing —
   relay that failure through `AskUserQuestion` and retry; never invent a required input yourself.

   **`--inputs-file <path>`** reads a JSON object from a file and injects it on exactly the same path as
   `--inputs` — it conflicts with `--inputs`, so pass one or the other, never both. Reach for it instead
   of typing the payload on the command line whenever the content is not plain ASCII: a ticket body with
   accents passed through argv has already come out as mojibake, and a file lets you write ASCII-escaped
   JSON (`\uXXXX`) that `serde_json` decodes losslessly. `--input k=v` still layers on top either way.

3. **Start the run** — `aidp run start --pipeline <id> --target <id> [--input k=v ...] [--inputs-file
   <path>] [--approve-gate <stage> ...]` replaces `run init` + `workflow emit`: it initializes, compiles,
   and prints one JSON object — `run_id`, `run_dir`, `vault` (`{root, source}` or `null`), `script_path`,
   `awaiting_plan`, `awaiting_composite` (booleans — either can already be true on this very first call,
   e.g. when a template opens with a `type: pipeline` stage), `auto_approved_gates` (the gates this call
   pre-approved), `gates` — now **the
   gates still awaiting approval**, i.e. this pass's gates minus whatever was pre-approved (with nothing
   pre-approved the two lists are identical, so a driver that never passes `--approve-gate` sees no
   change) — and `warnings` (LOC-33): a list of init-time problems that no longer stop the run or go to
   stderr, such as an unevaluable `when:`/`gate` expression or an undeclared `--input` key. An empty list
   is normal; a non-empty one must be relayed to the user, not silently dropped. Capture
   `run_id`/`script_path`; relay `run_dir` (vault path, not derivable from the repo).
   `--pipeline` takes the pipeline **id**, not a file path.
   `aidp validate` and `aidp run start` refuse a pipeline whose `needs` graph has a cycle or an
   unresolved dependency: they exit 1 and name the stages (`cycle or unresolved dep among: [...]`).
   The fix is in the template, never a retry — relay the error to the user and stop.

4. **Run the Workflow** — invoke the `Workflow` tool with `{ scriptPath: "<script path>" }`.
   On the first call do NOT pass `resumeFromRunId`. Capture the returned `runId` and the final result
   object.

5. **Approve a gate, if one is pending** — a gate pre-approved at `run start` (via `--approve-gate` or,
   absent that flag, `AIDP_AUTO_APPROVE_GATES`) never produces `awaiting_gate` in the first place, not
   even on the very first `Workflow` call, so this step is simply never reached for it. When you are
   driving an autonomous session — no human on the other end to ask — set `AIDP_AUTO_APPROVE_GATES` to a
   comma-separated list of stage names (or `*` for every gated stage) before starting the run, and it
   gets past any gate the session is allowed to pass without ever pausing here.

   Otherwise, when the result is `{ status: "awaiting_gate", gate: "<stage>", ... }`, `AskUserQuestion`
   ("Approve stage `<stage>`?" → approve / abort) after first persisting the
   pause so it survives even if never approved. The result file is already written (step 6, before any
   `aidp` call); now run `aidp gate pending --run <run_id> --gate <stage> --workflow-run-id <runId>`.
   - **approve** → `aidp gate approve --run <run_id> --gate <stage> --workflow-run-id <runId>`. It
     records the approval, re-compiles the script (baking `__approved` in — the `agent()` calls are
     unchanged, so the cached prefix still replays), and prints the exact object to resume with:
     `{ scriptPath, resumeFromRunId, args: { approved_gates: ["<stage>"] } }`. Pass that object to
     `Workflow` verbatim. **You no longer need to re-pass every prior gate** — persisted approvals are
     baked in. Repeat from step 6 with the new result.
   - **abort** → `aidp gate abort --run <run_id> --gate <stage>`; stop; print `aidp run summary --run
     <run_id>`. The runId + pending gate are already persisted, so `/pipeline-resume-v7` later
     **re-proposes this exact gate** instead of re-running the whole pipeline.

6. **Write the result, reconcile it, then handle it — on every branch, not only `done`.** On every
   Workflow result (`done`, `awaiting_gate`, `awaiting_plan`, `awaiting_composite`, `paused`) your
   **first** action, before any `aidp` call, is to write the result object verbatim to
   `<run_dir>/workflow-result.json` (the run dir is known from `run start` / `run init`, so no `aidp`
   call is needed; this single plumbing write is exempt from the no-`Write` rule). If the background
   Workflow returned an envelope, write its `.result`. This matters because, if `aidp` is unavailable
   right after the pass, the file is the only cold-start record of the `runId` and the pending gate.
   Then run `aidp run reconcile --run <run_id> --from <run_dir>/workflow-result.json` before acting on
   `status`, followed by the branch's own `aidp run record-resume` / `aidp gate pending` call. Every
   branch carries the full `results` map, so reconcile keeps run-state truthful whether the run
   finished, paused at a gate, stopped for a plan, or failed. `aidp run record-resume --run <run_id>
   --from-result <file>` rebuilds `workflow_run_id` and `pending_gate` from that file (bare or
   `{result:{...}}`-enveloped) under the usual invariants: an approved gate is never re-opened, and an
   explicit `--workflow-run-id` / `--pending-gate` wins.
   If the pricing grid named by `AIDP_MODEL_PRICES` is missing, unreadable, or not valid JSON, reconcile
   still succeeds (exit 0, summary printed): every stage's usage reads `source: "unavailable"` with no
   `cost_usd` key, the run carries no top-level `cost` key at all, and one stderr line names
   `AIDP_MODEL_PRICES` and the file. Relay that line; it is not a failure.
   - **`done`** — print the reconciled summary and stop.
   - **`awaiting_gate`** — handled in step 5 above; reconcile first, then go there.
   - **`{ status: "awaiting_plan", at: "<stage>", producer: "<planner stage>", plan_path: "<file>" }`** —
     the run reached a **plan-driven** stage (`plan_tasks:`). Its work is not described by the template:
     it is compiled from the planner's `plan.json` task graph, which only exists now that the planner has
     run. So is everything after it. This is a normal, expected stop — not an error, and **not** a gate
     (no `approved_gates` value passes it).
     In native mode the daemon then runs the tasks itself: in parallel when write sets are disjoint
     (cap `min(plan_tasks.max, settings.max_parallel)`), in plan order otherwise. Each task is held
     to its own `files`; an out-of-scope write is restored and fails the task. See
     `docs/agents-reference.md` §7.1.
     1. `aidp run record-resume --run <run_id> --workflow-run-id <runId>` — persist the resume anchor.
     2. `aidp workflow emit --run <run_id>` — `aidp` loads the plan, validates the task graph (unknown
        deps, cycles), **inserts an ordering wherever two tasks would write the same file**, and compiles
        the tasks plus the rest of the pipeline. If the plan is malformed the command **fails and says
        what to fix** — the plan is machine input, so the fix is to re-run the planner (or fix the
        template), never to hand-edit `workflow.js`.
     3. `aidp workflow plan-status --run <run_id> --format json-pretty` — optional and cheap: show the
        user how the work was parallelised (tasks, waves, concurrency cap, every inserted ordering and
        why). Worth doing: this is the run's real execution plan.
     4. Re-invoke `Workflow` with `{ scriptPath, resumeFromRunId: "<runId>" }` (plus
        `args.approved_gates` if a gate was approved earlier in this run). The cached prefix replays
        instantly; only the newly-compiled tasks and tail actually run. Then repeat from step 6.
   - **`{ status: "awaiting_composite", at: "<stage>", composite: "<name>" }`** — the run reached the
     entry of a composite (`type: pipeline`) stage, e.g. `review` in `dev-aidp`, and stopping here was
     actually necessary: crossing it without stopping would have pushed the current Workflow pass over
     `settings.max_stages_per_pass` (LOC-48, default 10 — see "Composite stages" below). A composite
     small enough to fit under the budget alongside what the pass already accumulated is instead merged
     straight into the surrounding pass and never produces this status at all — so how many separate
     `Workflow` invocations a pipeline like `dev-aidp` needs depends on the budget, not on how many
     `type: pipeline` stages it declares. Unlike `awaiting_plan`, there is nothing to wait for — the
     composite's stages are already fully known — so this is purely a deliberate size/organisation
     boundary. This is a normal, expected stop — not an error, and **never** a gate: no human is
     asked, and no `approved_gates` value passes it.
     1. `aidp run record-resume --run <run_id> --workflow-run-id <runId> --crossed-boundary <name>` —
        persist the resume anchor AND record this boundary as crossed, in one call.
     2. `aidp workflow emit --run <run_id>` — the boundary is now behind it; `aidp` compiles the
        composite's stages plus everything after, up to the next boundary (another composite, a
        `plan_tasks:` stage, or the end of the pipeline).
     3. Re-invoke `Workflow` with `{ scriptPath, resumeFromRunId: "<runId>" }` (plus
        `args.approved_gates` if a gate was approved earlier in this run). Then repeat from step 6.
   - **`{ status: "paused", at: "<stage>", envelope: {...} }`** — a sub-agent returned
     `blocked`/`failed`/`needs_review` or asked questions. **First, always**: `aidp run record-resume
     --run <run_id> --workflow-run-id <runId>` — persist the resume anchor *before* diagnosing or
     relaying anything, the same discipline as the `awaiting_plan` branch above. This is not a gate (no
     `--pending-gate`, no `--approved-gate`), just the runId — but without it, `state.workflow_run_id`
     stays null and a later `/pipeline-resume-v7` (especially from a fresh session) has no anchor to
     resume from and falls back to re-running the whole pipeline. Do this for every `paused` subtype
     below, including a review stage's `harness_gap` pause. If `envelope.signals.transport_error === true`,
     this is a dropped connection, not a real failure: just re-run the same `Workflow` call (the cached
     prefix replays) — do not investigate. If `envelope.signals.malformed_envelope === true` instead,
     `safeAgent` (`workflow_emit.rs`) caught a *shape* problem in the rendered envelope after a
     successful call — `status`/`summary` not a string, `signals` present but not an object, or a
     `signals` fragment leaking inside the `summary` string (the case seen in production: a sub-agent's
     `signals` object got flattened into its `summary` text and blocked a whole pipeline queue). This
     guard only makes the symptom diagnosable — the flattening happens inside the agent()/Task platform,
     outside this repo, and the guard cannot fix that root cause. Treat it like any other failed
     envelope: read `envelope.summary` for which field was malformed and re-run the stage; it is not a
     transport retry. Otherwise relay `envelope.ask_user_questions` (if any) via
     `AskUserQuestion`, surface `envelope.summary`, and stop unless the user resolves the blocker. (v7 v1
     does not auto-resume mid-stage questions — re-run after the user provides what's missing.)
     **No reviewer ever pauses on `needs_review`** — the emitter marks every `agent: reviewer` stage
     soft, and `agents/reviewer.md` requires `completed` + `signals.has_blocking_findings` for findings.
     So a paused reviewer means `blocked`/`failed`/questions: the review could not be *produced*, not
     that the code has problems. `at: "<loop name>"` means the correction loop **exhausted its
     iterations** with blocking findings still open; the envelope is that reviewer's. A loop body must
     end with a stage re-declaring the output its `when:` reads (for convergence detection), which
     `templates/dev-aidp.yaml` now does like every other `dev-*` template; a reviewer stage feeding
     a correction loop no longer hard-pauses on a skipped control alone.
     
     **Authoring `type: loop` templates:** A correction loop's body MUST end with a stage that 
     re-declares the output its `when:` clause reads — conventionally `rereview` with `agent: reviewer` 
     and `outputs: [findings.json, summary.md]`. Without it, the loop cannot converge: the condition 
     never changes and the review stage is not linked back for the emitter to track. Every `dev-*` 
     template (`dev-aidp`, `dev-csharp`, `dev-lambda-ts`) now follows this pattern; see docs/agents-reference.md 
     for the full field-level contract.
     
     **Nor does a single plan task** — a task deferring shared-file work to `follow_ups`, or flagging a
     concern with `needs_review`, is normal (§ Plan-driven stages below); the merge sub-agent sees every
     task's full envelope and decides whether the *stage* still needs a human. Only `blocked`/`failed`/
     `ask_user_questions` pause at `<stage>#<task_id>`; an unreconciled `needs_review` instead shows up
     one step later, on the merge dispatch's own envelope at `<stage>`.
     **Nor does a skills downgrade.** An executor that did not load its declared skills keeps its reply,
     capped at `needs_review` with `signals.skills_not_loaded`; the run continues, reconcile records the
     list per stage and `aidp run summary` prints it as a warning. Read-only roles still fail after one retry.
     **Nor does a stage a correction loop repairs.** Examples: `implement` in `dev-aidp` and
     `dev-lambda-ts`, `tests__tests_suite` in `dev-csharp` (the stage the review axes read), and the
     in-loop `refix`. When such a stage replies
     `needs_review` (red tests it could not fix, even ones that also fail on master), the run
     continues. The review receives the report and files it as a blocking finding, and the first
     correction loop iteration fixes it. The emitted `__REVIEWED` set lists these stages. If the loop
     cannot fix it, the run pauses at `at: "<loop name>"`.
     ⚠️ This pause is **not a gate**: no `approved_gates` value reopens it, and resuming replays the
     cached envelope and pauses at the same place. Fix the cause, don't try to approve past it.

   Approved gates are persisted and baked into `__approved` on re-emit, so you only pass the **latest**
   gate in `args.approved_gates`; prior approvals stay open automatically.

   A `paused` at `<stage>#<task_id>` is a **plan task** that is blocked/failed or asked for a human —
   never one that merely returned `needs_review` (see above) — and the stage's other tasks may well have
   succeeded; `aidp workflow plan-status` / `aidp run state` show which.

   If `aidp run reconcile` prints a `warning:` line about a plan stage's task count — a `0/N` sitting
   under a stage that itself succeeded — **relay it and do not read the count as failed work**. It means
   the per-task outcomes never reached run-state, which is a reporting failure, not a run failure. The
   likeliest cause is handing `reconcile` a *wrapped* result: it wants the workflow object itself (the
   `.result` of what a background `Workflow` returns), not the enclosing envelope. Re-run `reconcile`
   with the unwrapped object; never re-run finished work on the strength of that count alone.

   `aidp run reconcile` also derives a scope verdict from git, and this **never affects its exit
   code** — reconcile always exits 0 (gated by `settings.verify_scope`, default `true`). If the
   reconciled summary carries a non-empty `scope_violations` (paths a successful stage wrote outside
   its declared `scope`, folded into `run-state.json`), relay that to the user as information — it is
   not a pause and not a failure. `aidp scope verify --run <run_id>` is the command that exits non-zero
   on a violation; `aidp run verify --run <run_id>` replays the separate output-content checks
   (`StageState.verification`); `aidp git outcomes` reports whether a stage's commits were later
   reverted or hotfixed.

   Every one of the five result shapes above — `done`, `awaiting_gate`, `awaiting_plan`,
   `awaiting_composite`, `paused` — can also carry `guard_findings`: prompt-injection patterns the
   emitted script's own guard caught at a
   stage-to-stage recollement (`fanin`, `taskin`, `taskUp`, `up`). A non-empty `guard_findings` is
   **neither a pause nor an error** — the suspect block was wrapped and labeled untrusted, never
   stripped, and the branch you are already on still applies exactly as documented above.
   `aidp run reconcile` folds it into `run-state.json` under the same key; relay it to the user as
   information, the same way as `scope_violations`.

   A review stage's `has_blocking_findings` is no longer a model's word alone: the synthesis stage of
   `templates/review.yaml` calls `aidp findings merge --input <p> --input <p> ... --out <p>` **itself**,
   which recomputes `severity_counts` and derives `has_blocking_findings` from the merged findings
   instead of trusting a summary. You never call `aidp findings merge` yourself — it is the stage's own
   work, same as any other `aidp` subcommand a sub-agent runs.

   A stage can also report an item as `cancelled` — a fan-out shard or a plan task denied dispatch for
   lack of budget, distinct from one that actually failed — without the run itself failing:
   `cancelled` is a per-item status inside `fanin`/`taskin`, not a run status. `settings.on_item_failure`
   (default `pause`) decides whether a failed or cancelled item still stops the stage, or is instead
   named to the merge sub-agent so the stage can carry on.

## Local PR review — `pr-review` then `verify-review`

Run `pr-review` with the PR as `pr` for pass 1. Run `verify-review` for pass 2 or 3 with `pass` and
`previous_run` (the prior pass's run id). The agents only read and write `findings.json` or
`verify.json`; GitHub writes go through `aidp gh review-post` and `aidp gh thread-reopen`, called by
the workflow, never by the templates. The loop ends when a pass is clean, or after pass 3 with
status `escalated`. See `docs/agents-reference.md` §3, §9.2, §12.

## Plan-driven stages — how the plan decides the parallelism

In the `dev` and `dev-lambda-ts` templates the planner no longer writes prose for a human to interpret:
it writes **`plan.json`** (`schema/plan.schema.json`), *one object per task to realize*, each carrying its
own `prompt`/`instructions`, its `needs` (real data dependencies) and its `files` (the task's write set).
The `implement` stage then declares `plan_tasks: { from: "{{ stages.plan.outputs.plan }}", max: N }` and
has **no work of its own** — `aidp` compiles it from the plan:

- every task becomes one sub-agent, dispatched as soon as *its own* dependencies finish (no wave barrier);
- tasks whose write sets are disjoint run **in parallel** in one shared working tree, up to `max`;
- tasks that would write the same file are **serialized automatically**, and the reason is logged;
- files listed in the plan's `shared_files` are off-limits to every task: each describes the change it
  needs in `follow_ups` and a final **merge sub-agent** applies them all, builds, runs the suite, and
  produces the stage's declared outputs;
- production code **and** tests are tasks of the same plan (`kind: code` / `tests`), so there is no
  separate stack-specific test stage.

The consequence for you as the driver: a plan-driven run compiles in **two passes** and therefore needs one
extra `emit` + resume round-trip (the `awaiting_plan` branch above). Nothing else changes.

## Composite stages — one Workflow pass per `type: pipeline`, *unless it fits the budget* (LOC-48)

A stage declared `type: pipeline, uses: <other pipeline>` (e.g. `dev-aidp`'s `review`, which reuses
`templates/review.yaml`) is inlined by `aidp` at `run init` — by the time a script is compiled, its
axes/synthesis stages already exist as plain, namespaced stages (`review__review_correctness`, …).

Whether composition folds a composite into whichever pass happens to be compiling depends on
`settings.max_stages_per_pass` (default **10**, close to the Workflow tool's own "keep workflows under
~10 agents" sizing guideline): a composite's entry stage forces its own new Workflow pass only if
crossing it *without* stopping would push the current pass's accumulated stage count over that budget —
exactly like `plan_tasks:` already does unconditionally for `plan`→`implement` (see the
`awaiting_composite` branch above, which is unaffected by this budget). A composite that fits under the
budget alongside what the pass already holds is merged straight through instead: it never produces
`awaiting_composite`, and the driver never learns it was even a `type: pipeline` stage. For `dev-aidp`
this means small composites like `knowledge` (1 stage) and `knowledge_sync` (2 stages) merge into the
neighbouring pass, while `implement` (a `plan_tasks:` stage, unconditional boundary) still stops on its
own, and `review` (5 stages) still gets its own pass if the accumulated total at that point would
otherwise exceed the budget. Raise `settings.max_stages_per_pass` on a pipeline whose composites are
individually small but numerous enough to still want fewer passes; lower it to force more, smaller passes
on a pipeline whose sub-agents are unusually heavy.

Unlike a plan, there is no artifact to wait for — a composite's shape is fully known before the run even
starts — so crossing a boundary that IS taken is purely mechanical and never involves the user:
`record-resume --crossed-boundary <name>`, re-`emit`, re-invoke `Workflow` with `resumeFromRunId`. A
pipeline with several composites still large enough to each need their own boundary (or a composite
before its `plan_tasks:` stage, or two in a row) simply compiles across more passes, one boundary at a
time, in whatever order they appear — you do not need to plan for this, `aidp` decides it the same way it
decides everything else.

## Execution plan

Workflow mode is unchanged: you still run `aidp workflow emit` and the `Workflow` tool exactly as above. Internally, `aidp` first compiles the pass into a typed execution plan, and `workflow.js` is a rendering of it. To inspect the plan for a run, call `aidp run plan-exec --run <run_id>`: it writes `<run_dir>/execution-plan.json` and prints the path. The plan covers the current pass only and is recomputed on every call. See `docs/native-execution.md`.

## Native execution

The daemon can also run a pipeline itself, with no Workflow session: `aidp run exec <template> --target <id> --inputs-file <path> [--foreground]`, or `POST /api/runs`. The driver protocol above applies to Workflow runs only and is unchanged. See `docs/native-execution.md`.

A native run carries an immutable `autonomous` flag. In an autonomous run every gate is pre-approved, agents never ask and record their choices as `assumptions`, and a stage blocked twice fails with reason `blocked-twice`. This does not change the driver protocol. See the Autonomy section of `docs/native-execution.md`.

### Which mode for which launch

| Launch | Mode | Autonomous by default |
|---|---|---|
| `/run-pipeline-v7`, `aidp workflow emit` | workflow | no |
| `aidp run exec`, `POST /api/runs` | native | per request |
| Viewer launch, `aidp gateway start --template` | native | no (viewer: user chooses) |
| Ticket dispatcher and Start, CI autopilot, flow `pipeline` step | native | yes |
| Presets and automations | native | opt-in with `autonomous: true` |

`engine: "workflow"` (legacy alias `v7`) on a daemon launch, preset or automation restores the Claude-session-driven `workflow.js` path. This skill itself always drives workflow mode. See `docs/native-execution.md`.

## ABSOLUTE RULES

1. **You MUST NOT** call `Edit`, `Write`, `MultiEdit`, or non-`aidp` `Bash` yourself during a run. All
   real work happens inside the Workflow's sub-agents.
2. **You MUST NOT** hand-edit the emitted `workflow.js`. If something is wrong, fix the template and
   re-run `aidp workflow emit`.
3. **You MUST NOT** plan, analyse, write code, or review yourself — that is sub-agent work inside the
   Workflow.
4. **You MUST NOT** invent stages, reorder them, or skip the gate/resume protocol. The emitted script is
   the only source of truth.
5. **You MUST NOT** create or edit a `_*.md` note in the vault. Those are generated by `aidp`; anything
   you write there is overwritten on the next call. Fix the generator, not the note.
6. **You MUST NOT** edit the `status:` or `autopilot:` keys of a note under `TODO/**`. Only `aidp
   ticket` transitions a ticket, every transition is journalled, and a hand-edited status is silently
   inconsistent with the journal. Reading such a note is fine. Editing the note's other frontmatter
   keys by hand stays allowed, but `key:` and `service:` must remain a single safe path component (no
   `/`, `\`, `:`, no `..`, not empty, never starting with `_` or `.`, no control character, none of
   `[`, `]`, `|`, `#` or a backtick, and not a Windows reserved device name like `CON`/`NUL`/`COM1`) —
   `aidp ticket` now refuses a note that breaks this, which drops it from `aidp ticket list` until the
   frontmatter is fixed; see `docs/tickets.md` for the full rule.

## What v7 changes vs v6

| Concern | v6 | v7 |
|---|---|---|
| Where artefacts live | `<repo>/.aidp/pipelines/<feature>/<run_id>/` | `<vault>/Pipelines/<repo>/<feature>/<run_id>/`, plus `<vault>/Knowledge/<repo>/` for durable service doc (transverse notes + `Features/<slug>.md`) |
| Reading a past run | open JSON/markdown in the repo tree | Obsidian: linked notes, properties, graph, search |
| Cross-repo view | none — one `.aidp/` per repo | one vault, `Pipelines/_Pipelines.md` dashboard |
| Agent summaries | only in the Workflow result file | folded into run-state and rendered in the stage note |
| Repo cleanliness | run artefacts sit in the working tree | the repo only keeps config + hook markers |

## How it differs from v4 (why it is more robust)

| Concern | v4 | v7 |
|---|---|---|
| Orchestration loop | LLM "dumb driver" calls `orchestrate next` each step | deterministic JS Workflow script |
| Sub-agent reply | free-text `reply.md` → `aidp reply parse` (brittle) | `agent({schema})` schema-validated |
| Token capture | driver passes `--last-tokens` from JSONL (race-prone) | Workflow runtime accounting |
| Independent stages | strictly sequential | `parallel()` fan-out |
| Executor write isolation | reactive PostToolUse scope-check | proactive `isolation: 'worktree'` (opt-in) |
| Resume | manual re-drive | native `resumeFromRunId` (cached prefix) |
| Gates | `ask_user` action in the loop | `awaiting_gate` return + resume with `args.approved_gates` |
| Plan | prose `plan.md`, read by the next agent | `plan.json` task graph — `aidp` compiles the workflow tail from it |
| Work sharding | one label list from the model (`signals.shards`) | per-task write sets; `aidp` parallelises and serializes conflicts |

## Files

- `AGENTS.md` — essential rules; `docs/agents-reference.md` — **full agent-facing reference** (architecture, command surface, data contracts, all
  schemas, template catalog, troubleshooting). Read it when this procedure isn't enough.
- `README.md` — human onboarding.
- `templates/` — v7 pipelines (`version: 5`). Stages may add `isolation: worktree`, `fan_out:` (shard
  labels) or `plan_tasks:` (compiled from the planner's `plan.json`). Extend via `/pipeline-new-v7`.
  Every `dev-*` / `test-csharp` template is **plan-driven**. `templates/legacy/` is the archive of
  templates retired from the discoverable park (same id, content unchanged, moved there with `git mv`)
  — `aidp` excludes that folder from discovery, so its ids never show up in `aidp pipeline list` and
  stay runnable only by explicit path.
- `agents/` — the 4 generic sub-agents (analyzer / planner / executor / reviewer). Extend via
  `/agent-new-v7`.
- `commands/` — the full v7 command set (`run-pipeline-v7`, `pipeline-setup-v7`, `pipeline-doctor-v7`,
  `pipeline-status-v7`, `pipeline-resume-v7`, `pipeline-validate-v7`, `pipeline-new-v7`, `agent-new-v7`).
- `hooks/` — one set. `ci-events.example.json` **starts** a pipeline (the CI autopilot, see above),
  merged idempotently via `aidp setup merge-hooks`. There is no pipeline-guard hook: v7 has a single
  execution path (the Workflow tool), so there is no separate "main orchestrator" session to block
  from editing files while sub-agents work. `settings.example.json` used to also install a
  PreToolUse main-agent write block and a PostToolUse `aidp scope check`, gated on
  `AIDP_RUN`/`AIDP_STAGE` being set — but those variables are only ever written into the sub-agent
  prompt text, never into the hook process's environment, so the gate could never be satisfied and
  the guard never fired. It also depended on `.aidp/active-run` + `agent-session.lock`, markers only
  `aidp orchestrate` ever wrote — and that command is gone. Both hooks were removed rather than
  re-armed on a new marker; scope is checked after the fact instead. `aidp doctor` surfaces the
  removal as `pipeline_guard_hooks_removed` (`ok: true` once no such hook is installed).
- `schema/` — JSON schemas consumed by the `aidp` binary, including `plan.schema.json` (the executable
  plan / task-graph contract) and `pipeline.schema.json` (where `plan_tasks:` is defined).
- `rust/` — the full `aidp` workspace (`aidp-cli`, `aidp-core`, `aidp-integrations`, `aidp-bench`,
  `aidp-gateway` — the resident control plane: headless claude sessions, automations, HTTP API + SSE
  for the viewer, `aidp-tickets` — the local ticket store behind `aidp ticket`; it defines the `IssueFetcher` port
  and does not depend on `aidp-integrations`, whose `JiraIssueFetcher` adapter `aidp-cli` injects:
  aidp-tickets <- aidp-integrations <- aidp-cli). Built by
  `bootstrap.ps1` / `bootstrap.sh` (or `/pipeline-setup-v7`).
- `docs/gateway-api.md` — the `aidp-gateway` HTTP contract (routes, events, presets), including the
  session-to-run link (`GET /api/sessions/:id/run`, plus `session.run` and, while a Workflow run is
  in flight, `session.workflow_progress` bus events), the
  automation dry-run/trace (`POST /api/automations/:id/test`), usage aggregation (`GET /api/usage`
  with persisted rate-limit history at `GET /api/usage/rate-limit`), the per-project launch queue
  (`GET /api/queue`, `session.queued` / `session.dequeued`), the dispatcher's token-budget verdict
  (`GET /api/dispatcher/budget`), the daemon's UI-token / read-only
  auth gate (`AIDP_UI_TOKEN`, `AIDP_READ_ONLY`), the LOC-26 access policy (`AIDP_ALLOWED_ORIGINS`
  for mutating cross-origin calls and CORS, header-only tokens with no `?token=`, single-use SSE
  tokens via `POST /api/events/token`, the `X-Hub-Signature-256` webhook signature), the request-body bound (`AIDP_MAX_BODY_BYTES`,
  `413`), `GET /api/health` (doctor, hooks, the `aidpBinary` PATH-binary block, Claude login,
  state-dir size, orphaned sessions), the adaptive CI probe (`GET /api/ci/probe-state`), the
  shell-free `claude` spawn and its `AIDP_CLAUDE_BIN` override, and the `POST /api/sessions` input
  validation (`permissionMode`/`effort` enums, `model` allow-list). It also covers the LOC-29
  bounds: `AIDP_MAX_INFLIGHT_REQUESTS` (`503` beyond it, distinct from the `413` body bound),
  bounded SSE streams (a stalled client is disconnected and one `overflow` bus event emitted), a
  tick that can answer `{"mode":"busy"}`, and a clean Ctrl-C stop of `aidp gateway serve` — see its
  `Concurrency and resource bounds — LOC-29` section. It also covers the per-ticket worktrees
  (`<vault>/Pipelines/<repo>/<KEY>/.worktree`, `GET /api/worktrees`, `worktree.*` events): the daemon
  admits one session per workspace, not per project, so `ui` sessions stay on the checkout while
  dispatcher sessions run in worktrees. `aidp gateway worktrees list|prune` inspects and sweeps them,
  and `aidp doctor` warns on Windows through the `git_longpaths` check when `core.longpaths` is not
  `true`. Read it when wiring or calling the gateway API.
**Free sessions.** `aidp gateway start --workspace project|ticket:<KEY>|scratch:<name>|path:<dir>` starts a
session outside a pipeline run (also `workspace`, `external` resume and reply `attachments` on the HTTP API). `path:` workspaces must lie under
`AIDP_ALLOWED_ROOTS` (default: registered projects and the vault). Sessions in `plan` permission mode run on a
read-only lane (ceiling 3) beside a writer; a writer on a busy workspace queues as `workspace-busy`. See
`docs/gateway-api.md`.

- `docs/tickets.md` — the local ticket store (`aidp ticket`): the Markdown-note format, the verbs
  (`new`, `list`, `show`, `render`, `edit`, `status`, `route`, `cancel`, `log`, `reindex`, `sync`,
  `start`, `plan`, `dispatch`, `budget`), and the `TODO/` layout. Read it before touching a ticket
  note or scripting against `aidp ticket`.
- `docs/flows.md` — the ticket workflow engine (`aidp flow validate|show|list|start|status|approve|verdict|retry|skip|pause|resume|abort`, plus the action verbs `ci-watch|deploy-dev|notify-teams|git-branch|jira-comment`, `aidp jira issue-assign --me`, `aidp jira issue-transition --to-status`, `aidp gh copilot-review <repo> <n>` and `aidp gh deployment-approve <run_id>`; they print JSON and exit 6 on a business refusal):
  the definition format, storage under `<vault>/Pipelines/<repo>/<KEY>/flow/`, restart recovery and the
  `flow.*` bus events. Definitions live in `templates/flows/`, which is not a pipeline directory.
- `benchmark/` — scenarios, fixtures and KPI definitions for `aidp bench`.
