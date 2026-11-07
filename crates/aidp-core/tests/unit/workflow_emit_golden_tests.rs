//! Golden corpus for the Workflow emitter: every discovered template is compiled over a fixed
//! input matrix and compared with committed snapshots. Set `AIDP_UPDATE_GOLDEN=1` to rewrite them.

use crate::clock::FixedClock;
use crate::cost::PriceTable;
use crate::exec_plan::test_support::{CaseCollector, FixturePlanSource, GoldenCase, GoldenMeta};
use crate::git::RecordingGit;
use crate::id::FixedIdGen;
use crate::models::case_task::CaseTask;
use crate::models::enums::CaseKind;
use crate::models::port::PlanSource;
use crate::models::run_state::RunPorts;
use crate::pipeline;
use crate::plan::{self, Plan};
use crate::run::{init_with, record_resume_with, RunState};
use crate::skills;
use crate::workflow_emit::{compile_with_source, Emitted};
use aidp_testkit::EnvGuard;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tempfile::TempDir;

const UPDATE_ENV: &str = "AIDP_UPDATE_GOLDEN";
const UPDATE_ENABLED: &str = "1";
const FIXED_RUN_ID: &str = "20260101-000000-golden";
const FIXED_TIME: &str = "2026-01-01T00:00:00+00:00";
const WORKSPACE_PLACEHOLDER: &str = "<WORKSPACE>";
const HOME_PLACEHOLDER: &str = "<HOME>";
const PLACEHOLDERS: [&str; 2] = [WORKSPACE_PLACEHOLDER, HOME_PLACEHOLDER];
const HOME_VARIABLES: [&str; 2] = ["USERPROFILE", "HOME"];
const SKILLS_RELATIVE_DIR: &str = ".claude/skills";
const SKILL_FILE: &str = "SKILL.md";
const SKILL_STUB: &str = "stub skill
";
const SNAPSHOT_RELATIVE_DIR: &str = "tests/snapshots/workflow_emit_golden";
const PLAN_FIXTURE_RELATIVE_PATH: &str = "tests/fixtures/workflow_emit_golden/plan.json";
const TEMPLATE_EXTENSION: &str = "yaml";
const SCRIPT_EXTENSION: &str = "js";
const META_EXTENSION: &str = "meta.json";
const CASE_SEPARATOR: &str = "__";
const TRACK_INPUT: &str = "track";
const STACK_INPUT: &str = "stack";
const TRACK_PREFIX: &str = "track";
const STACK_PREFIX: &str = "stack";
const AUTO_VALUE: &str = "auto";
const KIND_ENUM: &str = "enum";
const KIND_BOOL: &str = "bool";
const KIND_LIST: &str = "list";
const KIND_INTEGER: &str = "int";
const PLACEHOLDER_STRING: &str = "golden-value";
const PLACEHOLDER_ITEM: &str = "GOLDEN-1";
const CASE_DEFAULT: &str = "default";
const CASE_PLAN: &str = "plan";
const CASE_APPROVED_GATE: &str = "approved-gate";
const CASE_CROSSED_COMPOSITES: &str = "crossed-composites";
const MAX_WORKERS: usize = 4;
const MAX_COMPOSITE_PASSES: usize = 32;
const COMPILED_AT_KEY: &str = "compiled_at";
const TIMESTAMP_PLACEHOLDER: &str = "<TIMESTAMP>";
const DOUBLE_BACKSLASH: &str = "\\\\";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn snapshot_dir() -> PathBuf {
    crate_dir().join(SNAPSHOT_RELATIVE_DIR)
}

fn fixture_plan() -> Plan {
    plan::load(&crate_dir().join(PLAN_FIXTURE_RELATIVE_PATH)).expect("fixture plan loads")
}

fn discovered_templates() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(aidp_testkit::fixtures::templates_dir())
        .expect("templates directory is readable")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|ext| ext.to_str()) == Some(TEMPLATE_EXTENSION)
        })
        .collect();
    paths.sort();
    paths
}

fn base_inputs(pipe: &pipeline::Pipeline) -> BTreeMap<String, Value> {
    let mut inputs = BTreeMap::new();
    for (name, spec) in &pipe.inputs {
        if !spec.required {
            continue;
        }
        let value = match spec.kind.as_str() {
            KIND_ENUM => Value::from(spec.values.first().cloned().unwrap_or_default()),
            KIND_BOOL => Value::from(false),
            KIND_LIST => Value::from(vec![PLACEHOLDER_ITEM]),
            KIND_INTEGER => Value::from(1),
            _ => Value::from(PLACEHOLDER_STRING),
        };
        inputs.insert(name.clone(), value);
    }
    inputs
}

