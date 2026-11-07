---
description: Validate a v7 pipeline YAML against the pipeline schema. Accepts a template id or a file path.
argument-hint: "[template_id_or_path]"
---

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7`, then run `aidp validate <id|path>` and explain
the result.

Arguments:
- `$1` = either a template id (e.g. `dev`, `study`) **or** an absolute/relative path to a YAML file.
- If omitted, call `aidp pipeline list --format json-compact` and use `AskUserQuestion` to let the user pick one.

Procedure:

1. Run `aidp validate <id_or_path>` directly with `$1` as given — `aidp` resolves an id or a path itself, positionally. On an unknown id it prints a "pipeline not found" error suggesting close ids; on a missing path it prints the underlying I/O error.
2. The command exits 0 on success and prints `{"ok": true, "path": "..."}`. On failure it prints a parse/schema error on stderr.
3. Report to the user:
   - Success → `Pipeline '<id>' is valid.`
   - Failure → show the error verbatim, then hint at common causes (missing `agent:` on a stage, unknown skill name, malformed `with:`, `version:` not set to 5).
4. Do NOT modify the YAML. This is a read-only check.
