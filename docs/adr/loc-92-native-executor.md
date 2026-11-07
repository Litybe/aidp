# ADR: Native executor

Status: accepted

## Context

A v7 run needs a Claude Workflow session and a driver that relays gates. The daemon cannot run a pipeline on its own, and `aidp-core` already compiles the routing decisions into a typed `ExecPlan`.

## Decision

1. Add the module `aidp-gateway/src/native_runs/`. It walks the steps of `compile_exec_plan` and never re-derives routing.
2. Run each stage as one daemon session with engine `stage` and origin `{kind: run, ref: run_id, label: stage}`.
3. Share only persistence primitives with the flow executor. Native runs have their own state machine, store and routes.
4. Keep `run-state.json` in the workflow-mode shape, with the added field `mode: "native"`.
5. Run the tasks of a `plan_tasks` stage one session at a time. Parallel tasks come later.
6. Leave Workflow mode unchanged.

## Consequences

- A run can start from `POST /api/runs` or `aidp run exec` with no driver session.
- Gates surface in the inbox and over the API, or are skipped with `autoApproveGates`.
- Both modes write the same run-state, so `aidp run summary` and the vault notes work for either.

## State and recovery diverge from flows

The native executor keeps its own store and recovery (`native_runs/state.rs`, `native_runs/recovery.rs`). It reuses only `flows::lock` (workspace lock) and `flows::registry` (budget guard).

- Different persisted shape. `RunStore` writes the `aidp-core` `RunState` as `run-state.json` plus `native-run.json` (the `NativeRunView`) in the run directory. `FlowStore` (`flows/store.rs`) writes `state.json`, `journal.jsonl` and `runtime.json` under `<ticket>/flow/`, modelling per-ticket flow steps with phases and iterations.
- Different index. Native runs are indexed by run id in `native-runs/index.json`. Flows are indexed per repo and ticket in `flows/index.json`.
- Different recovery rules. `native_runs::recovery::resume_all` reloads runs whose status is pending, running or waiting for a gate, and relaunches a stage that was running once with a resume instruction. A stage that was already resumed and is interrupted again is failed with the reason `interrupted twice`. `flows::recovery` instead reconciles every active flow through `reconcile_all` and re-polls on session end, GitHub webhook and a 60 s timer; it has no interrupted-twice rule and no stage relaunch.
- A shared store would force `RunState` and the loop, fan-out and plan-task bookkeeping into the flow model, or flow steps into `run-state.json`, which Workflow mode and `aidp run summary` read unchanged.

## Alternatives rejected

- Model runs as `aidp-flows` `FlowDef`s: flows describe per-ticket steps, while a run needs loops, fan-out, plan tasks and composite boundaries that the plan already encodes.
