---
description: Resume a paused ai-dev-pipeline-v7 (Workflow-native) run. Usage — /pipeline-resume-v7 [run_id].
argument-hint: "[run_id]"
---

Resume a v7 (Workflow-native) pipeline run. This is **not** the v4 `orchestrate next` loop — v7 resumes
by re-emitting `workflow.js` and re-invoking the `Workflow` tool with `resumeFromRunId`.

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7` for every `aidp` call so v7 templates are
discovered.

- `$1` = run_id. If omitted, run `aidp status --format json-pretty` (it lists both the vault runs and
  the legacy in-repo ones) and `AskUserQuestion` to pick one. Resolve the run directory with
  `aidp run summary --run $1`, which prints it — under v7 it is inside the Obsidian vault, not under
  `.aidp/pipelines/`, so never construct that path by hand.

Procedure:
1. Read `<run_dir>/run-state.json` (or `aidp run summary --run $1`) to see where the run is paused and to
   recover the persisted resume context:
   - `workflow_run_id` → the Workflow-tool `runId` to pass as `resumeFromRunId`.
   - `gates[]` with `decision: "approve"` → the gates already approved (baked into the emitted
     `__approved` seed on re-emit).
   - `pending_gate` → **a gate that was proposed but never approved.** If it is set, the run is paused
     waiting on a human decision at that gate. Resume must **re-propose it** (below) — it must NOT be
     baked into `__approved` and the run must NOT be forced to re-run from scratch to reach it.

   **Cold start.** If `workflow_run_id` is null, `pending_gate` is null, and
   `<run_dir>/workflow-result.json` exists, run
   `aidp run record-resume --run $1 --from-result <run_dir>/workflow-result.json`, re-read the state,
   and continue with the `pending_gate` branch (step 2) if it is now set. The flag rebuilds
   `workflow_run_id` and `pending_gate` from the file; an approved gate is never re-opened.
2. **If `pending_gate` is set, re-propose it right away** — this is the whole point of resuming a
   gate-paused run. `AskUserQuestion` ("Approve stage `<pending_gate>`?" → approve / abort):
   - **approve** → `aidp run record-resume --run $1 --workflow-run-id <workflow_run_id> --approved-gate <pending_gate>`
     (bakes the approval into `__approved` and clears `pending_gate`). Continue to step 3.
   - **abort** → `aidp run summary --run $1` and stop (the pending gate stays recorded for next time).
3. Re-emit the script: `aidp workflow emit --run $1` → capture the printed `workflow.js` path.
4. Re-invoke the `Workflow` tool:
   `{ scriptPath: "<path>", resumeFromRunId: "<workflow_run_id>", args: { approved_gates: ["<gate just approved>"] } }`.
   - Only the **latest** approved gate is needed in `args` — prior approvals are baked into `__approved`.
   - If you did NOT approve a gate in step 2 (there was no pending gate), omit `args` and just pass
     `{ scriptPath, resumeFromRunId: "<workflow_run_id>" }`.
   - If `workflow_run_id` is still null after the cold-start step (the result file is missing or records
     no `runId`), the run has no anchor. `AskUserQuestion` to confirm a full re-run first; on confirm,
     call `Workflow` with just `{ scriptPath }` — completed stages re-execute unless idempotent, and
     already-approved gates do not re-prompt. On decline, stop.
   - Note: the `Workflow` cache (`resumeFromRunId`) only replays within the **same session**. Across
     sessions the prefix re-executes; `__approved` still suppresses gate prompts, so an approved gate is
     never re-asked even after a full re-run.
5. Then follow `SKILL.md` step 6 (result-object state machine). On **every** branch below, first write
   the result object (its `.result` if the Workflow returned an envelope) to
   `<run_dir>/workflow-result.json`, before any `aidp` call, then
   `aidp run reconcile --run $1 --from <run_dir>/workflow-result.json`, then the branch's own call:
   - `done` → print `aidp run summary --run $1`.
   - `awaiting_gate` → persist the anchor: `aidp run record-resume --run $1 --workflow-run-id <runId> --pending-gate <stage>`; then `AskUserQuestion`. On approve, `aidp run record-resume --run $1 --workflow-run-id <runId> --approved-gate <stage>`, re-emit, resume again. On abort → `aidp run summary` and stop (the gate stays re-proposable on the next resume).
   - `awaiting_plan` → the run stopped at a plan-driven stage whose tasks (and tail) are compiled from
     the planner's `plan.json`: `aidp run record-resume --run $1 --workflow-run-id <runId>`, then
     `aidp workflow emit --run $1` again (aidp now loads the plan, validates the task graph and inserts
     an ordering wherever two tasks would write the same file), optionally
     `aidp workflow plan-status --run $1 --format json-pretty` to show the parallelisation, then resume
     with `{ scriptPath, resumeFromRunId }`. If the emit **fails**, the plan is malformed — the planner
     stage has to be re-run; never hand-edit `plan.json` or `workflow.js`.
   - `paused` → **first** `aidp run record-resume --run $1 --workflow-run-id <runId>` to persist the
     anchor (not a gate — no `--pending-gate`/`--approved-gate` — but without this the run still has no
     resumable anchor and the *next* resume attempt falls back to re-running from scratch), **then**
     relay `envelope.ask_user_questions` / `envelope.summary` and stop. An `at` of `<stage>#<task_id>`
     is one failed **plan task**; `aidp workflow plan-status --run $1` shows which tasks did complete.

If any `aidp` call exits 127 (`command not found`), follow `SKILL.md` "Exit 127": (1) retry the same call with `"$AIDP_BIN"` (PowerShell: `& $env:AIDP_BIN`) and keep using it; (2) if `AIDP_BIN` is unset or fails, STOP and tell the user the run dir, `run_id`, Workflow `runId`, last completed step, and the `aidp run record-resume --run $1 --workflow-run-id <runId> [...]` command to replay, then `/pipeline-resume-v7 $1`.

Do NOT call `Edit`, `Write`, `MultiEdit`, or non-`aidp` `Bash` yourself, and do NOT hand-edit
`workflow.js`. All real work happens inside the Workflow's sub-agents.
