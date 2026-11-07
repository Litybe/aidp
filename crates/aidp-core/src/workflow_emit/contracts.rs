use crate::exec_plan::refs::{parse_fanout_from, parse_plan_from, plan_artifact_path};
use crate::models::constants::review::AXIS_INPUT;
use crate::models::enums::{ReviewAxis, ToolEnforcement};
use crate::pipeline::{self, Pipeline};
use crate::plan;
use crate::run::RunState;
use crate::toolset;
use std::collections::{BTreeMap, BTreeSet};

/// Every stage carrying `fan_out:` must declare exactly one shard source and
/// be a plain agent stage (not a loop/composite container).
pub(super) fn validate_fanout(pipe: &Pipeline) -> anyhow::Result<()> {
    for st in pipeline::flat_stages(pipe) {
        let Some(fo) = &st.fan_out else { continue };
        if st.is_loop() || st.is_composite() {
            anyhow::bail!("stage `{}`: fan_out is only supported on plain agent stages, not loop/pipeline containers", st.name);
        }
        match (fo.items.is_empty(), fo.from.is_none()) {
            (true, true) => anyhow::bail!("stage `{}`: fan_out needs `items:` or `from:`", st.name),
            (false, false) => anyhow::bail!(
                "stage `{}`: fan_out takes `items:` OR `from:`, not both",
                st.name
            ),
            _ => {}
        }
        if let Some(from) = &fo.from {
            parse_fanout_from(&st.name, from)?;
        }
    }
    Ok(())
}

/// For each dynamic fan-out (`from: stages.<p>.signals.<prop>`), build the contract section
/// injected into producer `<p>`'s prompt.
pub(super) fn fanout_contracts(pipe: &Pipeline) -> anyhow::Result<BTreeMap<String, String>> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for st in pipeline::flat_stages(pipe) {
        let Some(fo) = &st.fan_out else { continue };
        let Some(from) = &fo.from else { continue };
        let (producer, prop) = parse_fanout_from(&st.name, from)?;
        if pipeline::find_stage(pipe, &producer).is_none() {
            anyhow::bail!(
                "stage `{}`: fan_out.from references unknown stage `{producer}`",
                st.name
            );
        }
        let contract = format!(
            "\n\n## Fan-out contract (consumer: `{consumer}`)\n\nThe downstream stage `{consumer}` fans out into up to {max} parallel sub-agents that share ONE working tree. Your reply envelope MUST include `signals.{prop}`: a JSON array of 1..{max} short shard labels, each naming an independent work package covering together ALL of the work. The packages MUST be file-disjoint — no two shards may plan to create or modify the same file. Make each label self-explanatory to an agent that sees only the label plus your plan artifact (e.g. \"handlers + validators for X\", \"repository + migrations for Y\"). If the work cannot be split into safely-disjoint packages, return a single-element array.\n",
            consumer = st.name,
            max = fo.max.max(1),
            prop = prop,
        );
        map.entry(producer)
            .and_modify(|c| c.push_str(&contract))
            .or_insert(contract);
    }
    Ok(map)
}

