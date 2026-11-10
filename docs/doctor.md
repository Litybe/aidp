# Pipeline doctor

When a flow or a run breaks, the daemon opens an incident, runs the `flow-doctor` investigation,
executes an allowed action when it is safe, and reports. The doctor treats failures of the
pipeline. A test that returns `passed: false` is a failure of the product: the flow's correction
loop handles it, not the doctor.

Code: `crates/aidp-gateway/src/doctor/` (`detect`, `evidence`, `policy`, `actions`, `store`,
`http_routes`). Template: the skill's `templates/flow-doctor.yaml`.

## Triggers

One incident per ticket and cause. The id is `<key>/<step>@<iteration>` for a flow step, so the
flow failure and the paused run of that step are one incident; a run outside any flow is
`<ticket>/run:<run id>`.

| Trigger | When |
|---|---|
| `flow_failed` | A flow is `failed`. |
| `flow_escalated` | A flow is `escalated`, except by `max-iterations` (exhausted loop), `rejected` (a human said no), `ticket-budget`, `aborted` or `manual-skip`. |
| `run_paused` | A native run is `paused` (needs review, blocked, reply rejected, write out of scope) **and live**: it is the run of the current step of a flow that is not over, or it belongs to no flow and is at most 24 h old. A run replaced by a newer run of the same ticket and template does not count. A `flow-doctor` run never counts. |
| `step_stuck` | The current step is `running` while its native run or session is gone or never started, for over `stuck_minutes`; or its run **ended** for over a grace of 3 minutes (and the doctor itself has seen it for that long, over several ticks: the flow engine has time to reconcile); or an action step is `running` with no worker executing it for that long. |
| `waiting_too_long` | The current step is `waiting_external` for over 30 minutes (budget, worktree cap, Unity lock...). Action jobs of an external executor are left to their host. |
| `bugs_not_converging` | A flow failed on its `bug_capped_stop` step. See [Bugs that do not converge](#bugs-that-do-not-converge). |

Paused, pausing, done and aborted flows are left alone.

**Live only.** Detection reports what is broken now. Flows that are going (`running`,
`waiting_external`, `waiting_human`) come first, the most recent first, then the rest by recency. An
incident still `detected` that is no longer a live failure (run replaced, flow over, run too old, or
recovered) closes at the next pass without investigation: `no_action` with the note `obsolete: …`
(no new status: the front does not know one), or `fixed` when the doctor's own action cured it.

**The art pipeline is not the doctor's.** The doctor never fixes sprites: no generation, retouch or
asset edit is proposed or started, and none of its actions does one. A run of an `art-*` or
`pixelart` template stopped on art findings (for example `correction did not converge`) is not an
incident, and neither is a flow that failed on one. Only a passing fault (see below) on an art run
makes an incident, and the only thing the doctor does is resume that run.

## The doctor's thread

The doctor runs on its own thread, started after the flow engine. Its first pass waits 30 seconds,
then it passes every 60 seconds. It is never called from `reconcile_all`, which also runs at
startup before the HTTP server: detection reads `state.json`, `runtime.json` and the index from
disk, and the live runs from memory, and takes no run lock. An action calls the flow engine like
any API caller; the doctor never holds a run's lock when it does.

Each pass:

1. Detects the broken flows and runs and opens the new incidents (`detected`).
2. Follows the running investigation, if any. There is never more than one.
3. Closes the `detected` incidents that are obsolete (see above), repairs the passing faults without
   a model (next section), then starts the investigation of the first live `detected` incident.
   There is still never more than one investigation at a time.

Before starting an investigation:

- **Cap**: a ticket gets at most `max_attempts_per_ticket` investigations over a sliding 24 hours
  (default 2). Past it the incident closes `capped` and is reported.
- **Budget**: the investigation is held to the budget guard (`flow-doctor` template). While the
  budget refuses, the incident stays `detected`, with the note `investigation waits for the token
  budget` journaled once.
- **Slots**: the investigation is a native run started by the doctor, not by the dispatcher: it
  takes no dev or test slot.

## Passing faults: repaired without a model

Before any investigation, the daemon reads the pause reason of the run, the reasons of its stages
and its small stage replies (`.json`, `.md`, 64 KiB at most). A match is a passing fault and the
incident goes straight to the action `retry_step`, which resumes the paused run where it stopped
(its finished stages are kept; the flow's `retry` does the same through `resume_paused`). A run
outside a flow is resumed the same way.

| Fault | Category | Recognised by |
|---|---|---|
| MCP server did not connect | `mcp_connect_timeout` | `CONNECT_TIMEOUT`, `MCP server … connection timed out` |
| Unity compile gate | `unity_compile_timeout` | `compile` + `gate` or `unity` + `timed out` / `timeout` |
| Agent session lost | `session_lost` | `session lost`, `session expired`, `session not found`, `no conversation found` |
| Provider overloaded or rate limited | `api_overloaded` | `overloaded`, `rate limit`, `usage limit`, `too many requests`, `HTTP 429` |

The wait grows between two resumptions: none before the first, 2 min before the second, doubling
after. At most 2 resumptions per incident; the incident then climbs the escalation below. These
repairs do not start an investigation, so they never consume `max_attempts_per_ticket`.

## Escalation

An incident takes several actions, 3 at most, whoever proposed them, in this order:

1. resumption (`retry_step`), twice at most;
2. requeue, to the test lane (`requeue_to_test`) or the dev lane (`requeue_dev`) according to the
   step (a project-checkout flow, or a step or template named for tests, is the test lane);
3. bug ticket (`create_bug_ticket`), for a `product_bug` only, which goes first for that category;
4. a human (`needs_human`): nothing else is left, or the three actions are spent.

A rung already taken is skipped. A `harness_defect` has no rung: it goes to `open_harness_pr` or to
a human. An action fails or is refused: for a reversible action or a bug ticket the next rung runs
under a new idempotency key, `<incident id>:action:<n>`, written before it runs. The investigation
cap (`max_attempts_per_ticket`, sliding 24 h) counts investigations only: resumptions of a passing
fault and the actions that follow do not count in it, and one investigation still yields at most 3
actions.

## Investigation

The daemon writes the evidence bundle, then starts `flow-doctor` (autonomous, headless, every gate
approved) with the bundle as its workspace:

```
<vault>/Pipelines/<repo>/<KEY>/doctor/<incident>/      (Pipelines/doctor/<incident>/ outside a flow)
  evidence/incident.json          the incident and the list of copied files
  evidence/flow-state.json        state.json of the flow
  evidence/flow-journal.jsonl     journal of the flow
  evidence/flow-runtime.json      sessions, waits, inbox of the flow
  evidence/run-state.json         of the run behind the step, when there is one
  evidence/native-run.json
  evidence/stages/<stage>/        replies and results (.json, .md, 64 KiB at most each)
  evidence/daemon.err.log.tail    last 200 lines of the daemon's log naming the key, the ticket
                                  or the run, or carrying WARN or ERROR (last 256 KiB read)
  evidence/pr-status.json         live PR status of the ticket (60 s at most), else null
  evidence/unity-status.json      body of GET /api/unity/status for the project
  comment.md                      the comment the daemon posted
```

`flow-doctor` runs `prepare` (haiku), `diagnose` (opus), `report` (haiku). Its instruction is
oriented to action: act, `none` only with a written reason, `needs_human` only when no allowed action
can help; a failed test on a ticket is a `product_bug` and gets `create_bug_ticket`. `diagnose` writes
`diagnosis.json`:

```json
{ "category": "false_failure", "cause": "…", "evidence": [{"file": "…", "at": "…", "line": "…"}],
  "confidence": 0.85, "action": "retry_step", "params": {"step": "report"},
  "needs_human": false, "proposed_patch": null, "summary": "…" }
```

`category` is one of `false_failure`, `run_ended_flow_running`, `unity_reimport_not_frozen`,
`unity_modal_dialog`, `budget_wait_silent`, `worktree_cap_saturated`, `dead_flow_ownership`,
`meta_add_add_conflict`, `rebase_conflict_only`, `head_moved_before_merge`, `harness_defect`,
`product_bug`, `mcp_connect_timeout`, `unity_compile_timeout`, `session_lost`, `api_overloaded`;
anything else reads `other`. A harness defect carries a unified diff in `proposed_patch`; with
`open_harness_pr` it becomes a pull request, otherwise it is stored with the incident and never
applied.

A failed, paused or cancelled investigation, or one without a readable `diagnosis.json`, closes the
incident `investigation_failed`. Nothing restarts it, and a `flow-doctor` run is never an incident:
a failed doctor never calls a doctor.

## Actions

The agent proposes; the daemon decides and executes.

1. An `action` outside the list below is refused (`refused`, the name in the note).
2. `needs_human: true`, or a confidence below the bar of the action: no action (`needs_human`). The
   bar is **0.5** for the reversible actions (`retry_step`, `requeue_to_test`, `requeue_dev`) and
   **0.7** for the others.
3. `none` with a written cause: no action (`no_action`). `none` with an empty cause is `needs_human`:
   the instruction of `flow-doctor` allows `none` only with a reason, and `needs_human` only when no
   allowed action can help.
4. Otherwise the action is claimed under the idempotency key `<incident id>:action:<n>` (n from 1,
   one per attempt), written before it runs. Then its precondition is checked again by the daemon;
   a failed precondition is `refused`, nothing changed.

| Action | Does | Precondition |
|---|---|---|
| `retry_step` | `retry` of the step (`params.step`, else the incident's), as `doctor`; the paused run of the step is resumed where it stopped. For a run outside a flow, the paused run is resumed. | The step is the current one, not a `human` step and not a `gh.merge` step. A step still `running` is reconciled first; still running after that, the action fails. |
| `requeue_to_test` | `aidp ticket label KEY to-test` | No active flow owns the ticket. |
| `requeue_dev` | `aidp ticket label KEY ready` | No active flow owns the ticket. |
| `abort_dead_flow_and_requeue` | `abort` of the flow, then `to-test` for a project-checkout (test lane) flow, else `ready` | The flow is terminal (`failed`, `escalated` or `aborted`). |
| `update_pr_branch` | `gh pr update-branch N --repo R` (merge, GitHub's default) | A PR number and repository (`params.pr`, `params.repo`, else the flow's `pr.json`); `rebase` or `force` in the params is refused. |
| `release_unity_lock` | Frees `unity-editor:<project>` (`params.resource` to name another) | Someone holds it and the holder's flow is terminal. |
| `create_bug_ticket` | Raises a bug issue under the ticket with `bug.create` (`aidp flow bug-create KEY`): same marker `<!-- aidp:bug parent=… base=… -->`, same cap of 3 bugs per parent, the parent waits while the bug is open. | The category is `product_bug` and the incident has a ticket. A parent that already holds 3 bugs: `refused`. |
| `open_harness_pr` | Opens a pull request for a harness patch: temporary worktree from `origin/main` of `Litybe/aidp` or `Litybe/aidp-skill` (named by `params.repo`, else by the paths of the patch), patch applied, branch `doctor/<incident>` pushed (no force), `gh pr create`. Never a merge, never a `gamedev/*` branch. | A `proposed_patch` that names files inside the repository, a local clone (`AIDP_SOURCE_ROOT` for `Litybe/aidp`, the skill root for `Litybe/aidp-skill`), and no PR already open for the incident (one per incident). A patch that does not apply: nothing is pushed, the note says so, the incident is `needs_human`. |
| `none` | Nothing. | |

The doctor never merges a PR, approves a human step, force-pushes, edits the game's code or
assets, edits or installs the harness on the machine (a harness patch only travels as a pull
request for a human to review), generates, retouches or edits a sprite, or opens Unity.

## Bugs that do not converge

A flow escalated on its `bug_capped_stop` step (3 bugs raised under one parent, the issue is
blocked) opens an incident with the trigger `bugs_not_converging`. Its id is
`<key>/bug_capped_stop@<iteration>.<flow start ms>`: one investigation per cap reached. It does not
count in `max_attempts_per_ticket` and is not held back by it.

The daemon computes the evidence before the run starts, in the bundle (`evidence/`):

| File | Content |
|---|---|
| `bug-history.json` | Per `fix/GH-*` bug merged into `feat/<parent>`: commit, subject, commits, files with added and deleted lines. |
| `test-infra.json` | Files under `Assets/Tests/Playtest/**`, `Packages/**` and `ProjectSettings/**` that the branch changed against `origin/main`: status, `before_bytes`, `after_bytes`, `suspect` (deleted, or shrunk by 30 % or more). |
| `pr-lag.json` | `behind_by`, `mergeable`, `conflicts`, `pr_mergeable`. |
| `shared-patterns.json` | A normalized error found in the test reports of at least 2 different tickets over 24 h (a bug counts for its parent): `key`, `sample`, `tickets`, `runs`. |

Categories and actions it adds:

| Category | Action | Does |
|---|---|---|
| `test_infra_regression` | `restore_test_infra` | Restores the files (`params.files`, else the suspect ones) from `origin/main` in a temporary worktree, commits as `aidp-doctor`, pushes normally to `feat/<parent>` (never `main`, never a force). Refused for a path outside the three folders or a file the branch did not change. |
| `branch_behind_main` | `update_pr_branch` | Update by merge. |
| `shared_harness_failure` | `open_harness_pr`, else `create_bug_ticket` | A patch when it is safe; otherwise a bug on the issue `doctor.harness_ticket`, never under a game ticket. One incident covers every ticket of `params.tickets`: the other tickets' incidents close `no_action` (`covered by incident …`) and the comment is posted on each. Without `harness_ticket` the bug is refused. |
| `real_defect` | `create_bug_ticket` | One more bug past the cap (`--allow-extra`), only if `params.defect`, `params.file`, `params.criterion` and evidence are given and the cause differs from every earlier bug on that file. Otherwise `needs_human`, with a note « défaut : …; fichier : …; critère : … ». |

After `restore_test_infra`, `update_pr_branch` or `open_harness_pr` the bugs closed before the stop
no longer count for the parent (`aidp flow bug-rearm KEY`, state `bug_rearm` in the note of the
ticket): one re-arm per parent per 24 h. After a restore or an update, a terminal flow is aborted
and the ticket goes back to the test lane to rerun the test; a flow still going is left alone. If
the re-arm is refused, the fix and the rerun still happen.

## Report

On every closing status the daemon:

- posts one short comment on the ticket's issue through `aidp ticket comment KEY --body-file`:
  cause, action and its detail, status and note, followed by the `report.md` of the report stage
  when there is one. `commented` records whether it was posted;
- journals `doctor: <what happened>` on the flow, as actor `doctor`;
- publishes `doctor.incident` on the bus with the incident. It is also published when the incident
  opens and when its investigation starts.

`GET /api/flows/:key/doctor` and `GET /api/doctor` read the incidents
(see [gateway-api.md](gateway-api.md#pipeline-doctor)). aidp-studio shows them in a « Docteur »
section of the flow page.

## Statuses

`detected` (also while a passing fault waits between two resumptions), `investigating`, then one of `fixed` (the action succeeded, or the incident recovered after one), `action_failed`,
`needs_human`, `refused`, `no_action` (also: obsolete incident), `investigation_failed`, `capped`. An incident carries its
`actions` (3 at most) beside `outcome`, the latest one.

## Settings

`TODO/config.yaml`, read at every pass:

```yaml
doctor:
  enabled: true                  # false: no detection, no investigation
  stuck_minutes: 10              # a running step with no live run for this long is stuck
  max_attempts_per_ticket: 2     # investigations per ticket over 24 h (passing-fault repairs not counted)
  harness_ticket: GH-9           # issue that receives the bugs of a harness failure shared by several tickets
```

Incidents are kept in `<state dir>/doctor/incidents.json` (`~/.aidp/gateway/doctor/`).
