use super::js_str;
use crate::pipeline::Pipeline;
use crate::plan;
use std::collections::{BTreeMap, BTreeSet};

/// The union of task kinds this pipeline's `plan_tasks:` stages claim, or `None` when some stage
/// claims every kind (no `kinds:` filter).
pub(super) fn claimed_kinds(pipe: &Pipeline) -> Option<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    for st in &pipe.stages {
        let Some(pt) = &st.plan_tasks else { continue };
        if pt.kinds.is_empty() {
            return None;
        }
        set.extend(pt.kinds.iter().cloned());
    }
    Some(set)
}

/// Which of `skills` a task of the given `kind` should load, per `plan_tasks.skills_by_kind`.
pub(super) fn restrict_skills_for_kind<'a>(
    skills: &'a [String],
    by_kind: &BTreeMap<String, Vec<String>>,
    kind: &str,
) -> BTreeSet<&'a str> {
    let all: BTreeSet<&str> = skills.iter().map(String::as_str).collect();
    if by_kind.is_empty() {
        return all;
    }
    match by_kind.get(kind) {
        None => all,
        Some(allowed) => {
            let allowed_set: BTreeSet<&str> = allowed.iter().map(String::as_str).collect();
            let mentioned: BTreeSet<&str> =
                by_kind.values().flatten().map(String::as_str).collect();
            all.into_iter()
                .filter(|s| allowed_set.contains(s) || !mentioned.contains(s))
                .collect()
        }
    }
}

/// The skill a line of the "## Skills" section names, if it is one: either a `- \`name\` → path`
/// entry or a mandatory `- Skill("name")` call.
pub(super) fn extract_skill_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix("- Skill(\"") {
        let end = rest.find('"')?;
        return Some(&rest[..end]);
    }
    let rest = t.strip_prefix("- `")?;
    let end = rest.find('`')?;
    Some(&rest[..end])
}