/// Config validation for every `plan_tasks:` stage.
pub(super) fn validate_plan_tasks(pipe: &Pipeline) -> anyhow::Result<()> {
    let mut claimed_kinds: BTreeMap<String, String> = BTreeMap::new();
    let mut all_claimers: Vec<&str> = Vec::new();
    for st in &pipe.stages {
        for b in &st.body {
            if b.plan_tasks.is_some() {
                anyhow::bail!(
                    "stage `{}` (inside loop `{}`): plan_tasks is not supported in a loop body — a plan-driven stage compiles in two passes and cannot re-compile per iteration",
                    b.name,
                    st.name
                );
            }
        }
        let Some(pt) = &st.plan_tasks else { continue };
        if st.is_loop() || st.is_composite() {
            anyhow::bail!(
                "stage `{}`: plan_tasks is only supported on plain agent stages, not loop/pipeline containers",
                st.name
            );
        }
        if st.fan_out.is_some() {
            anyhow::bail!(
                "stage `{}`: plan_tasks and fan_out are mutually exclusive — a plan-driven stage already shards itself over the plan's tasks",
                st.name
            );
        }
        if st.composite_entry.is_some() {
            anyhow::bail!(
                "stage `{}`: plan_tasks is not supported on a composite's entry stage — the composite boundary and the plan's two-pass compilation cannot both decide when this stage compiles",
                st.name
            );
        }
        if pt.max == 0 || pt.max > 16 {
            anyhow::bail!(
                "stage `{}`: plan_tasks.max must be 1..=16, got {}",
                st.name,
                pt.max
            );
        }
        let (producer, output) = parse_plan_from(&st.name, &pt.from)?;
        let Some(prod) = pipeline::find_stage(pipe, &producer) else {
            anyhow::bail!(
                "stage `{}`: plan_tasks.from references unknown stage `{producer}`",
                st.name
            );
        };
        let Some(filename) = prod.declared_filename(&output) else {
            anyhow::bail!(
                "stage `{}`: plan_tasks.from references `{producer}.{output}`, but stage `{producer}` declares outputs {:?}",
                st.name,
                prod.declared_outputs()
            );
        };
        if !filename.to_ascii_lowercase().ends_with(".json") {
            anyhow::bail!(
                "stage `{}`: plan_tasks.from resolves to `{filename}` on stage `{producer}`, but the plan must be JSON. Declare the `.json` artifact first in `{producer}.outputs` (e.g. `[plan.json, plan.md]`), or point `from:` at a different output.",
                st.name
            );
        }
        for k in &pt.kinds {
            if !plan::KINDS.contains(&k.as_str()) {
                anyhow::bail!(
                    "stage `{}`: plan_tasks.kinds has unknown kind `{k}` — expected one of {:?}",
                    st.name,
                    plan::KINDS
                );
            }
            if let Some(other) = claimed_kinds.insert(k.clone(), st.name.clone()) {
                anyhow::bail!(
                    "stages `{other}` and `{}` both claim plan task kind `{k}` — plan_tasks.kinds must be disjoint",
                    st.name
                );
            }
        }
        if pt.kinds.is_empty() {
            all_claimers.push(&st.name);
        }
    }
    if all_claimers.len() > 1 {
        anyhow::bail!(
            "stages {all_claimers:?} each claim EVERY plan task (no `kinds:` filter) — the same task would run twice; give each stage a disjoint `kinds:` list"
        );
    }
    if !all_claimers.is_empty() && !claimed_kinds.is_empty() {
        anyhow::bail!(
            "stage `{}` claims every plan task while {:?} claim specific kinds — the same task would run twice",
            all_claimers[0],
            claimed_kinds.values().collect::<BTreeSet<_>>()
        );
    }
    Ok(())
}

/// For each `plan_tasks:` stage, the contract appended to the **producer's** prompt. This is what
/// turns a prose plan into an executable one.
pub(super) fn plan_contracts(
    state: &RunState,
    pipe: &Pipeline,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for st in &pipe.stages {
        let Some(pt) = &st.plan_tasks else { continue };
        let (producer, output) = parse_plan_from(&st.name, &pt.from)?;
        let path = plan_artifact_path(state, pipe, &producer, &output);
        let kinds: Vec<&str> = if pt.kinds.is_empty() {
            plan::KINDS.to_vec()
        } else {
            pt.kinds.iter().map(|s| s.as_str()).collect()
        };
        let contract = format!(
            r#"

## Plan contract — write an EXECUTABLE plan (consumer: `{consumer}`)

Your plan is not read by a human before it runs: `aidp` compiles it. Write it as JSON at

    {path}

conforming to `schema/plan.schema.json` (`{schema_id}`): **one object per task to realize**, each carrying its own prompt. `aidp workflow emit` then turns every task into one sub-agent of stage `{consumer}` and schedules them itself.

### Shape

```json
{{
  "schema": "{schema_id}",
  "summary": "one paragraph: what this plan achieves",
  "shared_files": ["src/Program.cs", "serverless.yml"],
  "tasks": [
    {{
      "id": "T1",
      "title": "one imperative line",
      "kind": "code",
      "prompt": "The full instruction for the sub-agent that will execute THIS task and nothing else.",
      "instructions": ["step 1", "step 2"],
      "needs": [],
      "files": ["src/Feature/FooHandler.cs", "src/Feature/FooRequest.cs"],
      "reads": ["src/Existing/BarHandler.cs"],
      "verification": "how to check this task alone is correct",
      "acceptance": ["the AC this task satisfies"],
      "jira_keys": ["<US this task serves>"],
      "risk": "low"
    }}
  ]
}}
```

### The five rules that make it executable

1. **`prompt` is self-contained.** The sub-agent executing task `T1` sees only its own prompt plus this stage's artifact paths — never your other tasks, never this plan's prose. Put the decision *in* the prompt: what to change, which existing pattern/utility to reuse (with its path), which conventions apply, what NOT to touch. Never write "as described above" or "see the plan".
2. **`files` is the write set, and it is a contract.** List every repo-relative file the task will create or modify — exactly. `aidp` runs tasks with disjoint write sets **in parallel** in ONE shared working tree, and inserts an ordering whenever two tasks would write the same file. A missing path is a lost race; a padded path costs parallelism.
3. **`needs` is for real dependencies only** (task B needs the type/API task A introduces). Do NOT add `needs` to avoid write collisions — that is `files`' job, and `aidp` handles it. Every `needs` you add narrows the parallelism.
4. **`shared_files` is hands-off for tasks.** Files several tasks would have to touch (DI registration, module wiring, `.csproj`/`serverless.yml` entries, `.sln`) belong here and in **no** task's `files`. A merge sub-agent applies all of them once, at the end, from the tasks' `follow_ups`.
5. **Slice for parallelism, not for paperwork.** Kinds this pipeline executes: {kinds}. Tests are tasks too — a test task typically `needs` the code task it covers, and owns its own test files. Aim for the widest set of file-disjoint tasks that are each a single coherent, verifiable change; up to {max} run concurrently. Do not split one file across two tasks, and do not bundle unrelated files into one task.

A task says **what work to do and which files it owns** — nothing about how it is dispatched. The sub-agent role, the model and the skills every task runs with come from the consuming stage `{consumer}`, so do NOT put `agent`, `model` or `skills` in a task; they would be ignored.

Also write the human-readable narrative you would normally produce (design, risks, rollback) — the JSON is the machine contract, not a replacement for explaining yourself. Both files count as this stage's outputs, and `outputs.{output}` in your envelope MUST be the absolute path of the JSON above.

If the work genuinely cannot be split (one file, one change), emit a single task — that is a valid plan and costs nothing extra.
"#,
            consumer = st.name,
            path = path.display(),
            schema_id = plan::PLAN_SCHEMA_ID,
            kinds = kinds.join(", "),
            max = pt.max,
            output = output,
        );
        map.entry(producer)
            .and_modify(|c| c.push_str(&contract))
            .or_insert(contract);
    }
    Ok(map)
}

