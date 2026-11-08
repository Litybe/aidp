---
aidp: doc
title: Native execution plan
tags: [aidp/doc]
---

# Native execution plan

## Purpose

The Workflow emitter makes every routing decision of a pass: which stages run, in what order, which run in parallel, which are skipped. These decisions now live in a pure, typed plan with no JavaScript in it, built by `aidp_core::exec_plan::compile_exec_plan(pipeline, state, plan_source)`. `workflow.js` is a byte-identical rendering of that plan, guarded by `crates/aidp-core/tests/workflow_emit_golden.rs`.

Workflow-mode drivers see no change. The plan exists so a future native executor can walk the same decisions instead of re-deriving them.

## ExecPlan shape

The plan is an ordered list of steps. Each step has one kind:

| Kind | Meaning |
|---|---|
| `stage` | One sub-agent dispatch. |
| `parallel` | Adjacent stages with no `needs` between them. |
| `loop` | A correction loop: entry condition, `max_iterations`, body steps. |
| `fanout` | One sub-agent per item, plus a merge. Items are static, or a `from_signal` expression read at runtime. |
| `plan_tasks` | A plan-driven stage. It carries the `plan.json` source; tasks are still expanded by `plan.rs::schedule` once the plan exists. |
| `composite_boundary` | The point where a `type: pipeline` forces a new pass. |
| `skipped` | A stage that will not run, with a reason enum. |

Conditions reading only inputs are resolved at compile time. Conditions reading a stage output stay expressions. The plan may end with a pass end (`awaiting_plan` or `awaiting_composite`).

The plan is pass-scoped and recomputed on every emit. It is never stored in run-state. Its schema is `schema/execution-plan.schema.json`.

## Inspecting a plan

`aidp run plan-exec --run <run_id>` writes `<run_dir>/execution-plan.json` and prints its path.

## How the native executor walks it

The daemon runs a v7 run without a Claude Workflow session. The module `aidp-gateway/src/native_runs/` reads the steps of `compile_exec_plan` in order, stops at the pass end, recompiles, and continues. It never re-implements routing.

### Stage sessions

Stages run on Claude Code, the only built-in provider. `provider`, `providerByRole` and the
template provider fields still select a provider by id; see [providers.md](providers.md).
Set `mcpConfigId` to a registered runtime configuration to pass explicit MCP servers to each stage.
Only the reference is persisted; the host supplies definitions and credentials in memory before recovery.

Each stage is one daemon session: engine `stage`, origin `{kind: run, ref: <run_id>, label: <stage>}`. The prompt is the file form of `build_prompt_text`. The agent writes its reply to `<stage_dir>/reply.md`. The executor then checks that the reply:

- validates against `agent-reply.v4`;
- follows the reviewer rules, for a reviewer stage;
- names every declared output, and each file exists.

On a failed check the stage is relaunched once with the error. A second failure marks the stage `failed`.

### Stage session permissions

A stage session runs unattended, so nothing can answer a permission prompt: the daemon never reads `permissions.json` (it only writes it for the autonomous profile) and no component answers claude's permission requests. What a session may do is decided at launch.

