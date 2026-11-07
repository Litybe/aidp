---
description: List ai-dev-pipeline-v7 runs in the current workspace. Usage — /pipeline-status-v7.
argument-hint: ""
---

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7`, then run `aidp status --format json-pretty`
and render a friendly table.

Procedure:

0. Run `aidp vault status --format json-compact` so you can tell the user where the runs live.
1. Run `aidp status --format json-pretty` from the user's current working directory. It scans **two**
   roots and merges them: this repo's folder in the Obsidian vault
   (`<vault>/Pipelines/<repo_segment>/`, holding every v7 run) and the legacy in-repo
   `.aidp/pipelines/` (holding every pre-v7 run, still listed and still resumable).
2. Parse the JSON array. Each entry has: `run_id`, `pipeline_id`, `target`, `overall_status`
   (`pending` | `in_progress` | `awaiting_gate` | `paused` | `done` | `failed`), `current_stage`,
   `stages_done`, `stages_total`, `started_at`.
3. If empty → say "No runs for this repo, in `<vault root from step 0>` nor in `<cwd>/.aidp/pipelines/`.
   Start one with `/run-pipeline-v7 <template>`."
4. Else render a markdown table sorted by `started_at` desc:

| Run | Pipeline | Target | Status | Stage | Progress | Started |
|---|---|---|---|---|---|---|
| `20260524-...` | dev | PIXEL-1234 | in_progress | code | 3/7 | 2026-05-24 14:30 |

5. After the table, mention `/pipeline-resume-v7 <run_id>` for paused/awaiting-gate runs, and give
   the vault path so the user can open the runs as notes in Obsidian.