fn enum_values(pipe: &pipeline::Pipeline, input: &str) -> Vec<String> {
    pipe.inputs
        .get(input)
        .filter(|spec| spec.kind == KIND_ENUM)
        .map(|spec| spec.values.clone())
        .unwrap_or_default()
}

/// Runs `action` with deterministic clock, id, git and price ports.
fn with_fixed_ports<T>(action: impl FnOnce(&RunPorts) -> T) -> T {
    let clock = FixedClock::from_rfc3339(FIXED_TIME);
    let ids = FixedIdGen(FIXED_RUN_ID.to_string());
    let git = RecordingGit::new();
    let prices = PriceTable::from_env_or_embedded();
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    action(&ports)
}

fn init_state(
    template: &Path,
    workspace: &Path,
    inputs: &BTreeMap<String, Value>,
) -> anyhow::Result<RunState> {
    let inputs_json = Value::Object(inputs.clone().into_iter().collect());
    with_fixed_ports(|ports| init_with(template, workspace, inputs_json, None, ports))
}

fn approve_gate(state: &mut RunState, gate: &str) {
    with_fixed_ports(|ports| {
        record_resume_with(state, None, &[gate.to_string()], None, None, ports)
            .expect("gate approval is recorded")
    });
}

fn cross_boundary(state: &mut RunState, composite: &str) {
    with_fixed_ports(|ports| {
        record_resume_with(state, None, &[], None, Some(composite.to_string()), ports)
            .expect("boundary crossing is recorded")
    });
}

fn compile_state(state: &RunState, plans: &dyn PlanSource) -> anyhow::Result<Emitted> {
    let pipe = pipeline::load(&state.pipeline_path)?;
    compile_with_source(&pipe, state, plans)
}

/// The plan schedules with their wall-clock `compiled_at` replaced by a placeholder.
fn schedules_without_timestamps(emitted: &Emitted) -> Value {
    let mut schedules = serde_json::to_value(&emitted.plan_schedules).expect("schedules serialize");
    for schedule in schedules.as_array_mut().into_iter().flatten() {
        if let Some(entry) = schedule.get_mut(COMPILED_AT_KEY) {
            *entry = Value::from(TIMESTAMP_PLACEHOLDER);
        }
    }
    schedules
}

fn meta_of(result: &anyhow::Result<Emitted>) -> GoldenMeta {
    match result {
        Ok(emitted) => GoldenMeta {
            error: None,
            warnings: emitted.warnings.clone(),
            gates: emitted.gates.clone(),
            awaiting_plan: emitted
                .awaiting_plan
                .as_ref()
                .map(|awaiting| serde_json::to_value(awaiting).expect("awaiting plan serializes")),
            awaiting_composite: emitted.awaiting_composite.as_ref().map(|awaiting| {
                serde_json::to_value(awaiting).expect("awaiting composite serializes")
            }),
            plan_schedules: schedules_without_timestamps(emitted),
        },
        Err(error) => GoldenMeta {
            error: Some(format!("{error:#}")),
            warnings: Vec::new(),
            gates: Vec::new(),
            awaiting_plan: None,
            awaiting_composite: None,
            plan_schedules: Value::Null,
        },
    }
}

fn is_segment_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'<' | b'>' | b'~')
}

/// Rewrites a path following the workspace placeholder to forward slashes. Only `/` and the
/// doubled backslash of an escaped string literal count as separators.
fn unify_separators(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let Some(placeholder) = PLACEHOLDERS
            .iter()
            .find(|placeholder| text[index..].starts_with(**placeholder))
        else {
            let ch = text[index..].chars().next().expect("index is in bounds");
            out.push(ch);
            index += ch.len_utf8();
            continue;
        };
        out.push_str(placeholder);
        index += placeholder.len();
        loop {
            let rest = &text[index..];
            let next_is_segment = |offset: usize| {
                rest.as_bytes()
                    .get(offset)
                    .is_some_and(|byte| is_segment_char(*byte))
            };
            if rest.starts_with(DOUBLE_BACKSLASH) && next_is_segment(2) {
                out.push('/');
                index += 2;
            } else if rest.starts_with('/') && next_is_segment(1) {
                out.push('/');
                index += 1;
            } else if next_is_segment(0) {
                out.push(rest.as_bytes()[0] as char);
                index += 1;
            } else {
                break;
            }
        }
    }
    out
}

fn replace_root(text: &str, root: &Path, placeholder: &str) -> String {
    let raw = root.to_string_lossy().to_string();
    let doubled = raw.replace('\\', DOUBLE_BACKSLASH);
    let forward = raw.replace('\\', "/");
    text.replace(&doubled, placeholder)
        .replace(&forward, placeholder)
        .replace(&raw, placeholder)
}

