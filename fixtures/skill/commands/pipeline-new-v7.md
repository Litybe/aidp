---
description: Scaffold a new ai-dev-pipeline-v7 template via interactive Q&A. Generates and validates the YAML.
argument-hint: "[id]"
---

Create a new **v7** pipeline template by walking the user through the schema, then write the YAML and
validate it. The user picks the destination: either the **skill templates** (shared across projects) or
`.aidp/pipeline/flow/` in the **current repo** (project-local).

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7` for every `aidp` call.

Arguments:
- `$1` = pipeline id (kebab-case). If omitted, ask via `AskUserQuestion`.

Skill root: the directory containing this skill's `SKILL.md` (resolve via `aidp doctor` if unsure).

## Procedure

### 1. Gather metadata

Ask the user via `AskUserQuestion`:
- **Pipeline id** (kebab-case, no spaces) — if not provided in `$1`.
- **Description** (one short sentence shown in `aidp pipeline list`).

### 1bis. Choose destination

Ask via `AskUserQuestion`:

Question: "Où créer la pipeline?"
- **Partagée (skill templates)** — écrit dans `<skill_root>/templates/<id>.yaml`. Visible depuis n'importe quel repo qui utilise ce skill. Bon choix pour les pipelines réutilisables (dev, study, audit, ...).
- **Locale à ce repo (`.aidp/pipeline/flow/`)** — écrit dans `<cwd>/.aidp/pipeline/flow/<id>.yaml`. Visible **uniquement** quand `/run-pipeline-v7 <id>` est lancé depuis `<cwd>` (ou un sous-dossier). Bon choix pour une pipeline propre à un service.

Note pour Claude:
- Récupère `<cwd>` via le contexte de session.
- Récupère `<skill_root>` via `aidp doctor` (champ `skill_root.detail`) ou via la variable `AIDP_SKILL_ROOT` si définie.
- Mémorise la valeur choisie dans `dest_path` (string) pour utilisation aux étapes 4-6.

### 2. Gather inputs

Ask: "How many inputs does this pipeline need?" (Options: 0 / 1 / 2 / 3+).

For each input, ask in one batched `AskUserQuestion` (one question per input is fine):
- **Name** (snake_case)
- **Kind** (string / enum / bool / int)
- **Required** (true / false)
- If kind=enum → ask for the allowed values (comma-separated).

### 3. Gather stages

Ask: "How many stages?" (typical: 2-6).

For each stage, ask:
- **Name** (snake_case verb: `discover`, `plan`, `code`, `review`, ...)
- **Agent** — pick one of the 4 generic agents via `AskUserQuestion`:
  - `analyzer` — investigates, reads code, synthesizes (default scope: stage_dir_only)
  - `planner` — produces a step-by-step plan (default scope: stage_dir_only)
  - `executor` — writes code, opens PRs, creates tickets (default scope: production)
  - `reviewer` — audits the work, returns findings (default scope: stage_dir_only)
- **Model** (optional) — `sonnet` / `opus` / `haiku` / `(default — inherit session model)`. Overrides the model family for this stage only. Passed to the Workflow `agent()` call as `model`. If user picks "default", omit the field from YAML.
- **Skills** (comma-separated, optional) — e.g. `graphify-context`, `dotnet-engineering`, `pr`, `git-push`. Call `aidp skills list` to suggest names.
- **Depends on** (comma-separated previous stage names, optional). Stages with disjoint `needs` run **in parallel** in the emitted Workflow — encourage fan-out where stages are independent.
- **Outputs** (comma-separated filenames, e.g. `context.md`, `plan.md`, `changed-files.json`).
- **Scope mode** for executor stages: `production` / `tests` / `stage_dir_only` / `repo_wide`.
- **Worktree isolation** (executor stages only, optional) — set `isolation: worktree` if several executors may write files in parallel (v7 Workflow engine only; collision-free parallel writes).
- **Plan-driven?** (executor stages only, optional) — if an upstream `planner` stage declares a `plan.json`
  output, this stage can be compiled from that task graph instead of describing its own work:
  `plan_tasks: { from: "{{ stages.<planner>.outputs.plan }}", max: 4, merge_hint: "..." }`. `aidp` then
  runs one sub-agent per plan task, in parallel wherever their declared write sets are disjoint, and adds
  a merge sub-agent. Offer it when the stage is "implement the plan" work; the run then needs one extra
  `emit`+resume round-trip (`awaiting_plan`). Mutually exclusive with `fan_out`, and not allowed in a loop
  body. See docs/agents-reference.md §7.1.

### 4. Generate YAML

Produce a file at `<dest_path>` (from step 1bis) with this exact shape:

```yaml
version: 5
id: <id>
description: <description>
inputs:
  <input_name>: { required: <bool>, kind: <kind> }
  # ... per input
stages:
  - name: <stage_name>
    agent: <agent>
    model: <sonnet|opus|haiku>      # only if user picked non-default
    skills: [<skill>, <skill>]
    needs: [<dep>, <dep>]
    outputs: [<output_file>]
    scope: { mode: <scope_mode> }   # only if executor with non-default scope
    isolation: worktree             # only if the user opted in (executor stages)
    plan_tasks:                     # only for a plan-driven stage (see above)
      from: "{{ stages.<planner_stage>.outputs.plan }}"
      max: 4
  # ... per stage
```

**Before writing**, ensure the parent directory exists:
- For the project-local destination, `.aidp/pipeline/flow/` may not exist yet in a fresh repo. Run `Bash mkdir -p <cwd>/.aidp/pipeline/flow` (allowed since no run is active).
- For the shared destination, `<skill_root>/templates/` always exists (validated by `aidp doctor`).

Then write the file with the `Write` tool to `<dest_path>`.

### 5. Validate

Run `aidp validate <dest_path>`.
- Success →
  - If destination was **Partagée**: print `Pipeline '<id>' created at <dest_path> and validated. Try /run-pipeline-v7 <id> from any repo.`
  - If destination was **Locale**: print `Pipeline '<id>' created at <dest_path> and validated. Try /run-pipeline-v7 <id> from <cwd> (or a subfolder). It won't be visible from other repos.`
- Failure → show the error, ask the user if they want to revise (re-run the relevant Q&A step) or abort.

### 6. Cleanup on abort

If the user aborts mid-flow, delete the partially-written file at `<dest_path>` (use `Bash rm <dest_path>` since we're not in a run).