/// `merge_findings:` is only meaningful on a plain sequential agent stage.
pub(super) fn validate_merge_findings(pipe: &Pipeline) -> anyhow::Result<()> {
    for st in pipeline::flat_stages(pipe) {
        if st.merge_findings.is_none() {
            continue;
        }
        if st.is_loop() || st.is_composite() || st.fan_out.is_some() || st.plan_tasks.is_some() {
            anyhow::bail!(
                "stage `{}`: merge_findings is only supported on a plain sequential agent stage, not loop/pipeline/fan_out/plan_tasks",
                st.name
            );
        }
    }
    Ok(())
}

/// the real guardrail behind `settings.tool_enforcement: strict`.
pub(super) fn validate_tool_enforcement(pipe: &Pipeline) -> anyhow::Result<()> {
    validate_tool_enforcement_against(pipe, toolset::registry_dir().as_deref())
}

/// The testable half of [`validate_tool_enforcement`], taking the registry directory explicitly
/// instead of resolving `USERPROFILE`/`HOME` itself.
pub(super) fn validate_tool_enforcement_against(
    pipe: &Pipeline,
    registry: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    if pipe.settings.tool_enforcement != ToolEnforcement::Strict {
        return Ok(());
    }
    let missing = toolset::missing_agent_toolsets(pipe, registry);
    if !missing.is_empty() {
        anyhow::bail!(
            "settings.tool_enforcement: strict, but the agent registry is missing a `tools:`-bearing persona for: {}. Run `aidp setup agents` to generate them (or `aidp doctor` for the same check, named `agent_toolsets`), then re-run `aidp workflow emit`.",
            missing.join(", ")
        );
    }
    Ok(())
}

/// For each stage declaring `merge_findings: { from, out }`, annex to ITS OWN prompt the
/// mechanical-verdict contract.
pub(super) fn merge_contracts(
    state: &RunState,
    pipe: &Pipeline,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for st in pipeline::flat_stages(pipe) {
        let Some(mf) = &st.merge_findings else {
            continue;
        };
        if mf.from.is_empty() {
            anyhow::bail!(
                "stage `{}`: merge_findings.from must name at least one producer stage",
                st.name
            );
        }
        let mut input_flags = String::new();
        for producer in &mf.from {
            if pipeline::find_stage(pipe, producer).is_none() {
                anyhow::bail!(
                    "stage `{}`: merge_findings.from references unknown stage `{producer}`",
                    st.name
                );
            }
            let path = plan_artifact_path(state, pipe, producer, "findings");
            input_flags.push_str(&format!(" --label {producer}=\"{}\"", path.display()));
        }
        let out_path = plan_artifact_path(state, pipe, &st.name, "findings");
        let contract = format!(
            "\n\n## Verdict — produced mechanically, not by you\n\nThis stage's verdict (`severity_counts`, `has_blocking_findings`, `checks[]`) is computed by a tool, not synthesized by you. Run this FIRST, exactly:\n\n```\naidp findings merge{input_flags} --out \"{out}\"\n```\n\nThen re-read the file it wrote at `{out}` and write `summary.md` from it. Do NOT recompute or hand-edit `severity_counts`, `has_blocking_findings`, or `checks[]` — the tool already wrote the final file; copy `has_blocking_findings` verbatim into your reply's `signals.has_blocking_findings`, and `checks[]`'s skip-count into `signals.unevaluated_checks` (set `signals.has_harness_gap: true` whenever that count is > 0). A named axis whose findings.json is missing, unreadable, or invalid JSON does NOT fail this command — the tool degrades that one axis into its own `skip` check instead and names it in the command's own JSON output under `degraded_axes`; read that field so your `summary.md` names the degraded axis instead of guessing from free text. The command still exits non-zero — and THAT is a `blocked` reply naming the command's error, never an invented verdict — only when an axis's severity_counts contradicts its own findings[]: that axis actively misreported its own tally, which is never something to paper over.\n",
            out = out_path.display(),
        );
        map.entry(st.name.clone())
            .and_modify(|c| c.push_str(&contract))
            .or_insert(contract);
    }
    Ok(map)
}