fn normalize(text: &str, workspace: &Path, home: &Path) -> String {
    let replaced = replace_root(text, workspace, WORKSPACE_PLACEHOLDER);
    unify_separators(&replace_root(&replaced, home, HOME_PLACEHOLDER))
}

fn render_case(
    name: String,
    result: &anyhow::Result<Emitted>,
    workspace: &Path,
    home: &Path,
) -> GoldenCase {
    let script = match result {
        Ok(emitted) => normalize(&emitted.script, workspace, home),
        Err(_) => String::new(),
    };
    let meta_json = serde_json::to_string_pretty(&meta_of(result)).expect("meta serializes");
    GoldenCase {
        name,
        script,
        meta: format!("{}\n", normalize(&meta_json, workspace, home)),
    }
}

fn run_case(
    template: &Path,
    workspace: &Path,
    inputs: &BTreeMap<String, Value>,
    plans: &dyn PlanSource,
    approve: Option<&str>,
    cross_all_composites: bool,
) -> anyhow::Result<Emitted> {
    let mut state = init_state(template, workspace, inputs)?;
    if let Some(gate) = approve {
        approve_gate(&mut state, gate);
    }
    let mut emitted = compile_state(&state, plans)?;
    if cross_all_composites {
        for _ in 0..MAX_COMPOSITE_PASSES {
            let Some(awaiting) = emitted.awaiting_composite.clone() else {
                break;
            };
            cross_boundary(&mut state, &awaiting.composite);
            emitted = compile_state(&state, plans)?;
        }
    }
    Ok(emitted)
}

fn case_name(template: &Path, case: &str) -> String {
    let stem = template
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("template stem");
    format!("{stem}{CASE_SEPARATOR}{case}")
}

fn tasks_for(template: &Path) -> Vec<CaseTask> {
    let pipe = pipeline::load(template).expect("template loads");
    let mut tasks = vec![
        (CASE_DEFAULT.to_string(), CaseKind::Default),
        (CASE_PLAN.to_string(), CaseKind::Plan),
    ];
    for (input, prefix) in [(TRACK_INPUT, TRACK_PREFIX), (STACK_INPUT, STACK_PREFIX)] {
        for value in enum_values(&pipe, input) {
            if value != AUTO_VALUE {
                let name = format!("{prefix}-{value}");
                tasks.push((name, CaseKind::EnumValue { input, value }));
            }
        }
    }
    tasks.push((CASE_APPROVED_GATE.to_string(), CaseKind::ApprovedGate));
    tasks.push((
        CASE_CROSSED_COMPOSITES.to_string(),
        CaseKind::CrossedComposites,
    ));
    tasks
        .into_iter()
        .map(|(name, kind)| CaseTask {
            template: template.to_path_buf(),
            name,
            kind,
        })
        .collect()
}

/// Runs one case in its own workspace; the approved-gate case yields nothing when no gate exists.
fn run_task(task: &CaseTask, workspace: &Path, home: &Path) -> Option<GoldenCase> {
    let template = task.template.as_path();
    let pipe = pipeline::load(template).expect("template loads");
    let base = base_inputs(&pipe);
    let with_plan = FixturePlanSource {
        plan: Some(fixture_plan()),
    };
    let without_plan = FixturePlanSource { plan: None };
    let result = match &task.kind {
        CaseKind::Default => run_case(template, workspace, &base, &without_plan, None, false),
        CaseKind::Plan => run_case(template, workspace, &base, &with_plan, None, false),
        CaseKind::EnumValue { input, value, .. } => {
            let mut inputs = base.clone();
            inputs.insert(input.to_string(), Value::from(value.clone()));
            run_case(template, workspace, &inputs, &with_plan, None, false)
        }
        CaseKind::ApprovedGate => {
            let planned = run_case(template, workspace, &base, &with_plan, None, false);
            let gate = planned
                .ok()
                .and_then(|emitted| emitted.gates.first().cloned())?;
            run_case(template, workspace, &base, &with_plan, Some(&gate), false)
        }
        CaseKind::CrossedComposites => run_case(template, workspace, &base, &with_plan, None, true),
    };
    Some(render_case(
        case_name(template, &task.name),
        &result,
        workspace,
        home,
    ))
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(MAX_WORKERS)
}

