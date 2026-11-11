# Ticket flows

A flow runs one ticket through a fixed sequence of steps: actions without AI, v7 pipeline runs, waits and human decisions. The daemon executes it, so it survives restarts and needs no Claude session. The viewer only displays it.

Two parts:

- `aidp-flows` (crate): model, validation, condition evaluator and the pure state machine `advance(def, state, event) -> (state, Vec<Effect>)`. No HTTP, no subprocess, no dependency on `aidp-core` or `aidp-integrations`.
- `aidp-gateway/src/flows/` (executor): applies each effect, feeds the result back as an event, persists state, publishes bus events.

## Definition format

Definitions live in `templates/flows/`, validated against `schema/flow.schema.json`. That directory is not a pipeline directory: `aidp pipeline list` does not read it.

```yaml
id: example-delivery
version: 1
inputs:
  ticket: { required: true }
settings:
  local: false
steps:
  - id: prepare
    kind: action
    use: noop
    phase: setup
  - id: implement
    kind: pipeline
    use: dev-aidp
    phase: build
    pushes: true
    with: { track: standard }
  - id: review
    kind: human
    phase: review
    then: [approve, reject]
```

| Field | Meaning |
|---|---|
| `id` | Flow id. |
| `version` | Integer. A run keeps the version it started with. |
| `inputs` | Named inputs given at start (`{ticket, flow?, settings?}`). |
| `settings` | Defaults, overridable at start. Read as `settings.*`. |
| `blocks` | Named step lists that steps insert by reference. `publish` then `ci` are the blocks inserted after a step that pushes, when the definition declares them; see [Host publishing](#host-publishing). |
| `steps` | Ordered list of steps. Ids are unique across steps and blocks. |

### Step fields

| Field | Meaning |
|---|---|
| `id` | Unique step id. |
| `kind` | `action`, `pipeline`, `wait`, `human` or `loop`. |
| `phase` | Label the viewer groups steps by. Returned for every step. |
| `when` | Condition. False gives status `skipped`, with the reason journaled. |
| `skip_on` | `local`: skip when the run is local (no remote). |
| `pushes` | `true` when the step's commits must reach the remote. The `publish` block, then the `ci` block, are inserted after it when declared. |
| `resource` | Daemon resource the `pipeline` step holds while it runs. Known value: `unity-editor`. Any other value, or a `resource` on another kind of step, fails validation (`bad_resource`). See [Daemon resources](#daemon-resources). |
| `use` | Registry entry (action or wait probe) or pipeline template id. |
| `then` | Follow-up step id or ids. |
| `on_error` | `fail` (default) stops the run on error. `warn` records a warning on the step and continues. |
| `with` | Parameters passed to the action, probe or pipeline inputs. |
| `optional` | `true` when the flow can go on without this step. The pipeline doctor may then skip it (`skip_optional_step`) when it keeps failing. Default `false`. |

### Step kinds

- `action`: a subprocess `aidp ... --format json-compact` built by `AidpCommand`, with a timeout. Its exit code decides the outcome (see Exit codes).
- `pipeline`: a native run (`mode: "native"`, autonomous by default) started by the gateway, origin `{kind: "flow", ref: <KEY>, label: <step>}`, cwd = the run's workspace. Inputs go in an ASCII-escaped JSON file. The executor awaits the native run, then reads the outcome from `run-state.json` and the stage artifacts `pr.json`, `pr-body.md`, `findings.json` and `verify.json` when present (outputs `pr_file`, `pr_body`, `findings`, `verify`), then the keys of the last stage's `result.json` (see Step outputs of a pipeline step). `engine: "workflow"` in `with` restores the session-driven workflow path. When the token budget is insufficient the step goes to `waiting_external` with reason `budget`.

  In a flow that defines the `publish` block, every run also gets `publish: host`: it commits and never pushes.

  The run's inputs carry the ticket's demand, the way the dispatcher launches a ticket: a local ticket gets its note body as `prompt`, or its summary when the body is empty (a legacy Jira mirror gets `jira_keys: [KEY]`). The ticket's `track` becomes `track`. `with.report_from: <step>` heads the `prompt` with the report that step stored (a correction step reads the bug list of a KO `verdict`); it is never passed to the run. The step's `with` overrides the rest, and `feature` is always the ticket key. A template that requires `stack` without a default gets the stack `aidp run init` detects in the workspace (`aidp project stack`), when the template allows that value.
- `wait`: polls a probe from the registry until it reports ready. The `timer` probe ships in this lot.
- `human`: creates an inbox entry `kind: "flow.human"` and waits (`waiting_human`). A decision resolves it through the API. `reject` escalates without advancing; `ko` remains a completed negative verdict.
- `loop`: runs a `body` of steps. Fields: `max`, `until`. A loop needs `max`, or a `human` step in its body.

### Loops

`until` is evaluated after each iteration (do-while). When it is true the loop is `done`. When `max` iterations ran and it is still false, the loop ends `escalated`. `escalated` means a human must look; it is distinct from `failed`, which is a technical error.

### Host publishing

No agent pushes in a flow that defines the `publish` block. Its pipelines receive `publish: host`, so their last stage only commits in Conventional Commits and writes what the host needs, such as the PR body. Each time a step with `pushes: true` finishes, the machine inserts a `publish@<step>` reference to the `publish` block, then the `ci@<step>` reference to the `ci` block when the definition declares one. The host therefore publishes the branch before anything else runs on it.

A delivery flow defines the block as two actions:

| Step | Action | Effect |
|---|---|---|
| `git_push` | `ticket.to_test` | `ticket label KEY to-test` | `key`, `label`, `changed` (label `aidp:to-test` and ticket phase `to-test`) |
| `git.push` | Publishes the branch as one commit GitHub signs. |
| `pr_create` | `gh.pr_create` | Opens the draft PR on the first publish; later publishes find it open and change nothing. |

`git.push` reads `origin`, refuses the repository default branch, and runs `git fetch origin --prune`. On a branch GitHub has, it commits on top of `origin/<branch>`. On a new branch, it creates the branch at `git merge-base HEAD origin/<default>`. It sends every file `HEAD` changes against that commit, whole and base64 encoded, through the GraphQL `createCommitOnBranch` mutation. That mutation runs through `gh api graphql`, with `expectedHeadOid` as an optimistic lock and the last local commit message. GitHub signs the commit. The worktree then runs `git fetch` and `git reset --soft origin/<branch>`, so it sits on the published commit with its uncommitted work kept.

| `git.push` case | Outcome |
|---|---|
| Nothing to publish | `changed: false`; the worktree lands on `origin/<branch>`. |
| `origin/<branch>` holds commits `HEAD` lacks | Refused (exit 4): publishing whole files would revert them. |
| A file over 45 MiB, or a payload over 80 MiB of base64 | Refused (exit 4). |
| Detached `HEAD`, or a worktree on another branch than the flow's | Refused (exit 4). |
| `git` or GitHub failure | Exit 5: the step fails and can be retried. |

A failed publish fails the run on the `publish@<step>/git_push` step. Retrying that step publishes again without rerunning the pipeline.

`gh.pr_create` titles the PR `<type>(<KEY>): <summary>` from the ticket: `fix` for a bug, `feat` otherwise. The key and summary come from the local note, else the flow's ticket snapshot. The body is the `pr-body.md` the pipeline wrote. The action writes `pr.json` next to that file and outputs its path as `pr`. That is the file a pipeline's `pr.json` was, so `{pr}`, `{url}`, `{repo}` and the flow view read the pull request as before.

A pipeline started by hand on a flow also gets `publish: host`. Set the manual request's `publish` boolean to `true` to publish its commits automatically after a `done` result. Omit it to retain the existing commit-only behavior.

A pipeline run outside a flow keeps `publish: agent`, so its last stage pushes and opens the PR as before.

## Conditions

Used by `when` and `until`. Operators: `==`, `!=`, `<`, `>`, `&&`, `||`, `!`. Literals: strings, numbers, booleans, null. Paths:

- `steps.<id>.outputs.<field>`
- `settings.*`
- `inputs.*`
- `ticket.*` (a plain object built by the executor)

### Step outputs of a pipeline step

When a `kind: pipeline` step ends, the daemon reads `result.json` in the stage directory of the last stage the run executed (a flat JSON object) and exposes each top-level key as `steps.<id>.outputs.<key>`, typed as in the file (booleans, numbers, strings, lists, objects). Without `result.json`, the `signals` of that stage's reply envelope are used. Shards and plan tasks (`<stage>#<id>`) are never the last stage.

| Key | Meaning |
| --- | --- |
| `verdict` | `done` or `need-fix` (review stages) |
| `passed` | boolean (test and import stages) |
| `report`, `summary`, `branch`, `findings` | as the stage wrote them |
| `pr` | pull request **number** |
| `blocked_reason` | why a stage stopped as `blocked`, from `result.json` or `signals.blocked_reason` |

The file outputs stay: `pr_file` (path of `pr.json`), `pr_body` (path of `pr-body.md`), `findings` and `verify` (paths of `findings.json` and `verify.json`), plus `runId` and `runDir`. A key of `result.json` wins over a file output of the same name and the path moves to `<key>_file` (`findings_file`, `verify_file`). `pr` was the path of `pr.json` before: it is now `pr_file`; `{pr}`, `{url}`, `{repo}` placeholders still read that file. `runId`, `runDir`, `sessionId`, `pr_file` and `pr_body` are never replaced. The `gh.pr_create` action keeps its own `pr` output (the path of the `pr.json` it wrote).

```yaml
- id: review-loop
  kind: loop
  max: 3
  until: steps.review.outputs.verdict == 'done'
  body:
    - { id: review, kind: pipeline, use: unity-review }
- id: fix-tests
  kind: pipeline
  use: unity-fix
  when: steps.test.outputs.passed == false
```

An unknown key is not an error: it depends on the runtime and reads as null.

### Templates in `with`

Every string in a step's `with` (nested lists and objects included) may hold `{{ path }}` placeholders, filled when the step starts from the same context as `when`: `steps.<id>.outputs.<key>`, `inputs.*`, `ticket.*`, `settings.*`. A string that is exactly one placeholder keeps the type of the value (`number: "{{ steps.fix.outputs.pr }}"` gives a number); inside longer text the value is written as text, and a missing key gives an empty text. This covers pipeline inputs and action bodies (`with.body`).

```yaml
- { id: note, kind: action, use: gh.pr_comment, with: { body: "Review {{ steps.review.outputs.verdict }} on #{{ steps.fix.outputs.pr }}" } }
```

## Validation

`aidp flow validate` refuses:

- duplicate ids;
- a `use` the registry does not know;
- a `when` or `until` reading `steps.<id>.outputs.<key>` of an unknown step (the key is not checked);
- a loop with no `max` and no `human` step in its body;
- a pipeline template that cannot be found (a warning only when the catalog is unreadable).

## Storage

Per ticket, under `<vault>/Pipelines/<repo>/<KEY>/flow/`:

| File | Content |
|---|---|
| `state.json` | The frozen definition (with its version), step states, outputs, iterations, timestamps, `effects_applied`. Written atomically, before each effect. |
| `journal.jsonl` | One line per transition: step, from, to, reason, actor, time. |
| `_flow.md` | Generated note. Never edit it. |

Index of active runs: `~/.aidp/gateway/flows/index.json`. No secret goes in `state.json` or the journal.

## Effects and restart recovery

Every effect has a deterministic id `<step>:<iteration>:<kind>`. The executor records applied ids in `state.json`; an effect already applied is never replayed.

At daemon start the executor reloads active runs and then:

- `pipeline` step: if the session ended meanwhile, its outcome is read and the run advances. Otherwise the step becomes `failed` with reason `orphaned`; it can be retried.
- `wait` step: the wait resumes.
- `human` step: the inbox entry is still there; approval advances the run.
- `done` steps are never replayed.

Wake-ups: bus events, the GitHub webhook and a 60 second timer.

## One writer per workspace

A flow step and a session share one admission lock per workspace. A manual session on the same worktree waits for the step to end. Runs for different tickets use distinct worktrees, so their `pipeline` steps run in parallel. Only one run per ticket can be active.

## Daemon resources

A `pipeline` step with `resource: unity-editor` holds the Unity editor lock of its project while its pipeline runs. The lock is the resource `unity-editor:<project id>`: one holder at a time, waiters served first in, first out. Projects do not block each other.

```yaml
- { id: implement, kind: pipeline, use: unity-implement, pushes: true, resource: unity-editor }
```

- The step takes the lock just before its pipeline starts. If another run holds it, the step becomes `waiting_external` with reason `resource:unity-editor` and the pipeline does not start.
- When the holder lets go, the first waiter is woken at once, retried and takes the lock. A `retry` or `skip` of a waiting step is accepted like for a step waiting for budget.
- The lock is released whenever the step stops running: success, failure, skip, abort, or a pause (the current step finishes, then the run stops). A waiting run that is aborted or skipped leaves the queue.
- The lock lives in the daemon's memory only. It is derived from the persisted runs: after a restart, every run whose `resource` step is still running holds the lock again, and no run waiting for it takes it first. A run whose session died is orphaned, so it holds nothing. A restart loses the order of the queue, not the lock.
- `POST /api/unity/lock/release` frees a lock by hand, for an orphan (see [gateway-api.md](gateway-api.md#unity-editor-lock-and-status)). The release is journaled on the holder's run. That run is not given the lock back while it runs.
- Each change publishes `unity.lock_changed`.

## Pause, abort, manual

- `pause`: the current step finishes, then the run stops.
- `resume`: continues a paused run.
- `abort`: cancels the running session. The worktree and the PR are kept.
- `manual`: runs one extra action or pipeline against the ticket outside the sequence. A pipeline request accepts optional `publish: true`; non-boolean values return HTTP 400.

## Events

`flow.started`, `flow.step_changed`, `flow.waiting_you`, `flow.ended`, and `unity.lock_changed` for the editor lock. Payloads are in [gateway-api.md](gateway-api.md#flows--lot-29a).

## CLI

```bash
aidp flow validate FILE     # local
aidp flow show FILE         # local
aidp flow list              # daemon
aidp flow start KEY [--flow ID]
aidp flow status KEY
aidp flow approve KEY STEP
aidp flow verdict KEY STEP --ok|--no [--report FILE]
aidp flow retry KEY STEP
aidp flow skip KEY STEP
aidp flow pause KEY
aidp flow resume KEY
aidp flow abort KEY
aidp flow unity-refresh [--timeout SECONDS]   # local, in the project checkout
```

`validate` and `show` run without the daemon. The others call it, as `aidp ticket start` does.

Retrying a failed `pipeline` step continues the previous attempt's native run when that run is paused: its failed and cancelled stages run again, the stages already done are kept, and the journal records `<step> resumed paused run <id>`. Otherwise the retry starts the pipeline from its first stage.

## Built-in actions

Each action runs an `aidp` verb that reads before it writes and prints `{..., changed}`.

An action's placeholders the step's `with` leaves out are filled from the run itself before it runs: `{branch}` from the `branch` step, `{body}` from the newest stored human report, `{pr_body}` from the newest `pr-body.md` a pipeline step wrote, and `{pr}`, `{url}`, `{repo}` from the newest `pr.json` a pipeline step of the run wrote, then from the live PR status of the ticket. `{sha}` is the live head commit when the live status names the same pull request as that `pr.json` (or when there is none), else the `pr.json` commit. The live status is read within 60 seconds; past that it counts as absent. A placeholder still missing fails the step as before. Exit 0 is `done`; 3, 4, 6 are `escalated`; anything else is `failed`.

| Action | Verb | Outputs | Rules |
|---|---|---|---|
| `git.worktree` | gateway worktree verb | `path`, `base_sha`, `reused` | per-project cap reached answers `waiting_external: true`, `reason: worktree_cap`: the step waits until retried |
| `git.branch` | `flow git-branch KEY` | `branch`, `created` | `fix/` for a bug, `feat/` otherwise, followed by the ticket key, not the flow execution key; commits by another author on the remote branch are escalated |
| `git.push` | `flow git-push --branch BRANCH` | `branch`, `sha`, `changed` | holds the workspace; one signed commit through the GitHub API, never `git push`; refusals are escalated, see [Host publishing](#host-publishing) |
| `gh.pr_create` | `flow pr-create KEY --branch BRANCH --body-file PATH --draft` | `pr`, `number`, `url`, `branch`, `changed` | reuses the open PR of the branch; writes `pr.json` next to the body file |
| `gh.pr_ready` | `gh pr-ready N` | `changed` | |
| `gh.merge` | `gh pr-merge N --sha SHA --report-conflict` | `merged`, `conflict` | behind the human gate; red gate or moved head is escalated. A PR whose gate is closed by a conflict with its base alone (or whose update onto the base conflicts) ends the step `done` with `merged: false`, `conflict: true`: the flow resolves the conflict and merges again, see [Merge conflict fallback](#merge-conflict-fallback). The ticket moves to `done` only when `merged` is true |
| `gh.issue_label` | `ticket label KEY STATE` | `key`, `label`, `changed`, `removed` | `with: { state }` (`in-progress`, `review`, `fixing`, `testing`, `awaiting-merge`, `done`...): replaces the single `aidp:*` state label of the ticket's issue; a no-op when it already shows it |
| `gh.issue_comment` | `ticket comment KEY --body-file PATH [--cost-footer]` | `key`, `url`, `costFooter` | `with: { body }` (the report file of a step); `with: { cost_footer: true }` appends the ticket's cost line (contract C7) when the usage source is wired, see below |
| `bug.create` | `flow bug-create KEY [--report-file PATH] [--body-file PATH] [--summary=TEXT] [--allow-extra]` | `bug`, `number`, `url`, `count`, `capped`, `changed`, `linked` | `with: { report, summary, body, body_dir, run_dir }`; `report` is text or a file name resolved under `run_dir`; `body` is the description written by the `bug-write` pipeline (a file name such as `bug.md` resolved under `body_dir`, else `run_dir`, or a path, or the text itself). Creates the bug issue (label `bug` and `bug:<route>`, title `fix(GH-n): summary`, body: the marker `<!-- aidp:bug parent=n base=feat/GH-n -->` first, then `body` (sections Issue de base, Description du bug, Plan à adopter, Critères d'acceptation), then the raw report in a folded `<details>` block; a marker inside `body` is dropped. `body` empty, unreadable or not found: the body is the one from before, marker, link to the parent, raw report and two generic criteria, and the ticket is created anyway), attaches it as a GitHub sub-issue, comments the parent. Reuses an open bug of the parent. At most 3 bugs per parent: the 4th answers `capped: true` and creates nothing (`allow_extra: true`, flag `--allow-extra`, lets one more bug through past the cap, for the doctor's `real_defect`). `flow bug-rearm KEY` forgives the bugs closed so far, once per 24 h: `rearmed`, `message` |
| `gh.issue_close` | `ticket close KEY` | `key`, `number`, `changed` | closes the issue; GitHub closes by itself only when a PR reaches the default branch, so a bug merged into its parent branch needs this |
| `worktree.remove` | gateway worktree verb | `removed`, `salvaged_ref` | |
| `unity.refresh` | `flow unity-refresh` | `ok`, `errors`, `warnings`, `duration_ms`, `request_id`, `blocked`, `message`, `heartbeat` | runs in the project checkout right after `git.checkout`: one compile gate (C1) through the playtest package's file bridge, so the editor's reimport of the new branch ends before the test stage. Skipped (`not-a-unity-project`, `unity-editor-closed`) when there is nothing to refresh; `blocked: true` when the editor never answers (600 s) or its heartbeat `.aidp/editor/heartbeat.json` is stale (30 s, 180 s during a domain reload). Compile errors are outputs, not a failure |

The pull request of a game flow closes its issue by itself: `gh.pr_create` writes `Closes #<n>` in the PR body, so the merge closes the issue and the next sync marks the ticket `done`. The flow definition of a ticket comes from its route: the dispatcher starts the `flow:` of the winning `repos:` entry of `routing.yaml` (see [tickets.md](tickets.md#routingyaml)); a ticket with no routed flow keeps `dispatcher.launch`.

The cost footer is produced by one isolated call point, `cost_footer_for_ticket(key)` in `crates/aidp-cli/src/cost_footer.rs`. It answers nothing until the per-ticket usage source (key `GH-<n>`) is wired, in which case the comment is posted as is.

## Built-in probes

A probe is polled in-process and returns `ProbeOutcome { satisfied, outputs, state }` from the PR status cache.

| Probe | Satisfied when | Outputs | Rules |
|---|---|---|---|
| `pr.reviews` | actionable threads remain after 10 quiet minutes, or the PR is approved with a green merge gate and no actionable threads | `new_threads`, `approved` | tracks thread and latest-comment identities; handled threads leave the pending batch |
| `pr.threads` | a fresh GraphQL thread snapshot is readable | `threads`, `clean` | excludes resolved and author-answered threads; outdated feedback remains actionable; unreadable snapshots remain pending |

## Repository settings

`.aidp/flow.json` overrides the defaults of the `flow` key in `~/.aidp/config.json`. Schema: [flow-repo.schema.json](../schema/flow-repo.schema.json).

```json
{
  "caps": { "worktrees": 3 }
}
```

`caps.worktrees` is the only key read: the maximum worktrees `git.worktree` holds at once for the
repository.

## Ticket delivery definition

The flow the daemon starts when a request names none is `perso-delivery`: the dispatcher with
`dispatcher.launch: flow`, `POST /api/tickets/:key/flow` without `flow`, and `aidp flow start KEY`
without `--flow`. The skill's `templates/flows/perso-delivery.yaml` holds it: worktree, branch,
`dev-unity` (pushes), `review-unity`, `fix-review-unity`, PR ready, human merge
(`merge_approval`, then `gh.merge` pinned on the head sha), worktree cleanup.

### Two lanes: development in a worktree, Unity test in series

A game ticket goes through two flows (decision of 2026-11-08, contract C13).

- **Dev lane**: the flow of the route (`flow:` in `routing.yaml`: `game-code`, `game-art`, `game-map`). Default workspace
  (a worktree per ticket), several runs in parallel up to `dispatcher.slots` (default 3). Its last steps are
  `ticket.to_test` then `worktree.remove`, and the run ends `done` with the ticket still `in-progress`: the issue
  carries the label `aidp:to-test` and the ticket the phase `to-test`. No Unity editor is open on a worktree: the
  `code` route verifies headless there (Unity in batchmode, compile gate C1 and EditMode suite C8, never PlayMode;
  `templates/recipes/unity-verify.md` of the skill); the `art` and `map` dev stages run no compile gate and no test.
- **Test lane**: the `test_flow:` of the same route. `game-test-code` (decision of 2026-11-09: PlayMode is manual)
  runs in the default workspace, a worktree of its own, with no editor and no `unity-editor` resource: headless
  `unity-test` ⇄ `unity-fix`, PR ready, label `awaiting-merge`, worktree removed, `done`; the human plays the PlayMode
  check-list of the PR comment and merges on GitHub, and the dispatcher moves the ticket to `done` when it sees the
  merged PR. Such a run occupies a dev slot (`dispatcher.slots`), not the `test_slots` lane. `game-test-art` and
  `game-test-map` keep `workspace: project`: the test checkout of the project (see below), where the editor is open,
  with the `unity-editor` resource. `dispatcher.test_slots` (default 1) such test flows at a time per project. The
  dispatcher starts the oldest `to-test` issue first (a manual start promotes one ahead), as soon as the lane of its
  project is free. A project test flow counts in the test lane while it holds, or waits for, the checkout; once it
  ran `git.restore` (the merge approval is pending) it no longer does. It never takes a dev slot. Pause, budget and
  night mode apply to both lanes.

**Test checkout.** A project may have a `testCheckout` (absolute path of a dedicated clone of the same repository,
for example on an SSD) in the registry `projects.json`. Set it with `aidp gateway projects test-checkout <project id>
<path>` (`--clear` removes it) or `PUT /api/projects/:id` with `{ "testCheckout": "<path>" }` (`null` clears it); it is
read back in `GET /api/projects`. When set, every `workspace: project` flow works there instead of in the project
path: `git.checkout` / `git.restore`, the pipeline sessions (`cwd`, project root, `.aidp/` folders), the editor
detection of `GET /api/unity/status` (`-projectpath`) and its compile and playtest results. The `unity-editor` lock
stays keyed by the project id. Without a `testCheckout`, they work in the project path as before. Dev flows
(worktrees of the project path) are not affected, and a session in the test checkout occupies the test lane only.
In that clone, `git.checkout` starts with `git fetch origin`, then takes the ticket branch from `origin/<branch>`
when it only exists there (a branch pushed from a dev worktree), fast-forwards a branch it already has, or creates
the branch from an up-to-date `origin/<base>`. `git.restore` switches back to the base branch and runs
`git pull --ff-only`. The test checkout is disposable and reserved to the test lane: no dev, art, map or doc flow
writes there (worktrees stay under the project path). Because the editor rewrites tracked files there,
`git.checkout` and `git.restore` never report `blocked: dirty-checkout` on it: they use `git switch --force` (local
changes are discarded, untracked `.meta` files in the way are removed) and keep the editor's
`ProjectSettings/ProjectVersion.txt` (saved before, written back after, then `git update-index --skip-worktree`) so the
editor asks for no migration. A project without `testCheckout` keeps the dirty-checkout check.

Ticket phase (`autopilot.phase`, exposed as `phase` and `branch` by `GET /api/tickets`): `dev` (default), `to-test`,
`testing` (set when the test flow starts). The label `aidp:to-test` and the phase go together: the flow action
`ticket.to_test` (= `aidp ticket label KEY to-test`) sets both; a human who puts the label on an issue (sync) or
`POST /api/tickets/:key/status` with `{ "status", "phase": "to-test" }` does the same. `aidp:testing` sets the phase
`testing`.

### How a delivery starts

- The dispatcher, when `dispatcher.launch` is `flow` (default `session`). It then starts the flow for each ticket it would otherwise launch as a session.
- Manually, with `POST /api/tickets/:key/flow`. This route ignores the dispatcher pause.

Only one run per ticket can be active. Every run started for a ticket moves its local ticket status and sets `autopilot.flow` to the flow id, whatever the flow; a key with no local ticket note is left alone. The status follows the steps `ready` (`in-review`) and `merge` (`done`).

### Restart recovery

The daemon reloads active runs at start and applies the rules of [Effects and restart recovery](#effects-and-restart-recovery). A delivery stopped at a `human` step keeps its inbox entry; a delivery stopped inside a loop resumes it at the recorded iteration.

### Ownership

A ticket with an active flow is owned by it. The session dispatcher skips it with the reason `owned-by-flow`. The flow state is the only source of ownership. When `dispatcher.launch` is `flow` and no flow engine is attached, the dispatcher skips the ticket with the reason `flow-unavailable`. See [adr/flow-owned-ticket-delivery.md](adr/flow-owned-ticket-delivery.md).

## Exit codes of actions

A technical failure (timeout, crash, usage fault, upstream error) sets the step `failed`. A business error the action declares sets it `escalated`. See [exit-codes.md](exit-codes.md).

## External hosts

A host can call `POST /api/flows` with an `executionKey` and a separate original `ticket.key`.
The frozen launch request makes retries idempotent across gateway restarts.
The ticket snapshot supplies project lookup and action context without requiring a local ticket note.
Worktrees and branches use the execution key; the PR is resolved from the original ticket key.
A flow policy applies to each pipeline step, including human questions and gate approvals.
With `policy.headless: true`, every session the flow starts runs without a permission prompt, retries and native stage sessions included: Claude runs `dontAsk` unless the policy names another mode.
A tool call the provider's allowlist does not grant is denied at once instead of waiting for a human; see the [host API contract](gateway-api.md#host-integration).
Native runs waiting at a gate remain active from the flow's perspective until a human resolves that gate.
See the [host API contract](gateway-api.md#host-integration).

## External executor

A host can launch a flow with `policy.executor: "external"` (see the [gateway API](gateway-api.md#external-executor)). The same definition runs; only who executes the non-agent steps changes.
The gateway keeps the step order, `when` and `until` conditions, loops, inserted blocks, human gates and escalation. It starts no action or probe subprocess.
The host reads the pending jobs, runs each one natively and posts the outcome; the machine applies it as if the built-in action had returned it.

| Runs in the gateway | Becomes an external job |
|---|---|
| Agent steps (`kind: pipeline`), human gates, `noop`, `fail`, the `timer` probe | Every other action or probe in the [built-in actions](#built-in-actions) and [probes](#built-in-probes), and manual actions |

A name the host does not know is answered `technical_error` with reason `unsupported_external_action`.

With `policy.externalAgents: true` the host also runs the agent sessions of the `pipeline` steps. The gateway still starts the native run, keeps its stages, loops, gates and scope guard, and checks each reply.
It publishes every session as an `agent` item of the same listing and waits for the host to claim it, renew its lease and post the result. See [agent jobs](gateway-api.md#agent-jobs-run-by-a-host) for the routes, the job and result bodies and the lease rules.
A flow without the policy, or on a gateway without `capabilities.externalAgents`, launches its sessions itself.

The host returns the outputs listed in the built-in tables, because later steps read them. The ones a delivery flow reads:

| Name | Outputs the flow reads |
|---|---|
| `gh.pr_create` | `pr` (path of the `pr.json` the host wrote), `number`, `url`, `branch` |
| `git.branch` | `branch` |
| `git.push` | `branch`, `sha`, `changed` |
| `pr.threads` (probe) | `threads` (actionable count), `clean` (`threads == 0`) |
| `pr.reviews` (probe) | `new_threads`, `approved`; answer `pending` with a `state` while the quiet period runs |

`business_error` escalates the step, as exit codes 3, 4 and 6 do for a built-in action. A `waiting_external` action stays listed until the host answers with another outcome.

## Pipeline doctor

A flow that ends `failed` or `escalated` (not by an exhausted loop or a human), a paused run, a step
stuck `running` or waiting external too long opens an incident of the daemon's doctor: it runs the
`flow-doctor` investigation, may retry the step or requeue the ticket, and comments on the issue.
Its lines are journaled as actor `doctor`. See [doctor.md](doctor.md).

## Merge conflict fallback

`gh.merge` no longer fails on a conflict with the base: it answers `conflict: true` (`merged: false`,
the reason in `detail`) when the only thing between the PR and its merge is that conflict, either
GitHub's `mergeable: CONFLICTING` with every other gate item open, or an update of a behind branch
that neither the rebase nor the merge of the base could apply. A red gate for any other reason, a
moved head or a disabled squash still fail the step as before.

The game flows of the skill wrap the merge in a loop that turns this into a fallback:

```yaml
- id: merge_loop
  kind: loop
  phase: Merge
  max: 2
  until: "steps.merge.outputs.merged == true || !(steps.merge_loop.iteration < settings.max_merge_rounds)"
  body:
    - { id: merge_fix, kind: pipeline, use: merge-conflict, pushes: true, when: "steps.merge.outputs.conflict == true" }
    - { id: merge, kind: action, use: gh.merge }
- { id: merge_blocked, kind: action, use: gh.issue_label, with: { state: blocked }, when: "steps.merge.outputs.merged != true" }
- { id: merge_escalate, kind: action, use: fail, with: { kind: business, message: "..." }, when: "steps.merge.outputs.merged != true" }
```

The `merge-conflict` pipeline (one stage on Sonnet at low effort) merges the base into the PR branch
in its own temporary worktree, resolves the conflicted files, pushes, and never touches the flow's
checkout. The next `gh.merge` pins the new live head. A conflict still standing after
`max_merge_rounds` labels the issue `blocked` and ends the run `escalated`, as a failed merge did.
The step id stays `merge`, so the ticket status and the `flow.merged` event follow it; both ignore a
`merge` step that ended on a conflict.

## See also

- [gateway-api.md](gateway-api.md) for routes
- [adr/loc-73-flow-engine.md](adr/loc-73-flow-engine.md) for the design decision
- [adr/flow-owned-ticket-delivery.md](adr/flow-owned-ticket-delivery.md) for flow ownership of a ticket
- [adr/flow-actions-subprocess-and-probe-outcomes.md](adr/flow-actions-subprocess-and-probe-outcomes.md) for actions and probes

### Automatic review corrections

A delivery flow reads the current review threads (`pr.threads`, `pr.reviews`) before running a correction pipeline, and rechecks after publishing.
The host publishes each correction through `git.push` and `gh.pr_create` and reuses the open PR.

Thread reads paginate GraphQL `reviewThreads` and include resolution, outdated status, authors and the latest reply.
A root review comment remains actionable even when you authored it. A subsequent reply from the PR author or authenticated host marks it handled.
A later reviewer reply makes it actionable again. These response rules leave the merge gate's unresolved-thread count unchanged.
Missing or incomplete thread data never means a clean PR. Flow runs resolve the PR using the ticket key, independently of their execution key.

### Manual publishing and refusals

Manual pipeline requests default to `publish: false`. With `publish: true`, the run persists the label in `manual_publish`.
After `done`, it emits `git.push` as `manual-<n>-publish`. Failed, cancelled or orphaned pipelines only record their result.
Publishing consumes the label, so duplicate completion events never publish again. A branch without new changes creates no commit.

The journal records `manual pipeline <effect-id> ended <status>` and `manual action git.push (manual-<n>-publish)`.
An occupied workspace records `manual-<n> not started: workspace busy`; the request is not queued or retried.
Manual action failures record `manual effect <effect-id> failed: <reason>`. Budget refusals retain their existing `not started: budget` line.
Manual publishing never merges. The original manual request retains the caller's actor in the journal.

### Rejected gates

A `reject` decision escalates the human step and flow with reason `rejected` and detail `rejected by <actor>`.
The decision, actor and time remain in the step outputs. Following steps never run, including on gates with `on_error: warn`.
Retry creates a fresh inbox entry and waits for another decision. Skip returns a conflict for a rejected gate.
These rules also apply to frozen definitions and expanded block step IDs. A `ko` test verdict still advances to the correction path.

### Bug tickets raised by a failed test

`game-test-code`, `game-test-art` and `game-test-map` (version 2) no longer loop on a fix. When `steps.test.outputs.passed == false`, `bug.create` opens a bug issue under the ticket and `ticket.to_test` leaves the ticket in the `to-test` phase. The run ends `done`, not failed.

- The sync reads the marker in the bug body: the bug mirror gets `github.parent` and `github.base_branch`, and the parent gets the open bug as a blocker. The test lane plan skips a `to-test` ticket with an open blocker (`waiting_on`). The native GitHub dependency is not created.
- The flows run the read-only pipeline `bug-write` (`on_error: warn`) before `bug.create`: it reads the parent issue, its PR, the test report and the project memory, then writes `bug.md` and the one-line title. If it fails, `bug.create` still raises the bug with the report alone. The bug flows (`game-bug-*`) read these sections, and their CA-n are the criteria of the review.
- The bug takes the route of the parent through the label `bug:code|art|map` (`routing.yaml`, flows `game-bug-code|art|map`). Its branch `fix/GH-<bug>` starts from `origin/feat/GH-<parent>`; the pipelines receive the input `base_branch`, and `flow git-branch`, `git-push` and `pr-create` use it. The PR targets the parent branch, never `main`.
- The bug flows merge on their own once the review verdict is `done` (`gh.pr_ready`, `gh.merge`, `gh.issue_close`). Without a `done` verdict the bug is labelled `blocked` and nothing is merged.
- When the bug closes, the sync (or `ticket close`, which also marks the blocker closed on the parent mirror) frees the parent, and the dispatcher starts the test flow again.
- Cap: 3 bugs per parent. Past it the parent is labelled `blocked`, commented, and the run ends `escalated`.
- The final PR of the parent toward `main` stays a human decision.