/// For every stage whose `inputs.axis` is one of the five review axes, annex to its prompt the
/// NAMED list of controls (`checks[].id`) it is expected to self-report on.
pub(super) fn review_contracts(pipe: &Pipeline) -> BTreeMap<String, String> {
    const DIMENSION_CHECKS: &[(&str, &str)] = &[
        ("C-diff-read", "Read the full diff for every file in scope (not just a summary of it)."),
        ("C-tests-exist", "Checked whether tests exist covering the changed behavior."),
        ("C-tests-run", "Actually ran (or confirmed CI ran) the relevant tests — not merely read them."),
        ("C-boundary-validation", "Checked input/boundary validation at the entry points this diff touches."),
        ("C-severity-self-check", "Recomputed severity_counts from findings[] by hand and confirmed it matches exactly before writing the file."),
    ];
    const SYNTHESIS_CHECKS: &[(&str, &str)] = &[
        ("C-merge-tool-invoked", "Ran `aidp findings merge` to produce findings.json rather than hand-writing the synthesis."),
        ("C-merge-output-read", "Re-read the findings.json the merge tool wrote before writing summary.md."),
        ("C-verdict-not-rewritten", "Did not recompute or edit severity_counts / has_blocking_findings / checks[] by hand — copied them verbatim from the merge tool's output."),
    ];
    const AXIS_OUTPUT_VERIFICATION: &str = "\nBefore you return `status: \"completed\"`, use the Read tool to re-open the `findings.json` you just wrote, at the exact path you are about to put in `outputs.findings` — yes, even with 0 findings: a clean axis still writes `{\"findings\": [], \"checks\": [...]}` and still re-reads it back. If the Read fails, or its content doesn't match what you're about to report, you have not produced a review — return `status: \"failed\"` naming exactly what the Read reported. Never report `outputs.findings` for a path you have not just successfully re-read. A harness gap (a control you could not run) is never a reason to skip the write either: `has_harness_gap: true` together with an empty `outputs` is invalid, exactly like a fabricated path — express \"I could not run this control\" as a `checks[].status: \"skip\"` entry, with `harness_gap` naming why, inside the `findings.json` you write and re-read exactly as above.\n";
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for st in pipeline::flat_stages(pipe) {
        let Some(axis) = st
            .inputs
            .get(AXIS_INPUT)
            .and_then(|v| v.as_str())
            .and_then(ReviewAxis::parse)
        else {
            continue;
        };
        let c = match axis {
            ReviewAxis::Correctness
            | ReviewAxis::Security
            | ReviewAxis::Performance
            | ReviewAxis::Maintainability => {
                let mut c = String::from(
                    "\n\n## Controls you must self-report (`checks[]`)\n\nYour findings.json's `checks[]` MUST report on exactly these, by id — one entry per control below, `status` one of `pass|fail|skip`. A control you did not actually execute is `skip` with a non-null `harness_gap` — NEVER `pass`: `pass` means you ran it and it is green.\n\n",
                );
                for (id, desc) in DIMENSION_CHECKS {
                    c.push_str(&format!("- `{id}` — {desc}\n"));
                }
                c.push_str(AXIS_OUTPUT_VERIFICATION);
                c
            }
            ReviewAxis::Synthesis => {
                let mut c = String::from(
                    "\n\n## Controls you must satisfy (self-discipline, not a `checks[]` entry)\n\nYour findings.json is written by `aidp findings merge`, not by you — never add, remove, or edit its `checks[]`. Before you write `summary.md`, confirm you actually did each of these (name any you skipped in your `summary`, and set `signals.has_harness_gap: true` if `aidp findings merge` itself failed):\n\n",
                );
                for (id, desc) in SYNTHESIS_CHECKS {
                    c.push_str(&format!("- `{id}` — {desc}\n"));
                }
                c
            }
        };
        map.insert(st.name.clone(), c);
    }
    map
}