/// Drop the skill-list lines in `base` for skills that are in `all` but not in `keep`.
pub(super) fn filtered_base(base: &str, keep: &BTreeSet<&str>, all: &BTreeSet<&str>) -> String {
    base.split('\n')
        .filter(|line| {
            !matches!(extract_skill_name(line), Some(name) if all.contains(name) && !keep.contains(name))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Progress label for one task: `implement · T2 Wire FooRepository`.
pub(super) fn task_label(stage: &str, t: &plan::Task) -> String {
    let mut title = t.title.trim().to_string();
    if title.chars().count() > 48 {
        title = title.chars().take(45).collect::<String>() + "…";
    }
    js_str(&format!("{stage} · {} {title}", t.id))
}

/// The task-specific section appended to the stage prompt.
pub(super) fn task_section(sched: &plan::Schedule, t: &plan::ScheduledTask, sole: bool) -> String {
    let task = &t.task;
    let mut s = String::new();
    s.push_str(&format!(
        "\n\n## Plan task `{id}` — {title}\n\n",
        id = task.id,
        title = task.title
    ));
    s.push_str(&format!(
        "kind: `{kind}`{risk}{us}\n\n",
        kind = task.kind,
        risk = task
            .risk
            .as_deref()
            .map(|r| format!(" · risk: `{r}`"))
            .unwrap_or_default(),
        us = if task.jira_keys.is_empty() {
            String::new()
        } else {
            format!(" · covers: {}", task.jira_keys.join(", "))
        },
    ));
    if sole {
        s.push_str("`aidp` compiled the plan's task graph and this stage resolved to a SINGLE task — so you also own this stage's declared outputs and its reply envelope. Do the task, then produce them.\n\n");
    } else {
        s.push_str(&format!(
            "`aidp` compiled the plan's task graph into {n} task sub-agents for this stage, up to {c} running at the same time in the **same working tree**. You are ONE of them: do only this task. A separate merge sub-agent runs afterwards and owns the stage's declared outputs — you do not produce them.\n\n",
            n = sched.tasks.len(),
            c = sched.effective_parallel(),
        ));
    }

    s.push_str("### What to do\n\n");
    s.push_str(task.prompt.trim());
    s.push_str("\n\n");

    if !task.instructions.is_empty() {
        s.push_str("### Steps\n\n");
        for (i, step) in task.instructions.iter().enumerate() {
            s.push_str(&format!("{}. {}\n", i + 1, step));
        }
        s.push('\n');
    }

    s.push_str("### Files you own (your write set)\n\n");
    if task.files.is_empty() {
        s.push_str("_The plan declared none._ Stay inside the files this task obviously needs, and list anything else you had to touch under `follow_ups`.\n\n");
    } else {
        for f in &task.files {
            s.push_str(&format!("- `{f}`\n"));
        }
        s.push_str("\nCreate or modify **only** these. Another task may be writing its own files right now, so a write outside this list can silently destroy a sibling's work.\n\n");
    }

    if !task.reads.is_empty() {
        s.push_str("### Read for context (do not modify)\n\n");
        for f in &task.reads {
            s.push_str(&format!("- `{f}`\n"));
        }
        s.push('\n');
    }

    if !task.verification.trim().is_empty() {
        s.push_str("### Verification\n\n");
        s.push_str(task.verification.trim());
        s.push_str("\n\n");
    }

    if !task.acceptance.is_empty() {
        s.push_str("### Acceptance criteria\n\n");
        for a in &task.acceptance {
            s.push_str(&format!("- {a}\n"));
        }
        s.push('\n');
    }

    if !task.notes.trim().is_empty() {
        s.push_str("### Notes\n\n");
        s.push_str(task.notes.trim());
        s.push_str("\n\n");
    }

    if !sole {
        s.push_str("### Ordering\n\n");
        if t.deps.is_empty() {
            s.push_str("- No declared dependency: nothing in the plan's `needs` blocks you.\n");
        } else {
            for d in &t.deps {
                s.push_str(&format!(
                    "- after `{d}` — dependency declared by the plan\n"
                ));
            }
        }
        s.push_str(&format!(
            "- wave {} of {} (informational: dispatch is dependency-driven, not wave-gated).\n\n",
            t.wave + 1,
            sched.waves.len()
        ));

        if !t.mutex_after.is_empty() {
            s.push_str("### Serialized after (file-lock only — not a dependency)\n\n");
            for d in &t.mutex_after {
                if let Some(x) = sched
                    .serializations
                    .iter()
                    .find(|x| x.after == task.id && x.before == *d)
                {
                    s.push_str(&format!(
                        "- after `{d}` — ordering inserted by `aidp`: both tasks write {}\n",
                        x.files
                            .iter()
                            .map(|f| format!("`{f}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            s.push_str("\n`aidp` only guarantees the working tree is not raced — it does NOT check that the task(s) above actually succeeded. If one failed, verify the state of the shared file(s) before you edit them; you may be picking up work left half-done.\n\n");
        }

        s.push_str("### Hands off — the merge step owns these\n\n");
        if sched.shared_files.is_empty() {
            s.push_str("The plan declared no shared file.\n");
        } else {
            for f in &sched.shared_files {
                s.push_str(&format!("- `{f}`\n"));
            }
        }
        s.push_str("\nIf this task needs a change in one of them — or in any file outside your write set (DI/module registration, project or serverless entry, `.sln`, shared config) — do **not** edit it. Describe the exact required change in `follow_ups`, precisely enough to apply blind, and the merge sub-agent will do it once for every task.\n");
    }
    s
}

/// The merge sub-agent's section.
pub(super) fn merge_section(merge_hint: Option<&str>, sched: &plan::Schedule) -> String {
    let mut s = String::new();
    s.push_str("\n\n## Plan merge — you are the LAST step of this stage\n\n");
    s.push_str(&format!(
        "`aidp` compiled the planner's task graph into {n} task sub-agents which have ALREADY done their work in THIS working tree (their envelopes are listed at the end of this prompt). Do NOT redo any of it.\n\nYour job, in order:\n\n",
        n = sched.tasks.len()
    ));
    s.push_str("1. **Apply every deferred shared-file change.** The tasks were forbidden from touching shared files and instead described the change in their `follow_ups`. Apply them all, coherently");
    if sched.shared_files.is_empty() {
        s.push_str(".\n");
    } else {
        s.push_str(&format!(
            ". The plan's shared files are: {}.\n",
            sched
                .shared_files
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    s.push_str("2. **Make the combined result coherent.** Build the project and run the test suite; fix cross-task breakage (a task that could not see its siblings' code may have guessed a signature).\n");
    s.push_str("3. **Produce this stage's declared outputs** covering ALL tasks, and one reply envelope for the whole stage. A task that returned `failed` must be reported in your `summary` — do not paper over it.\n");
    if let Some(hint) = merge_hint {
        s.push_str("\n### Stage-specific merge instructions\n\n");
        s.push_str(hint.trim());
        s.push('\n');
    }
    s
}

/// The skills a task of `kind` is told to load, in the stage's order.
pub(super) fn task_skills(
    skills: &[String],
    by_kind: &BTreeMap<String, Vec<String>>,
    kind: &str,
) -> Vec<String> {
    let keep = restrict_skills_for_kind(skills, by_kind, kind);
    skills
        .iter()
        .filter(|s| keep.contains(s.as_str()))
        .cloned()
        .collect()
}
