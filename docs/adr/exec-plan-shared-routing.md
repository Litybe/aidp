# ADR: One typed execution plan shared by every executor

Status: accepted

## Context

The Workflow emitter decides routing and renders JavaScript in the same pass. A native executor would need the same decisions, with no way to read them except from the JS text.

## Decision

1. Move the routing decisions into `aidp-core/src/exec_plan/`: a pure, typed plan built by `compile_exec_plan(pipeline, state, plan_source)`.
2. Render `workflow.js` from that plan. The output stays byte-identical, guarded by `crates/aidp-core/tests/workflow_emit_golden.rs`.
3. Resolve conditions that read only inputs at compile time; keep conditions that read a stage output as expressions.
4. Scope the plan to one pass and recompute it on every emit. Never store it in run-state.
5. Expose it with `aidp run plan-exec --run <id>`, which writes `<run_dir>/execution-plan.json` and prints its path. The schema is `schema/execution-plan.schema.json`.

## Consequences

- Both executors read the same decisions, so they cannot drift.
- Workflow-mode drivers see no change.
- A change to routing touches one module and its golden test.
- The plan has no persisted copy to go stale.

## Alternatives rejected

- Recording a plan as a side effect of the emitter: the plan then depends on running the JS rendering, and a stored copy can drift from what ran.
- Re-implementing routing in the native executor: two implementations of the same rules diverge.
