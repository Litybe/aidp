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
| `run_paused` | A native run is `paused` (needs review, blocked, reply rejected, write out of scope). A `flow-doctor` run never counts. |
| `step_stuck` | The current step is `running` while its native run or session ended, is gone, or never started, for over `stuck_minutes`; or an action step is `running` with no worker executing it for that long. |
| `waiting_too_long` | The current step is `waiting_external` for over 30 minutes (budget, worktree cap, Unity lock...). Action jobs of an external executor are left to their host. |

Paused, pausing, done and aborted flows are left alone.

## The doctor's thread

The doctor runs on its own thread, started after the flow engine. Its first pass waits 30 seconds,
then it passes every 60 seconds. It is never called from `reconcile_all`, which also runs at
startup before the HTTP server: detection reads `state.json`, `runtime.json` and the index from
disk, and the live runs from memory, and takes no run lock. An action calls the flow engine like
any API caller; the doctor never holds a run's lock when it does.

Each pass:

1. Detects the broken flows and runs and opens the new incidents (`detected`).
2. Follows the running investigation, if any. There is never more than one.
3. Otherwise starts the oldest `detected` incident that is still broken. One that recovered by
   itself closes `no_action`.

Before starting an investigation:

- **Cap**: a ticket gets at most `max_attempts_per_ticket` investigations over a sliding 24 hours
  (default 2). Past it the incident closes `capped` and is reported.
- **Budget**: the investigation is held to the budget guard (`flow-doctor` template). While the
  budget refuses, the incident stays `detected`, with the note `investigation waits for the token
  budget` journaled once.
- **Slots**: the investigation is a native run started by the doctor, not by the dispatcher: it
  takes no dev or test slot.

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

`flow-doctor` runs `prepare` (haiku), `diagnose` (sonnet), `report` (haiku). `diagnose` writes
`diagnosis.json`:

```json
{ "category": "false_failure", "cause": "…", "evidence": [{"file": "…", "at": "…", "line": "…"}],
  "confidence": 0.85, "action": "retry_step", "params": {"step": "report"},
  "needs_human": false, "proposed_patch": null, "summary": "…" }
```

`category` is one of `false_failure`, `run_ended_flow_running`, `unity_reimport_not_frozen`,
`unity_modal_dialog`, `budget_wait_silent`, `worktree_cap_saturated`, `dead_flow_ownership`,
`meta_add_add_conflict`, `rebase_conflict_only`, `head_moved_before_merge`, `harness_defect`,
`product_bug`; anything else reads `other`. A harness defect carries a unified diff in
`proposed_patch`; it is stored with the incident and never applied.

A failed, paused or cancelled investigation, or one without a readable `diagnosis.json`, closes the
incident `investigation_failed`. Nothing restarts it, and a `flow-doctor` run is never an incident:
a failed doctor never calls a doctor.

## Actions

The agent proposes; the daemon decides and executes.

1. An `action` outside the list below is refused (`refused`, the name in the note).
2. `needs_human: true`, or a `confidence` below 0.7: no action (`needs_human`).
3. `none`: no action (`no_action`).
4. Otherwise the action is claimed under the idempotency key `<incident id>:action`, written
   before it runs: an incident gets one action, even across a restart. Then its precondition is
   checked again by the daemon; a failed precondition is `refused`, nothing changed.

| Action | Does | Precondition |
|---|---|---|
| `retry_step` | `retry` of the step (`params.step`, else the incident's), as `doctor` | The step is the current one, not a `human` step and not a `gh.merge` step. A step still `running` is reconciled first; still running after that, the action fails. |
| `requeue_to_test` | `aidp ticket label KEY to-test` | No active flow owns the ticket. |
| `requeue_dev` | `aidp ticket label KEY ready` | No active flow owns the ticket. |
| `abort_dead_flow_and_requeue` | `abort` of the flow, then `to-test` for a project-checkout (test lane) flow, else `ready` | The flow is terminal (`failed`, `escalated` or `aborted`). |
| `update_pr_branch` | `gh pr update-branch N --repo R` (merge, GitHub's default) | A PR number and repository (`params.pr`, `params.repo`, else the flow's `pr.json`); `rebase` or `force` in the params is refused. |
| `release_unity_lock` | Frees `unity-editor:<project>` (`params.resource` to name another) | Someone holds it and the holder's flow is terminal. |
| `none` | Nothing. | |

The doctor never merges a PR, approves a human step, force-pushes, edits the game's code or
assets, edits or installs the harness, or opens Unity.

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

`detected`, `investigating`, then one of `fixed` (the action succeeded), `action_failed`,
`needs_human`, `refused`, `no_action`, `investigation_failed`, `capped`.

## Settings

`TODO/config.yaml`, read at every pass:

```yaml
doctor:
  enabled: true                  # false: no detection, no investigation
  stuck_minutes: 10              # a running step with no live run for this long is stuck
  max_attempts_per_ticket: 2     # investigations per ticket over 24 h
```

Incidents are kept in `<state dir>/doctor/incidents.json` (`~/.aidp/gateway/doctor/`).