/// Runs every task on a bounded pool of scoped threads and returns one slot per task, in order.
fn run_tasks(tasks: &[CaseTask], sandbox: &Path, home: &Path) -> Vec<Option<GoldenCase>> {
    let next_task = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<GoldenCase>>> = Mutex::new(tasks.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..worker_count())
            .map(|_| {
                scope.spawn(|| loop {
                    let index = next_task.fetch_add(1, Ordering::SeqCst);
                    let Some(task) = tasks.get(index) else {
                        break;
                    };
                    let workspace = sandbox.join(format!("ws{index}"));
                    std::fs::create_dir_all(&workspace).expect("workspace is created");
                    let case = run_task(task, &workspace, home);
                    slots.lock().expect("slots lock")[index] = case;
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("case worker");
        }
    });
    slots.into_inner().expect("slots lock")
}

/// Drops, per template and in task order, any case identical to one already kept.
fn dedupe_per_template(tasks: &[CaseTask], rendered: Vec<Option<GoldenCase>>) -> Vec<GoldenCase> {
    let mut kept = Vec::new();
    let mut collector = CaseCollector::default();
    let mut current: Option<&Path> = None;
    for (task, case) in tasks.iter().zip(rendered) {
        if current != Some(task.template.as_path()) {
            kept.append(&mut collector.cases);
            current = Some(task.template.as_path());
        }
        if let Some(case) = case {
            collector.push(case);
        }
    }
    kept.append(&mut collector.cases);
    kept
}

fn snapshot_path(name: &str, extension: &str) -> PathBuf {
    snapshot_dir().join(format!("{name}.{extension}"))
}

fn expected_files(cases: &[GoldenCase]) -> BTreeSet<String> {
    cases
        .iter()
        .flat_map(|case| {
            [
                format!("{}.{SCRIPT_EXTENSION}", case.name),
                format!("{}.{META_EXTENSION}", case.name),
            ]
        })
        .collect()
}

fn write_snapshots(cases: &[GoldenCase]) {
    let dir = snapshot_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("snapshot dir is created");
    for case in cases {
        std::fs::write(snapshot_path(&case.name, SCRIPT_EXTENSION), &case.script)
            .expect("script snapshot is written");
        std::fs::write(snapshot_path(&case.name, META_EXTENSION), &case.meta)
            .expect("meta snapshot is written");
    }
}

fn read_normalized(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.replace("\r\n", "\n"))
}

fn mismatches(cases: &[GoldenCase]) -> Vec<String> {
    let mut problems = Vec::new();
    for case in cases {
        for (extension, actual) in [
            (SCRIPT_EXTENSION, &case.script),
            (META_EXTENSION, &case.meta),
        ] {
            let path = snapshot_path(&case.name, extension);
            match read_normalized(&path) {
                None => problems.push(format!("missing snapshot {}", path.display())),
                Some(expected) if &expected != actual => {
                    problems.push(format!("changed snapshot {}", path.display()))
                }
                Some(_) => {}
            }
        }
    }
    let expected = expected_files(cases);
    if let Ok(entries) = std::fs::read_dir(snapshot_dir()) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            if !expected.contains(&file) {
                problems.push(format!("stale snapshot {file}"));
            }
        }
    }
    problems
}

/// Creates a stub `SKILL.md` for every skill any template declares, so prompts never depend on the
/// skills installed on the machine running the test.
fn stub_declared_skills(home: &Path, templates: &[PathBuf]) {
    let skills_root = home.join(SKILLS_RELATIVE_DIR);
    for template in templates {
        let pipe = pipeline::load(template).expect("template loads");
        for missing in skills::unresolved(&pipe, &[]) {
            let dir = skills_root.join(&missing.skill);
            std::fs::create_dir_all(&dir).expect("stub skill dir is created");
            std::fs::write(dir.join(SKILL_FILE), SKILL_STUB).expect("stub skill is written");
        }
    }
}

#[test]
fn it_every_template_compiles_to_its_golden_workflow() {
    let sandbox = TempDir::new().expect("temp sandbox");
    let home = sandbox.path().join("home");
    let templates = discovered_templates();
    stub_declared_skills(&home, &templates);
    let home_value = home.to_string_lossy().to_string();
    let _env = EnvGuard::with(&HOME_VARIABLES.map(|name| (name, Some(home_value.as_str()))));

    let tasks: Vec<CaseTask> = templates
        .iter()
        .flat_map(|template| tasks_for(template))
        .collect();
    let cases = dedupe_per_template(&tasks, run_tasks(&tasks, sandbox.path(), &home));
    assert!(!cases.is_empty(), "no template was discovered");

    if std::env::var(UPDATE_ENV).is_ok_and(|value| value == UPDATE_ENABLED) {
        write_snapshots(&cases);
        return;
    }
    let problems = mismatches(&cases);
    assert!(
        problems.is_empty(),
        "golden workflow drift (rerun with {UPDATE_ENV}=1 to accept):
{}",
        problems.join(
            "
"
        )
    );
}
