---
description: Scaffold a new ai-dev-pipeline-v7 agent via interactive Q&A. Generates the agent.md file.
argument-hint: "[name]"
---

Create a new sub-agent for **ai-dev-pipeline-v7**. Generates `agents/<name>.md` with the right frontmatter.

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7` for every `aidp` call.

Arguments:
- `$1` = agent name (kebab-case or snake_case). If omitted, ask via `AskUserQuestion`.

Skill root: the directory containing this skill's `SKILL.md`.

**Important:** Run this command **outside any pipeline run**.

## Procedure

### 1. Gather metadata

Ask via `AskUserQuestion`:
- **Agent name** (snake_case, e.g. `release_publisher`). If `$1` was provided, skip.
- **Role** — pick one:
  - `research` — explores and synthesizes (like analyzer)
  - `plan` — produces step-by-step instructions (like planner)
  - `produce` — creates artifacts: code, tickets, PRs, messages (like executor)
  - `review` — audits work, returns findings (like reviewer)
- **Default scope** — pick one:
  - `stage_dir_only` (default for research/plan/review)
  - `production` (default for produce / code agents)
  - `tests`
  - `repo_wide` (rare — explicit allowlist required in stage YAML)
- **One-line description** — what this agent specializes in. Example: "Authors release notes from a list of merged PRs."
- **Typical skills** (comma-separated, optional) — the skills this agent is usually paired with. Example: `generating-release-message, writing-clearly-and-concisely`.

### 2. Generate agent.md

Write to `<skill_root>/agents/<name>.md`:

```markdown
---
name: <name>
role: <role>
default_scope: <scope>
output_envelope: agent-reply.v4
description: <description>
---

# <name>

<one-paragraph description of what the agent does — derived from inputs>

## Inputs

Standard task prompt envelope (`goal`, `inputs`, `skills`, `outputs_declared`, `scope`).

## Procedure

1. Load every skill listed in `skills`. They prescribe the *how*.
2. Read every input path. Use `Glob`/`Grep`/`Read` natively.
3. If you hit a blocking ambiguity, call `AskUserQuestion`.
4. Produce each declared output in `stage_dir` (or under `scope`).
5. Emit the JSON envelope (`agent-reply.v4`) with `outputs` = name → absolute path.

## Typical skills

- <skill_1>
- <skill_2>

## What you must NOT do

- Modify files outside `scope`.
- Re-implement orchestrator logic — call `aidp` sub-commands.
- Skip producing the declared `outputs` — the run-state expects them.
```

Use the `Write` tool. (`output_envelope: agent-reply.v4` is the **envelope schema name**, shared by v4 and
v7 sub-agents — do not rename it.)

### 3. Verify

Run `aidp agents list` and confirm the new agent appears in the list.

Print to the user: `Agent '<name>' created. Reference it in a stage with 'agent: <name>'. To use, add the agent name in your pipeline YAML's stage.`

### 4. Cleanup on abort

If the user aborts mid-flow before the file is written, do nothing. If the file was written but they want to undo, use `Bash rm <skill_root>/agents/<name>.md`.
