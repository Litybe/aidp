---
description: Run the ai-dev-pipeline-v7 (Workflow-native) orchestrator. Usage — /run-pipeline-v7 [template_id] [target]. Example — /run-pipeline-v7 dev PIXEL-1234.
argument-hint: "[template_id] [target]"
---

Activate the **ai-dev-pipeline-v7** skill and run the Workflow-native orchestrator.

Arguments:
- `$1` = template id (e.g. `dev`, `study`, `ticket-create`, `discovery`, `tech-doc`, `func-doc`, `audit`). If omitted, list templates via `aidp pipeline list` and `AskUserQuestion`.
- `$2` = target identifier (Jira key, PRD path, service path — depends on template). If omitted, `AskUserQuestion`.

Set `AIDP_SKILL_ROOT` to this skill's directory (`~/.claude/skills/ai-dev-pipeline-v7`) for every `aidp` call so v7 templates are discovered (not v4's).

Follow `SKILL.md` exactly:
1. `aidp pipeline list` → `AskUserQuestion` for template (never infer). Collect enum inputs.
2. `aidp run init --pipeline <path> --target <id> [--inputs <json>]` → capture `run_id` from stdout.
   It also prints `run_dir:` and `vault:` on stderr — relay the run dir, since v7 writes it into the
   Obsidian vault rather than the repo. Run `aidp vault status --format json-compact` first if you
   have not this session, and warn the user if no vault is configured (output falls back to the repo).
3. `aidp workflow emit --run <run_id>` → capture the printed `workflow.js` path.
4. Invoke the `Workflow` tool with `{ scriptPath: "<path>" }` (no `resumeFromRunId` on the first call).
5. On **every** Workflow result (`done`, `awaiting_gate`, `awaiting_plan`, `awaiting_composite`, `paused`), first write the result object (its `.result` if the Workflow returned an envelope) to `<run_dir>/workflow-result.json`, before any `aidp` call, then `aidp run reconcile --run <run_id> --from <run_dir>/workflow-result.json`, then the branch's own call. On `awaiting_gate` → `aidp gate pending --run <run_id> --gate <stage> --workflow-run-id <runId>`, then `AskUserQuestion`; on approve, `aidp run record-resume --run <run_id> --workflow-run-id <runId> --approved-gate <stage>`, re-emit, then resume with `{ scriptPath, resumeFromRunId, args: { approved_gates: ["<stage>"] } }` (only the latest gate). On `done` → write the result to `<run_dir>/workflow-result.json`, `aidp run reconcile --run <run_id> --from <file>`, then `aidp run summary`. On `paused` → write the result to `<run_dir>/workflow-result.json`, `aidp run reconcile --run <run_id> --from <file>`, **then** `aidp run record-resume --run <run_id> --workflow-run-id <runId>` (persist the anchor even though this is not a gate — otherwise a later resume has nothing to resume from and re-runs everything), then relay the envelope and stop.
6. On `awaiting_plan` (a plan-driven stage: `dev` / `dev-lambda-ts` compile `implement` and everything after it from the planner's `plan.json`) → write and reconcile the result file as in step 5, then `aidp run record-resume --run <run_id> --workflow-run-id <runId>`, then `aidp workflow emit --run <run_id>` again (this is where `aidp` schedules the tasks and serializes write conflicts), optionally show `aidp workflow plan-status --run <run_id> --format json-pretty`, then resume with `{ scriptPath, resumeFromRunId }`. It is a normal stop, not a gate and not an error.

If any `aidp` call exits 127 (`command not found`), follow `SKILL.md` "Exit 127": (1) retry the same call with `"$AIDP_BIN"` (PowerShell: `& $env:AIDP_BIN`) and keep using it; (2) if `AIDP_BIN` is unset or fails, STOP and tell the user the run dir, `run_id`, Workflow `runId`, last completed step, and the `aidp run record-resume --run <run_id> --workflow-run-id <runId> [...]` command to replay, then `/pipeline-resume-v7 <run_id>`.

Do NOT call `Edit`, `Write`, `MultiEdit`, or non-`aidp` `Bash` yourself, and do NOT hand-edit `workflow.js`. All real work happens inside the Workflow's sub-agents.