- Every stage session is launched with `--permission-mode bypassPermissions` and `--add-dir <run directory>`, so the stage directories stay inside the directories claude may write.
- claude reports the mode it really runs in. A managed policy can forbid bypass (`permissions.disableBypassPermissionsMode: "disable"`); claude then starts in `default` and denies every write and shell command it cannot prompt for, so a stage ends with `reply.md is missing`.
- When the reported mode is not the one asked for, the daemon stops that session and relaunches it with `--permission-mode auto`, the unattended mode the managed policy's own `autoMode` rules govern. Every later stage session of the run asks for `auto` directly. The stopped session stays in the session list, `cancelled`.
- A stage that still fails names both modes in its reason (claude ran the session in one permission mode instead of the other), so the cause is not lost behind the missing reply. `auto` can itself be unavailable (plan, model or administrator setting); the run then fails the same way.
- The mode claude reported is kept as `effectivePermissionMode` on the session record, next to the `permissionMode` that was asked.
- A headless run (`headless: true`) asks `dontAsk` when its mode is absent or `default`, and keeps any other explicit mode. It never asks for bypass, so the fallback above never applies; a reported mismatch fails the stage like any explicit mode.
- Headless Claude sessions get a denial note appended to their system prompt (`--append-system-prompt`, after any role persona). It says a denial concerns one exact command, and tells the agent to retry without an `rtk` prefix, split compound commands or use Read, Grep, Glob, Edit or Write before reporting a stage blocked. Interactive sessions do not get it.
- Headless stage sessions get `--add-dir <run directory> <vault root>`, so knowledge notes and stage artefacts can be read without approval. Under `dontAsk`, `--add-dir` grants read access only: a write to a stage artefact in the run directory or the vault needs an `Edit` allow rule in the Claude settings allowlist; [gateway-api.md](gateway-api.md#host-integration) lists what the host grants. Claude denies any other call, edits included, that its settings allowlist does not grant.

### Semantics

| Step | Behavior |
|---|---|
| sequence | Stages run in order. |
| `parallel` group | Members run concurrently, capped by `settings.max_parallel` (default 3). |
| `when` | A false condition skips the stage, with the reason recorded. |
| `loop` | The condition is re-evaluated after each body pass, up to `max_iterations`. |
| `fanout` | One session per item, then the merge. |
| reviewer | Soft pause: blocking findings are a signal, never a halt. Findings are merged as in workflow mode. |
| gate | The run becomes `waiting_gate` and an inbox entry appears. Approve through the API, or pre-approve with `autoApproveGates`. |
| `composite_boundary` | Recompiles the plan and continues. |
| `plan_tasks` | One session per task. Tasks with disjoint write sets run in parallel, capped by `min(plan_tasks.max, settings.max_parallel)`; the others run in plan order. See [adr/native-plan-task-parallelism.md](adr/native-plan-task-parallelism.md). |

### Plan task scope and failures

- Each task is checked against its own `files` (per-task git status plus content hash). Out-of-scope writes are restored. A task in the ticket worktree then fails; an isolated task succeeds without them, since they are dropped from its patch, and the gateway logs the dropped paths. A tracked file that a build regenerates, such as a .NET `GenerateDocumentationFile` XML, is the usual case.
- `shared_files` are written only by the merge.
- When tasks run at the same time (more than one task and an effective parallelism above one), each task works in its own detached git worktree under `<run_dir>/iso/<task-id>`, created from the ticket worktree's HEAD plus its uncommitted state. The session's workspace is that tree, so the writes of a task are attributed exactly and a write to a sibling's file is an out-of-scope write. The same applies when the ticket worktree is a checkout the host prepared.
- When a task succeeds and passes the scope check, its changes are applied to the ticket worktree before its row is marked `Done`, then its tree is removed (also on failure, cancel and panic). Changes are applied in completion order; dependent tasks start after their dependency was applied.
- A change that does not fit the ticket worktree (for example an earlier task changed the same lines) fails the later task with `its changes conflict with the work of an earlier task` and lists the paths; the ticket worktree is left untouched and earlier tasks stay applied. A tree that cannot be created fails the task with `could not prepare an isolated worktree for the task`.
- Git-ignored files (`node_modules`, `bin/`, `obj/`) are not copied into an isolated tree.
- Patches are made and applied with `core.autocrlf=false`, from the root of the checkout, so line endings are kept and a ticket path below the repository root still works. A leftover tree of a run that died is removed when the next plan stage starts.
- A single task, or tasks that run one after the other, work in the ticket worktree directly.
- `on_item_failure: pause` stops launching and waits for running tasks; `continue` lets the merge see every task state.
- Sessions of one run share the run worktree through `shared_writer_group`; isolated tasks have their own workspace, so the session queue admits them in parallel. Other runs stay serialized per workspace.
- After a daemon crash, running rows go through recovery; a resume reruns the rows that are not `Done`.

### Workspace

A native run works in the directory its launch names: `workspace` of `POST /api/runs`, or the
workspace a session launch would get. `aidp run exec` resolves a relative `--workspace-root` (`.`
included) against the current directory before it registers the workspace, so the daemon, which
refuses a relative path, always receives an absolute one. A launch carrying a ticket (`worktree: KEY`, the dispatcher and
the flow pipeline steps) runs in that ticket's worktree, created or reused like a session's
(`<vault>/Pipelines/<repo>/<KEY>/.worktree` on `feat/<KEY>`). Without one, the run works in the
project checkout.

### Run directory and vault

The run directory is created in the vault the daemon resolves (`AIDP_VAULT`, then `~/.aidp/config.json`). `aidp run exec --vault <path>` picks another vault for that run; `--no-vault` keeps the run directory in the workspace, under `.aidp/pipelines/`, as `aidp run init --no-vault` does. Both work with `--foreground` and through the daemon (`vault` and `noVault` of `POST /api/runs`).

A run directory inside the workspace is invisible to the write-scope guards: its state files are not changes of the stage, and git collapsing an untracked `.aidp/` into one entry does not make that entry a violation.

### Write scope

After each writing stage the executor checks the changed files against the stage `scope`. Out-of-scope files are restored and the stage fails. The changed files come from the diff against the stage's starting commit plus `git status --untracked-files=all`, so a folder that was wholly untracked before a stage that commits it is not mistaken for new writes.

In a Unity project (the workspace contains `ProjectSettings/ProjectVersion.txt`) the guards also ignore the files the editor rewrites by itself, the constant `UNITY_EDITOR_OWNED` in `models/constants/task_scope.rs`: `Packages/packages-lock.json`, `ProjectSettings/**`, `UserSettings/**`, `Library/**`, `Temp/**`, `Logs/**`, `obj/**`, `*.csproj`, `*.sln`. They are neither reported nor restored, since an open editor would rewrite them at once. Other out-of-scope files are refused and restored as before; non-Unity projects are unchanged.

### State and restart

`run-state.json` has the same shape as in workflow mode, plus `mode: "native"`. After a daemon restart:

- a `done` stage is never replayed;
- an interrupted stage is relaunched once, with a resume instruction;
- a pending gate stays pending.

The budget guard waits when the token budget is exhausted. It never fails the run.

## Autonomy

A native run carries an immutable `autonomous` flag, fixed at launch. It defaults to true for ticket workflow and dispatcher launches; for a viewer launch the user chooses. Workflow mode (`workflow.js`) is unchanged.

In an autonomous run:

- Every gate is pre-approved.
- Every stage prompt starts with a versioned autonomy preamble (v1). The agent never asks a question. It takes the recommended option, else the most prudent and reversible one, and records the decision in `assumptions[]` with the choice, the reason and the rejected alternative.
- Each native stage session is launched with `AIDP_AUTONOMOUS=1`. That selects the `autonomous` session profile: `AskUserQuestion` is disallowed and the permission deny list is applied through launch arguments. Free autonomous sessions share the same profile. See [providers.md](providers.md).
- Ticket and dispatcher launches export the variable by default; a viewer launch keeps the explicit choice of the user.
- A reply with status `blocked`, or one that carries `ask_user_questions[]`, is auto-answered: the recommended option, else the first option, else a prudent default. Each answer is journaled as an assumption with the question id and text, then the stage is retried once.
- A second block fails the stage with reason `blocked-twice`.
- Assumptions are aggregated in `run-state.json` and returned by `GET /api/runs/:id` as `autonomous` and `assumptions`.
- A pipeline can report what it cannot fix in `signals.escalations`, an array of `{id, decision, reason}`, without pausing the run.

The rationale is in [adr/native-autonomy.md](adr/native-autonomy.md).

## Native by default for daemon launches

Every pipeline the gateway daemon launches creates a native run (`mode: "native"`): viewer template launch (`POST /api/sessions` with `templateId`), `aidp gateway start --template`, the ticket dispatcher and Start, automations and presets, and the flow `pipeline` step. A `RunLauncher` in `aidp-gateway` chooses the path; `SessionManager::start` is not branched, because the native executor starts its stage sessions through it.

| Launch | `autonomous` default |
|---|---|
| Ticket dispatcher and Start | true |
| Flow `pipeline` step | true |
| Viewer launch | false, the user chooses |
| Preset, automation | false, `autonomous: true` opts in |

`engine: "workflow"` (legacy alias `v7`) on a launch, preset or automation restarts the previous path: a Claude session drives `workflow.js`. `/run-pipeline-v7` and `aidp workflow emit` always stay in workflow mode. The rationale is in [adr/native-by-default-daemon-launches.md](adr/native-by-default-daemon-launches.md).

## Starting a native run

HTTP: `POST /api/runs`, see "Runs natifs" in `docs/gateway-api.md`. CLI:

```bash
aidp run exec dev-exemple --target LOC-1 --inputs-file inputs.json [--foreground]
```

The template `dev-exemple` runs collect, plan, then implementation. Workflow mode is unchanged: `aidp workflow emit` and the `Workflow` tool work as before.

## Host-controlled launches

An explicit `permissionMode` is honored for every native stage and never replaced by the legacy permission fallback.
Omitting it keeps the historical stage permission selection.
Hosted tool approvals use the session question and reply protocol; a flow gate and a tool approval are separate decisions.
`clientRequestId` identifies one launch intent, so an HTTP retry does not duplicate a run.
See [execution identities](gateway-api.md#execution-identities).

## Agent sessions run by a host

A run started with `agentExecutor: "external"` launches no session itself. `Dispatcher::run_job` builds an agent job for each stage, plan task, shard or merge session. It parks until a host reports the result. The reply check, the one repair, the scope guard, loops, gates and the run state stay in the gateway.

| Item | Rule |
| --- | --- |
| Storage | One file per job, `<run_dir>/agent-jobs/<seq>.json`. `seq` counts from 1 and is never reused |
| Job id | `agent:<runId>:<seq>` |
| States | `pending`, `claimed`, `done`, `cancelled` |
| Lease | A claim lasts 120 seconds. The host renews it every 30 seconds |
| Expired lease | The job returns to `pending` with `resumed: true`. The next claim appends the resumption note to the prompt. A second expiry fails the job with `host session interrupted twice` |
| Claim timeout | A `pending` job unclaimed for 30 minutes fails with `no host claimed the job` |
| Session timeout | A `claimed` job older than `timeoutMinutes` plus ten minutes fails with `host session exceeded its timeout` |
| Repair | An invalid reply on the first job publishes a repair job in the same call that delivers the result. An invalid reply on the repair fails the row |
| Waiting | The run thread parks on the run's wake condition and checks the deadlines every five seconds |
| Cancel and shutdown | Open jobs become `cancelled`. A late result answers `accepted: false` |
| Restart | The thread attaches to the open job of its row, or replays a verdict nobody consumed |

| Outcome posted by the host | Verdict |
| --- | --- |
| `completed` | The reply file is read and validated: accepted, repaired once, or failed |
| `failed` | `session failed: <reason>` |
| `timeout` | `session timed out: <reason>` |
| `rate_limited` | `session rate limited: <reason>`, with the retry delay when sent |
| `cancelled` | The row ends cancelled |

The usage of a result is priced like a gateway session, by the tier of the resolved model. `POST /api/runs` never accepts `agentExecutor`. Only a flow launch sets it.

## What stays at runtime

- Values of stage outputs, so conditions that read them. A flow reads the keys of the last stage's `result.json` (or its reply `signals`) as `steps.<id>.outputs.<key>`; see `docs/flows.md`.
- The `from_signal` items of a fan-out.
- The task graph of a `plan_tasks` step, expanded from `plan.json`.
- Gate approvals and resume anchors.
