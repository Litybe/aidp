use super::contracts::validate_tool_enforcement_against;
use super::*;
use crate::models::enums::{AgentRole, ToolEnforcement};
use crate::run;
use aidp_testkit::TempWorkspace;

const NESTED_SINK_INNER: &str = include_str!("../fixtures/workflow_emit/nested_sink_inner.yaml");
const NESTED_SINK_MID: &str = include_str!("../fixtures/workflow_emit/nested_sink_mid.yaml");
const NESTED_SINK_OUTER: &str = include_str!("../fixtures/workflow_emit/nested_sink_outer.yaml");
const GRAPHIFY_PIPE: &str = include_str!("../fixtures/skills/graphify_emit.yaml");
const GRAPHIFY_PROMPT_CALL: &str = r#"Skill(\"graphify-context\")"#;
const GRAPHIFY_OPTION_ENTRY: &str = "\"graphify-context\"";
const RUST_SKILL_ENTRY: &str = "\"rust-dev-aidp\"";
const GRAPH_JSON_PATH: &str = "/abs/graph.json";
const GRAPHIFY_OFF_TAG: &str = "graphify-off";
const GRAPHIFY_ON_TAG: &str = "graphify-on";
const PLAIN_STAGE: &str = "plain";
const SKIPPED_GRAPH_REASON: &str = "test";

/// Writes `yaml` into a fresh temp workspace and runs `run::init` on it.
/// The returned guard must outlive every use of the run directory.
fn init_run_in_workspace(
    tag: &str,
    yaml: &str,
    inputs: serde_json::Value,
) -> (TempWorkspace, RunState) {
    let ws_guard = TempWorkspace::new(tag);
    let ws = ws_guard.path();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, yaml).unwrap();
    let st = run::init(&p, ws, inputs, None).unwrap();
    (ws_guard, st)
}

// A composite whose child's OWN sink stage is itself composite must still
// give the downstream stage a resolvable `up([...])` argument. `up()`
// no-ops on a name missing from `results`, so a phantom terminal (the
// pre-fix `compose.rs` behaviour of reading the child's RAW stage list)
// silently dropped the "## Upstream outputs" section from the prompt
// instead of failing — this pins the emitted call site.
#[test]
fn nested_composite_sink_is_emitted_as_a_resolvable_upstream() {
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("inner.yaml"), NESTED_SINK_INNER).unwrap();
    std::fs::write(ws.join("mid.yaml"), NESTED_SINK_MID).unwrap();
    let outer = ws.join("outer.yaml");
    std::fs::write(&outer, NESTED_SINK_OUTER).unwrap();

    let mut st = run::init(&outer, &ws, serde_json::json!({}), None).unwrap();
    cross_all_composite_boundaries(&mut st);
    let js = compile(&st).unwrap().script;

    assert!(
        js.contains(r#"up(["c__m_sink__i_last"])"#),
        "emitted script must pass the real sink to up(), got:
{js}"
    );
    assert!(
        !js.contains(r#"up(["c__m_sink"])"#),
        "emitted script still references the phantom terminal:
{js}"
    );
}

const DEVISH: &str = include_str!("../fixtures/workflow_emit/devish.yaml");

#[test]
fn it_emits_meta_schema_and_helpers() {
    let (_ws, st) = init_run_in_workspace("base", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    assert!(
        js.starts_with("export const meta = {"),
        "meta must come first"
    );
    assert!(js.contains("const REPLY ="));
    assert!(js.contains("function isPause("));
    assert!(js.contains("function sig("));
    assert!(js.contains("return { status: \"done\""));
}

// F-2: an item that `pool`'s admission check never dispatched
// (`status: "cancelled"`, see `__poolBud() < floor`) is not a "success" —
// `isPause` must treat it exactly like `blocked`/`failed`, on the same
// `return true` line, so a real `needs`/mutex dependency on it is not
// silently treated as satisfied.
#[test]
fn is_pause_treats_cancelled_as_pause_worthy() {
    let (_ws, st) = init_run_in_workspace("base", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    assert!(
            js.contains(
                "if (r.status === \"blocked\" || r.status === \"failed\" || r.status === \"cancelled\") return true;"
            ),
            "isPause must treat cancelled as pause-worthy on the same line as blocked/failed:\n{js}"
        );
}

/// LOC-50: runs the emitted `isPause`/`selfAnswered` under node against
/// real envelopes, so the self-answered and blocker cases are proven on
/// behavior rather than on the source text.
fn eval_pause_helpers(case: &str) -> String {
    let start = HELPERS.find("function isPause(").unwrap();
    let end = HELPERS.find("function pauseInfo(").unwrap();
    let script = format!("let results = {{}};\n{}\n{case}\n", &HELPERS[start..end]);
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("pause.js");
    std::fs::write(&f, script).unwrap();
    let out = std::process::Command::new("node")
        .arg(&f)
        .output()
        .expect("node must be installed to run this test");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn it_is_pause_passes_a_completed_stage_whose_questions_are_all_self_answered() {
    let out = eval_pause_helpers(
        r#"const r = { status: "completed", ask_user_questions: [
                 { id: "q1", question: "A or B?", answer: "B, deferred to plan" },
                 { id: "q2", question: "Scope?", answer: "Rust only" } ] };
               results.study = r;
               console.log(JSON.stringify([isPause(r, false), isPause(r, true), selfAnswered().map(q => q.stage + ":" + q.id)]));"#,
    );
    assert_eq!(out, r#"[false,false,["study:q1","study:q2"]]"#);
}

#[test]
fn it_is_pause_still_halts_on_an_unanswered_question() {
    let out = eval_pause_helpers(
        r#"const cases = [
                 { status: "completed", ask_user_questions: [ { id: "q1", question: "A?", answer: "yes" }, { id: "q2", question: "B?" } ] },
                 { status: "completed", ask_user_questions: [ { id: "q1", question: "A?", answer: "   " } ] },
                 { status: "needs_review", ask_user_questions: [ { id: "q1", question: "A?", answer: "yes" } ] },
                 { status: "completed", ask_user_questions: [] },
               ];
               console.log(JSON.stringify(cases.map(c => isPause(c, true))));"#,
    );
    assert_eq!(out, "[true,true,true,false]");
}

#[test]
fn it_run_task_deps_loop_treats_cancelled_as_incomplete() {
    assert!(
            PLAN_HELPERS.contains(
                "if (!env || env.status === \"blocked\" || env.status === \"failed\" || env.status === \"cancelled\") {\n      return { id: id, skipped: true,"
            ),
            "runTask's deps loop must treat a cancelled upstream task as incomplete:\n{PLAN_HELPERS}"
        );
}

#[test]
fn it_run_task_mutex_loop_warns_on_cancelled_predecessor() {
    assert!(
            PLAN_HELPERS.contains(
                "if (!env || env.status === \"blocked\" || env.status === \"failed\" || env.status === \"cancelled\") {\n      mutexWarnings.push("
            ),
            "runTask's mutex loop must warn on a cancelled predecessor:\n{PLAN_HELPERS}"
        );
}

// Two init_run_in_workspace("base") calls must yield two distinct, both-live directories.
// This test verifies that TempWorkspace generates unique paths even with the same tag.
#[test]
fn two_setup_calls_with_same_tag_yield_distinct_directories() {
    let (_ws1, st1) =
        init_run_in_workspace("base", DEVISH, serde_json::json!({"track": "standard"}));
    let (_ws2, st2) =
        init_run_in_workspace("base", DEVISH, serde_json::json!({"track": "standard"}));

    // Verify that the directories are different
    assert_ne!(
        st1.run_dir, st2.run_dir,
        "Two init_run_in_workspace(\"base\") calls must yield distinct directories"
    );

    // Verify that both directories exist at the same time
    assert!(
        st1.run_dir.is_dir(),
        "First setup's directory must exist: {:?}",
        st1.run_dir
    );
    assert!(
        st2.run_dir.is_dir(),
        "Second setup's directory must exist: {:?}",
        st2.run_dir
    );
}

// C5 (Lot 3, Capitalisation): the guard helper lives in HELPERS — the one
// block every template always emits — so it must be present regardless of
// which of RETRY/FANOUT/PLAN this pipeline needs, and `fanin` (the
// highest-priority recombination site: it carries a model's own
// summary/follow_ups from every shard) must route through it.
#[test]
fn it_guard_helper_is_always_emitted_and_routes_fanin() {
    let (_ws, st) = init_run_in_workspace(
        "guard-helper-devish",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("function __guard("),
        "guard helper must always be emitted:\n{js}"
    );
    assert!(js.contains("function __zw("));
    assert!(js.contains("const __GP ="));
    assert!(js.contains("const __guardFindings = [];"));

    const FO: &str = include_str!(
        "../fixtures/workflow_emit/guard_helper_is_always_emitted_and_routes_fanin_fo.yaml"
    );
    let (_ws, st2) = init_run_in_workspace("guard-helper-fanin", FO, serde_json::json!({}));
    let js2 = emit(&st2).unwrap();
    assert!(
        js2.contains("__guard(JSON.stringify({ status: r.status"),
        "fanin must route each shard's JSON through __guard:\n{js2}"
    );
}

// The one test that protects the sandbox constraint without launching a
// run: the Workflow sandbox has no clock and no entropy source, so the
// emitted guard JS must never reach for either.
#[test]
fn guard_helper_uses_no_clock_and_no_random() {
    let (_ws, st) = init_run_in_workspace(
        "guard-helper-no-clock",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("Date.now"),
        "guard JS must never call Date.now:\n{js}"
    );
    assert!(
        !js.contains("Math.random"),
        "guard JS must never call Math.random:\n{js}"
    );
    assert!(
        !js.contains("new Date("),
        "guard JS must never construct a Date:\n{js}"
    );
}

// safeAgent must guard envelope shape, not just transport: a truthy but
// malformed envelope (e.g. `signals` nested inside a string `summary`)
// is rejected with signals.malformed_envelope instead of passing through,
// so a blocked queue is diagnosable instead of silently stuck. This must
// not regress the majority case of a conformant envelope with no signals.
#[test]
fn safe_agent_guards_envelope_shape() {
    let (_ws, st) = init_run_in_workspace(
        "shape-guard",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(js.contains("function safeAgent("));
    assert!(js.contains("malformed_envelope"));
    assert!(js.contains("function envelopeShapeError("));
}

// T02: the early-return shapes must all carry the full `results` map, not
// just `Object.keys(results)` — otherwise `reconcile` sees no results on a
// gated run and reports 0/N for work that was actually done and paid for.
// DEVISH has no `plan_tasks:` stage, so `emit_plan_stage` never fires for it
// and the `awaiting_plan` shape is unreachable here: it is asserted on the
// PLANNED fixture instead, by `awaiting_plan_carries_results`.
#[test]
fn awaiting_gate_carries_results() {
    let (_ws, st) =
        init_run_in_workspace("gate-results", DEVISH, serde_json::json!({"track": "deep"}));
    let js = emit(&st).unwrap();
    let line_has_both = |needle: &str| {
        js.lines()
            .any(|line| line.contains(needle) && line.contains("results: results"))
    };
    assert!(
        line_has_both("status: \"awaiting_gate\""),
        "awaiting_gate literal must carry results: results on the same line"
    );
    assert!(
        line_has_both("status: \"paused\""),
        "paused literal (pauseInfo) must carry results: results on the same line"
    );
}

// C1: no template may emit a bare, unwrapped `agent()` call — every dispatch
// site (sequential, fan-out, plan-driven) must go through `safeAgent`, whose
// own body is the sole remaining lowercase `agent(` call site. This compiles
// EVERY template `discover()` finds (root count intentionally not hard-coded
// here — the fixture below tracks whichever templates currently declare a
// required input with no default).
#[test]
fn it_no_bare_agent_in_any_template() {
    let skill_root = aidp_testkit::fixtures::skill_root();
    let tpl_dir = skill_root.join("templates");
    let discovered = pipeline::discover(&[tpl_dir.as_path()]).expect("discover templates/");
    assert!(
        !discovered.is_empty(),
        "expected to discover templates under {}",
        tpl_dir.display()
    );

    let fixture_path = &aidp_testkit::fixtures::template_inputs();
    let fixture: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display())),
    )
    .expect("template-inputs.json must be valid JSON");

    for (id, path) in discovered {
        let inputs = fixture
            .get(&id)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let ws_guard = tempfile::tempdir().unwrap();
        let ws = ws_guard.path().to_path_buf();
        std::fs::create_dir_all(&ws).unwrap();
        let st =
            run::init(&path, &ws, inputs, None).unwrap_or_else(|e| panic!("run::init({id}): {e}"));
        let js = emit(&st).unwrap_or_else(|e| panic!("emit({id}): {e}"));
        // Plan-driven templates: pass 1 (no plan.json on disk yet) is the
        // only script compiled without external state, and the only one
        // whose dispatch sites are all reachable from a fresh run.
        assert_eq!(
                js.matches("agent(").count(),
                1,
                "le seul `agent(` en minuscule doit etre l appel interne du corps de agentOnce (appele par safeAgent) (template: {id})"
            );
        assert!(
            js.contains("function safeAgent("),
            "missing safeAgent helper (template: {id})"
        );
    }
}

// 12) R-POL-1: `policy:` must not just parse and validate — it has to
// actually land in the emitted sub-agent prompt, or the whole migration
// is theater. Copies `no_bare_agent_in_any_template`'s pattern (a real
// `run::init` + `emit`, not a unit test of `prompt::build_prompt_text`
// in isolation) because the composition trap T6 called out is real:
// `compose::expand` drops a CHILD pipeline's own pipeline-level fields
// when inlining it (see `expand`'s doc comment), so only a genuine
// end-to-end emission proves the migrated rule survives that path too.
// Grepped on a ONE-LINE fragment ("NOSONAR", "FLAKY") rather than a full
// sentence or the heading alone: `js_str` (`serde_json::to_string`)
// turns the prompt's real newlines into literal `\n` inside the JS
// string, so a multi-line match would need to account for that escaping,
// and the heading itself carries accents ("Règles non négociables") that
// `js_str` leaves as literal UTF-8 — both survive intact only because the
// chosen fragments sit on a single rendered line.
#[test]
fn it_migrated_doctrine_reaches_the_emitted_prompt() {
    let skill_root = aidp_testkit::fixtures::skill_root();
    let tpl_dir = skill_root.join("templates");

    let fixture_path = &aidp_testkit::fixtures::template_inputs();
    let fixture: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display())),
    )
    .expect("template-inputs.json must be valid JSON");

    // (id, a one-line fragment unique to a rule this pipeline migrated
    // into `policy:` — see templates/sonar-fix.yaml's pipeline-level
    // policy and templates/gha-fix.yaml's pipeline-level policy).
    let cases = [("sonar-fix", "NOSONAR"), ("gha-fix", "FLAKY")];

    for (id, fragment) in cases {
        let path = tpl_dir.join(format!("{id}.yaml"));
        assert!(
            path.is_file(),
            "expected template file at {}",
            path.display()
        );
        let inputs = fixture
            .get(id)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let ws_guard = tempfile::tempdir().unwrap();
        let ws = ws_guard.path().to_path_buf();
        std::fs::create_dir_all(&ws).unwrap();
        let st =
            run::init(&path, &ws, inputs, None).unwrap_or_else(|e| panic!("run::init({id}): {e}"));
        let js = emit(&st).unwrap_or_else(|e| panic!("emit({id}): {e}"));

        assert!(
            js.contains("Règles non négociables"),
            "expected the migrated policy heading in the emitted prompt for {id}:\n{js}"
        );
        assert!(
                js.contains(fragment),
                "expected the migrated rule fragment {fragment:?} in the emitted prompt for {id}:\n{js}"
            );
    }
}

// WI-P1-5: needs_review is a checkpoint — the emitted isPause halts on it…
#[test]
fn needs_review_is_a_pause() {
    let (_ws, st) = init_run_in_workspace(
        "needsreview",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("r.status === \"needs_review\" && !soft"),
        "isPause must treat needs_review as a pause"
    );
    // …but NOT for a stage the correction loop consumes: `review` reports its
    // blocking findings and the loop fixes them. Halting at `review` stranded
    // the run one step before the loop built to handle it.
    assert!(
        js.contains("if (isPause(results[\"review\"], true))"),
        "the loop-fed reviewer must not pause on needs_review:\n{js}"
    );
    assert!(
        js.contains("if (isPause(results[\"rereview\"], true))"),
        "the in-loop re-reviewer must not pause on needs_review either:\n{js}"
    );
    // The stage the loop repairs (`code`, read by `review`) and the in-loop
    // fixer are soft too: their leftover problems are the loop's job.
    assert!(
        js.contains("if (isPause(results[\"code\"], true))"),
        "the stage the loop repairs must not pause on needs_review:\n{js}"
    );
    assert!(
        js.contains("if (isPause(results[\"refix\"], true))"),
        "the in-loop fixer must not pause on needs_review:\n{js}"
    );
    // Every other stage keeps the hard checkpoint.
    assert!(
        js.contains("if (isPause(results[\"discover\"], false))"),
        "an analyzer the review does not read still pauses"
    );
    assert!(
        js.contains("if (isPause(results[\"pr\"], false))"),
        "an executor downstream of the review still pauses"
    );
}

// A test left red by the stage the loop repairs no longer halts the run: the
// stage is listed in `__REVIEWED`, and `up()` forwards its `needs_review`
// report to the reviewers as a problem to turn into a blocking finding.
#[test]
fn repaired_stage_report_reaches_the_review() {
    let (_ws, st) =
        init_run_in_workspace("repaired", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("const __REVIEWED = new Set([\"code\", \"refix\"]);"),
        "the repaired stages must be baked into the script:\n{js}"
    );
    assert!(
        js.contains("r.status === \"needs_review\" && __REVIEWED.has(n)"),
        "up() must forward a repaired stage's needs_review report:\n{js}"
    );
    assert!(
        js.contains("## Unresolved upstream problems (needs_review)"),
        "up() must frame the forwarded report for the reviewer:\n{js}"
    );
}

// No correction loop, nothing to repair: the set is empty and every
// non-reviewer keeps its hard checkpoint.
#[test]
fn no_loop_means_no_repaired_stage() {
    let (_ws, st) = init_run_in_workspace("planned-noloop", PLANNED, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("const __REVIEWED = new Set([]);"),
        "a loop-free pipeline repairs nothing:\n{js}"
    );
}

// The exemption covers every reviewer, loop-fed or not, plus the executors
// the loop repairs — reached through the rubric axes the loop's source needs.
#[test]
fn every_reviewer_is_exempt_and_the_repaired_executors() {
    const DIMS: &str = include_str!(
        "../fixtures/workflow_emit/every_reviewer_is_exempt_and_the_repaired_executors_dims.yaml"
    );
    let (_ws, st) = init_run_in_workspace("dims", DIMS, serde_json::json!({}));
    let js = emit(&st).unwrap();
    for r in [
        "review",
        "rereview",
        "review_correctness",
        "review_security",
        "sidecar_review",
    ] {
        assert!(
            js.contains(&format!("if (isPause(results[\"{r}\"], true))")),
            "reviewer `{r}` must not pause on needs_review:\n{js}"
        );
    }
    // `code` is not a reviewer, but the axes feeding the loop read it and the
    // loop repairs it, so it is soft too — as is the in-loop fixer.
    assert!(
        js.contains("if (isPause(results[\"code\"], true))"),
        "the stage the loop repairs must not pause on needs_review:\n{js}"
    );
    assert!(
        js.contains("if (isPause(results[\"refix\"], true))"),
        "the in-loop fixer must not pause on needs_review:\n{js}"
    );
}

// The case the loop-anchored exemption structurally could not cover: `audit`
// and `review` have no correction loop at all, so before the contract-aligned
// rule every rubric dimension could halt the run one step before `verify` —
// with no gate to approve past it.
#[test]
fn reviewers_are_exempt_without_any_loop() {
    const AUDITISH: &str = include_str!(
        "../fixtures/workflow_emit/reviewers_are_exempt_without_any_loop_auditish.yaml"
    );
    let (_ws, st) = init_run_in_workspace("auditish", AUDITISH, serde_json::json!({}));
    let js = emit(&st).unwrap();
    for r in ["dim_correctness", "dim_security", "verify"] {
        assert!(
            js.contains(&format!("if (isPause(results[\"{r}\"], true))")),
            "loop-free reviewer `{r}` must not pause on needs_review:\n{js}"
        );
    }
    // The analyzer and the report executor keep the hard checkpoint.
    assert!(
        js.contains("if (isPause(results[\"explore\"], false))"),
        "analyzer still pauses"
    );
    assert!(
        js.contains("if (isPause(results[\"report\"], false))"),
        "executor still pauses"
    );
}

// The exemption must survive composition. `dev-lambda-ts` / `dev-csharp` pull
// the 4 rubric axes in via `type: pipeline / uses: review`, so the emitted
// stage names are namespaced (`review__review_security`) and the loop's
// `when:` is alias-rewritten to the inner synthesis stage. A run that paused
// at `review__review_security` was the observed symptom, so pin the composed
// shape too — the flat fixture above cannot catch a namespacing regression.
#[test]
fn loop_exemption_survives_composition() {
    const CHILD: &str =
        include_str!("../fixtures/workflow_emit/loop_exemption_survives_composition_child.yaml");
    const PARENT: &str =
        include_str!("../fixtures/workflow_emit/loop_exemption_survives_composition_parent.yaml");
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("revsub.yaml"), CHILD).unwrap();
    let p = ws.join("composed.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let mut st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();
    cross_all_composite_boundaries(&mut st);
    let js = emit(&st).unwrap();
    // The synthesis AND every namespaced axis stay soft…
    for r in [
        "review__review",
        "review__review_correctness",
        "review__review_security",
        "rereview",
    ] {
        assert!(
            js.contains(&format!("if (isPause(results[\"{r}\"], true))")),
            "composed reviewer `{r}` must not pause on needs_review:\n{js}"
        );
    }
    // …and the loop still reads the alias-rewritten inner synthesis stage.
    assert!(
        js.contains("sig(\"review__review\", \"has_blocking_findings\")"),
        "loop condition must resolve to the inner synthesis stage:\n{js}"
    );
    // The composed axes' cascaded `needs: [code]` still reaches the stage the
    // loop repairs.
    assert!(
        js.contains("if (isPause(results[\"code\"], true))"),
        "the stage the composed review reads must not pause on needs_review:\n{js}"
    );
    assert!(
        js.contains("const __REVIEWED = new Set([\"code\", \"refix\"]);"),
        "composition must not hide the repaired stage:\n{js}"
    );
}

// …and the human checkpoint moves to the far side of the loop: exiting it
// with the signal still set means the corrections never converged.
#[test]
fn unconverged_loop_with_on_exhausted_continue_does_not_pause() {
    const DEVISH_CONTINUE: &str = include_str!("../fixtures/workflow_emit/devish_continue.yaml");
    let (_ws, st) = init_run_in_workspace(
        "noconvergecont",
        DEVISH_CONTINUE,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("continuing with the remaining findings open")
            && !js.contains("return pauseInfo(\"correction_loop\""),
        "on_exhausted: continue must not pause after the loop:
{js}"
    );
}

#[test]
fn unconverged_correction_loop_pauses_after_the_loop() {
    let (_ws, st) = init_run_in_workspace(
        "noconverge",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    let guard = js
        .split("results[\"review\"] = results[\"rereview\"];")
        .nth(1)
        .expect("loop must redirect");
    assert!(
        guard.contains("if (sig(\"review\", \"has_blocking_findings\")) {")
            && guard.contains("return pauseInfo(\"correction_loop\", results[\"review\"]);"),
        "an unconverged loop must pause instead of falling through to `pr`:\n{js}"
    );
}

#[test]
fn gate_true_emits_guard_and_resumes_via_args() {
    let (_ws, st) = init_run_in_workspace("gate", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    // pr is gate: true → always guarded.
    assert!(js.contains("if (!__approved.includes(\"pr\"))"));
    assert!(js.contains("gate: \"pr\""));
}

#[test]
fn input_gate_evaluated_at_compile_time() {
    // track=standard → plan's gate (track=='deep') is false → NOT a gate.
    let (_ws, st) = init_run_in_workspace(
        "plan-nogate",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("if (!__approved.includes(\"plan\"))"),
        "plan must not be gated when track=standard"
    );
    // track=deep → plan IS a gate.
    let (_ws, st2) =
        init_run_in_workspace("plan-gate", DEVISH, serde_json::json!({"track": "deep"}));
    let js2 = emit(&st2).unwrap();
    assert!(
        js2.contains("if (!__approved.includes(\"plan\"))"),
        "plan must be gated when track=deep"
    );
}

#[test]
fn static_when_false_skips_stage() {
    // track=quick → study's when is false → study omitted (logged as skip).
    let (_ws, st) =
        init_run_in_workspace("when-skip", DEVISH, serde_json::json!({"track": "quick"}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("skip `study`"),
        "study should be compile-time skipped"
    );
    assert!(
        !js.contains("results[\"study\"] = await safeAgent("),
        "skipped study should not dispatch"
    );
}

#[test]
fn correction_loop_becomes_while_on_signal() {
    let (_ws, st) = init_run_in_workspace("loop", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("sig(\"review\", \"has_blocking_findings\")"),
        "loop cond must read the reviewer signal"
    );
    assert!(
        js.contains("results[\"review\"] = results[\"rereview\"];"),
        "loop must redirect to the re-review"
    );
    assert!(js.contains("results[\"refix\"] = await safeAgent("));
}

// WI-P2-2: an unpinned stage emits a deterministic per-role default model;
// a pinned stage keeps its model.
#[test]
fn default_model_per_role() {
    let (_ws, st) =
        init_run_in_workspace("models", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    // study has no model pin and is an analyzer → sonnet default.
    assert!(
        js.contains("agentType: \"analyzer\", model: \"sonnet\""),
        "unpinned analyzer → sonnet"
    );
    // code is an executor with no pin → sonnet default.
    assert!(
        js.contains("agentType: \"executor\", model: \"sonnet\""),
        "unpinned executor → sonnet"
    );
    // plan pins opus explicitly → preserved.
    assert!(
        js.contains("agentType: \"planner\", model: \"opus\""),
        "planner keeps its pin"
    );
    assert_eq!(AgentRole::Planner.default_model().as_str(), "opus");
    assert_eq!(AgentRole::Reviewer.default_model().as_str(), "sonnet");

    // The `plan` stage above pins `model: opus` explicitly, so it alone
    // cannot prove the *default* for an unpinned planner is opus too (a
    // hardcoded "opus" with no default_model call would pass it just the
    // same). Assert the unpinned-planner and no-agent-at-all defaults
    // against a separate fixture where neither stage pins a model.
    const UNPINNED: &str =
        include_str!("../fixtures/workflow_emit/default_model_per_role_unpinned.yaml");
    let (_ws, st2) = init_run_in_workspace("models-unpinned", UNPINNED, serde_json::json!({}));
    let js2 = emit(&st2).unwrap();
    // NOTE for future field additions to `agent_opts_with_label`: these
    // assertions match a CONTIGUOUS substring (`agentType: "x", model:
    // "y"`, no field ever inserted between `agentType` and `model`).
    // `reviewer_stage_requires_signals_schema` relies on the same
    // contiguity — always emit new opts fields after `model`.
    assert!(
        js2.contains("agentType: \"planner\", model: \"opus\""),
        "unpinned planner defaults to opus:\n{js2}"
    );
    assert!(
        js2.contains("agentType: \"executor\", model: \"sonnet\""),
        "a stage with no `agent:` at all defaults to executor/sonnet:\n{js2}"
    );
}

// C10 (Lot 3, Capitalisation): THE non-regression test of this chantier.
// `settings.tool_enforcement` defaults to `"off"` — DEVISH declares no
// `tool_enforcement:` at all, so every stage must keep emitting its bare
// role. No pipeline predating C10, and no run in flight, may change
// shape just because this module now exists.
#[test]
fn tool_enforcement_off_emits_the_bare_role() {
    let (_ws, st) = init_run_in_workspace(
        "toolenf-off",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("agentType: \"analyzer\""),
        "analyzer stage must emit the bare role:\n{js}"
    );
    assert!(
        js.contains("agentType: \"planner\""),
        "planner stage must emit the bare role:\n{js}"
    );
    assert!(
        js.contains("agentType: \"executor\""),
        "executor stage must emit the bare role:\n{js}"
    );
    assert!(
            js.contains("agentType: \"reviewer\""),
            "reviewer stage must emit the bare role — NOT a restricted variant — when tool_enforcement is off (the default):\n{js}"
        );
    assert!(
            !js.contains("agentType: \"aidp-"),
            "tool_enforcement: off (the default) must never emit a restricted `aidp-*-ro` agentType (note: the script's own `meta.name` legitimately contains `aidp-` as a run-id prefix, so this checks the `agentType:` key specifically):\n{js}"
        );
}

/// Compiles a one-stage pipeline in memory and returns the stage's resolved dispatch data.
fn strict_spec(agent: &str, scope_mode: &str) -> crate::exec_plan::StageSpec {
    let yaml = format!(
        "version: 5
id: strict-test
settings: {{ tool_enforcement: strict }}
stages:
  - name: only
    agent: {agent}
    scope: {{ mode: {scope_mode} }}
"
    );
    let pipe: Pipeline = serde_yaml::from_str(&yaml).expect("pipeline parses");
    let state = RunState::default();
    let plan =
        crate::exec_plan::compile_exec_plan(&pipe, &state, &DiskPlanSource).expect("plan compiles");
    match plan.steps.into_iter().next() {
        Some(crate::exec_plan::Step::Stage(spec)) => spec,
        other => panic!("expected a single stage step, got {other:?}"),
    }
}

/// Under `strict` read-only roles get their `-ro` agent type and `executor` never does.
/// Goes through the plan and the opts renderer, not the full `compile()`.
#[test]
fn strict_emits_a_restricted_type_for_read_only_roles_only() {
    let review_opts = agent_opts(&strict_spec("reviewer", "stage_dir_only"));
    assert!(
            review_opts.contains("schema: REPLY_REVIEWER, agentType: \"aidp-reviewer-ro\""),
            "a restricted reviewer stage must still select REPLY_REVIEWER — computed on the RAW role, contiguous with the substituted agentType:
{review_opts}"
        );

    let code_opts = agent_opts(&strict_spec("executor", "production"));
    assert!(
        code_opts.contains("schema: REPLY, agentType: \"executor\""),
        "executor must stay the bare role under strict:
{code_opts}"
    );

    for scope_mode in ["stage_dir_only", "tests", "repo_wide", "custom"] {
        assert!(
            agent_opts(&strict_spec("executor", scope_mode)).contains("agentType: \"executor\""),
            "executor must never be restricted (scope_mode={scope_mode})"
        );
    }

    for role in ["analyzer", "planner"] {
        let opts = agent_opts(&strict_spec(role, "stage_dir_only"));
        assert!(
            opts.contains(&format!("agentType: \"aidp-{role}-ro\"")),
            "role={role} must resolve to its restricted variant under strict:
{opts}"
        );
    }
}

// C10 (Lot 3, Capitalisation): the REAL guardrail — `aidp workflow emit`
// must refuse to compile a `strict` pipeline while the registry lacks a
// `tools:`-bearing persona for a restricted agentType it would emit.
// `doctor`'s own `agent_toolsets` check only ever reports this; a human
// could ignore it. Uses `validate_tool_enforcement_against` (not
// `validate_tool_enforcement`) to point at a throwaway temp directory
// instead of the real, host-dependent `~/.claude/agents/`.
#[test]
fn validate_tool_enforcement_refuses_strict_when_registry_is_incomplete() {
    let strict_settings = pipeline::Settings {
        tool_enforcement: ToolEnforcement::Strict,
        ..Default::default()
    };
    let mut review = bare_test_stage("review");
    review.agent = Some("reviewer".into());
    let pipe = Pipeline {
        version: 5,
        id: "toolenf-validate".to_string(),
        name: String::new(),
        description: String::new(),
        kind: "pipeline".to_string(),
        policy: Vec::new(),
        inputs: Default::default(),
        settings: strict_settings,
        stages: vec![review],
    };

    let dir_guard = tempfile::tempdir().unwrap();
    let dir = dir_guard.path().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();

    // Empty registry: strict must be refused, naming the missing type.
    let err = validate_tool_enforcement_against(&pipe, Some(&dir))
        .expect_err("strict must be refused when the registry lacks aidp-reviewer-ro.md");
    assert!(
        err.to_string().contains("aidp-reviewer-ro"),
        "the refusal must name the missing agentType: {err}"
    );

    // Populate the registry with a `tools:`-bearing persona: now safe.
    std::fs::write(
        dir.join("aidp-reviewer-ro.md"),
        "---\nname: aidp-reviewer-ro\ntools: Read, Grep, Glob, Bash, TodoWrite\n---\nbody\n",
    )
    .unwrap();
    validate_tool_enforcement_against(&pipe, Some(&dir))
        .expect("strict must be accepted once the registry has the needed persona");

    // `off` (the default) never even looks at the registry.
    let mut off_pipe = pipe.clone();
    off_pipe.settings.tool_enforcement = ToolEnforcement::Off;
    validate_tool_enforcement_against(&off_pipe, Some(std::path::Path::new("/does/not/exist")))
        .expect("off must never consult the registry at all");
}

fn bare_test_stage(name: &str) -> Stage {
    Stage {
        name: name.to_string(),
        description: String::new(),
        policy: Vec::new(),
        skills_by: None,
        r#type: None,
        uses: None,
        composite_entry: None,
        composite_when: vec![],
        agent: None,
        model: None,
        effort: None,
        provider: None,
        skills: Vec::new(),
        needs: Vec::new(),
        inputs: Default::default(),
        outputs: Vec::new(),
        with: Default::default(),
        when: None,
        gate: None,
        scope: None,
        isolation: None,
        tools: None,
        fan_out: None,
        merge_findings: None,
        on_harness_gap: None,
        plan_tasks: None,
        max_iterations: None,
        on_exhausted: None,
        knowledge: None,
        body: Vec::new(),
    }
}

// C5: `opts.effort`, per-role default gated behind `settings.emit_effort`
// (false by default so pre-existing benchmarks keep their emitted
// semantics). An explicit stage-level `effort:` always wins, in either
// configuration.
#[test]
fn default_effort_per_role() {
    // emit_effort: false (default) — no `effort:` anywhere in the script.
    let (_ws, st) = init_run_in_workspace(
        "effort-off",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("effort:"),
        "emit_effort defaults to false: no `effort:` must be emitted:\n{js}"
    );

    // emit_effort: true — planner gets `high`, everyone else `medium`.
    let on_yaml = DEVISH.replacen(
        "settings: { max_correction_iterations: 2 }",
        "settings: { max_correction_iterations: 2, emit_effort: true }",
        1,
    );
    let (_ws, st_on) = init_run_in_workspace(
        "effort-on",
        &on_yaml,
        serde_json::json!({"track": "standard"}),
    );
    let js_on = emit(&st_on).unwrap();
    assert!(
        js_on.contains("agentType: \"planner\", model: \"opus\", effort: \"high\""),
        "planner defaults to high effort when emit_effort is true:\n{js_on}"
    );
    assert!(
        js_on.contains("agentType: \"executor\", model: \"sonnet\", effort: \"medium\""),
        "executor defaults to medium effort when emit_effort is true:\n{js_on}"
    );

    // An explicit stage-level `effort:` always wins, in both configurations.
    let pinned_yaml_base = DEVISH.replacen(
            "  - name: code\n    agent: executor\n    needs: [plan]\n    scope: { mode: production }\n    outputs: [changed-files.json]\n",
            "  - name: code\n    agent: executor\n    effort: max\n    needs: [plan]\n    scope: { mode: production }\n    outputs: [changed-files.json]\n",
            1,
        );
    assert_ne!(
        pinned_yaml_base, DEVISH,
        "sanity: the `code` stage block must match DEVISH verbatim for the replace to apply"
    );
    for (tag, yaml) in [
        ("effort-off", pinned_yaml_base.clone()),
        (
            "effort-on",
            pinned_yaml_base.replacen(
                "settings: { max_correction_iterations: 2 }",
                "settings: { max_correction_iterations: 2, emit_effort: true }",
                1,
            ),
        ),
    ] {
        let (_ws, st) = init_run_in_workspace(
            &format!("effort-pin-{tag}"),
            &yaml,
            serde_json::json!({"track": "standard"}),
        );
        let js = emit(&st).unwrap();
        assert!(
            js.contains("agentType: \"executor\", model: \"sonnet\", effort: \"max\""),
            "explicit stage effort must win regardless of emit_effort ({tag}):\n{js}"
        );
    }
}

// run-state token fix: each sequential stage snapshots budget.spent() around
// its agent() call and folds the delta into metrics.tokens_used (reconcile
// reads it). The `results[name] = await ...` assignment form is preserved so
// the dispatch/loop tests still match.
#[test]
fn seq_stage_captures_tokens() {
    let (_ws, st) = init_run_in_workspace(
        "tokens-seq",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("function spent("),
        "must emit the spent() helper"
    );
    assert!(
        js.contains("function withTokens("),
        "must emit the withTokens() helper"
    );
    // `code` is a sequential stage.
    assert!(
        js.contains("const __b_code = spent();"),
        "seq stage must snapshot tokens before dispatch:\n{js}"
    );
    assert!(
        js.contains("withTokens(results[\"code\"], spent() - __b_code);"),
        "seq stage must fold the token delta:\n{js}"
    );
}

// run-state token fix: a parallel group measures the exact block token total
// and splits it across its stages (sum exact, per-stage estimated).
#[test]
fn parallel_group_splits_tokens() {
    // track=standard → study and plan are independent (both need discover) →
    // grouped into one parallel() block.
    let (_ws, st) = init_run_in_workspace(
        "tokens-par",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("const __pb = spent();"),
        "parallel block must snapshot tokens before:\n{js}"
    );
    assert!(
        js.contains("const __pt = Math.max(0, spent() - __pb);"),
        "parallel block must measure the total delta:\n{js}"
    );
    assert!(
        js.contains("withTokens(results[\"study\"], Math.round(__pt / __pn));"),
        "parallel stage must get an even split:\n{js}"
    );
}

// WI-P1-6: max_stage_retries=0 (default) emits a plain agent() call; >0
// emits a budget-guarded dispatch() retry wrapper.
#[test]
fn stage_retries_emit_dispatch_only_when_enabled() {
    let (_ws, st) =
        init_run_in_workspace("noretry", DEVISH, serde_json::json!({"track": "standard"}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("results[\"code\"] = await safeAgent("),
        "default must emit a plain agent() call"
    );
    assert!(
        !js.contains("dispatch("),
        "default (retries=0) must not emit dispatch()"
    );

    const RETRY: &str = include_str!(
        "../fixtures/workflow_emit/stage_retries_emit_dispatch_only_when_enabled_retry.yaml"
    );
    let (_ws, st2) = init_run_in_workspace("retry", RETRY, serde_json::json!({}));
    let js2 = emit(&st2).unwrap();
    assert!(
        js2.contains("async function dispatch("),
        "retry build must define dispatch()"
    );
    assert!(
        js2.contains("results[\"code\"] = await dispatch("),
        "stage must dispatch when retries>0"
    );
    assert!(
        js2.contains(", 2)"),
        "dispatch must carry the retry bound 2"
    );
}

// WI-P0-3: reviewer stages get the stricter schema requiring signals;
// non-reviewer stages keep the base schema.
#[test]
fn reviewer_stage_requires_signals_schema() {
    let (_ws, st) = init_run_in_workspace(
        "revschema",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(
        js.contains("const REPLY_REVIEWER ="),
        "must emit the reviewer schema const"
    );
    assert!(
        js.contains("\"required\":[\"status\",\"summary\",\"signals\"]"),
        "reviewer schema must require signals"
    );
    // The review stage (agent: reviewer) selects REPLY_REVIEWER.
    assert!(
        js.contains("schema: REPLY_REVIEWER, agentType: \"reviewer\""),
        "reviewer stage must use REPLY_REVIEWER"
    );
    // A non-reviewer stage (code, agent: executor) keeps REPLY.
    assert!(
        js.contains("schema: REPLY, agentType: \"executor\""),
        "executor stage must use the base REPLY"
    );
}

const SKILLED_PIPE: &str = include_str!("../fixtures/skills/skilled.yaml");
const BARE_PIPE: &str = include_str!("../fixtures/skills/bare.yaml");

/// A stage declaring skills gets a schema requiring `skills_loaded` and
/// carries its list for `safeAgent`; a stage declaring none keeps the plain
/// schema, and a pipeline declaring none does not define the `*_SKILLS` consts.
#[test]
fn skilled_stage_requires_skills_loaded_in_its_schema() {
    let (_ws, st) = init_run_in_workspace("skills-schema", SKILLED_PIPE, serde_json::json!({}));
    let js = emit(&st).unwrap();

    let schema = const_text(&js, "REPLY_SKILLS");
    assert!(
        schema.contains("\"required\":[\"status\",\"summary\",\"skills_loaded\"]"),
        "REPLY_SKILLS must require skills_loaded: {schema}"
    );
    assert!(
        const_text(&js, "REPLY_REVIEWER_SKILLS")
            .contains("\"required\":[\"status\",\"summary\",\"signals\",\"skills_loaded\"]"),
        "the reviewer variant keeps signals and adds skills_loaded"
    );
    assert!(
        js.contains("schema: REPLY_SKILLS, agentType: \"executor\""),
        "{js}"
    );
    assert!(
        js.contains("skills: [\"a\", \"b\"], skillsRetry: false }"),
        "an executor is never re-dispatched over its attestation: {js}"
    );
    assert!(
        js.contains("schema: REPLY_REVIEWER_SKILLS, agentType: \"reviewer\""),
        "{js}"
    );
    assert!(js.contains("skills: [\"c\"] }"), "{js}");
    assert!(
        js.contains("phase: \"note\", schema: REPLY, agentType: \"executor\""),
        "a stage without skills keeps REPLY: {js}"
    );

    let (_ws2, st2) = init_run_in_workspace("skills-schema-bare", BARE_PIPE, serde_json::json!({}));
    let js2 = emit(&st2).unwrap();
    assert!(!js2.contains("REPLY_SKILLS"), "{js2}");
    assert!(!js2.contains("skills: ["), "{js2}");
}

/// Runs the emitted `safeAgent` under node against a scripted `agent()`,
/// returning `JSON.stringify` of whatever `case` logs.
fn eval_safe_agent(replies: &str, case: &str) -> String {
    let start = HELPERS.find("function envelopeShapeError(").unwrap();
    let script = format!(
            "const calls = [];\nconst logs = [];\nfunction log(m) {{ logs.push(m); }}\nconst __replies = {replies};\nasync function agent(p, o) {{ calls.push({{ p, o }}); return __replies[calls.length - 1]; }}\n{}\n{RETRY_HELPERS}\n(async () => {{ {case} }})();\n",
            &HELPERS[start..]
        );
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("safe_agent.js");
    std::fs::write(&f, script).unwrap();
    let out = std::process::Command::new("node")
        .arg(&f)
        .output()
        .expect("node must be installed to run this test");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const OK_A: &str = r#"{ name: "a", applied_rule: "rule of a" }"#;
const OK_B: &str = r#"{ name: "b", applied_rule: "rule of b" }"#;

#[test]
fn it_safe_agent_re_dispatches_once_then_fails_naming_the_missing_skill() {
    let out = eval_safe_agent(
        &format!(
            r#"[ {{ status: "completed", summary: "s1", skills_loaded: [{OK_A}] }},
                    {{ status: "completed", summary: "s2", skills_loaded: [{OK_A}, {{ name: "b", applied_rule: "  " }}] }} ]"#
        ),
        r###"const r = await safeAgent("P", { label: "code", schema: {}, skills: ["a", "b"] });
               console.log(JSON.stringify({ status: r.status, summary: r.summary, miss: r.signals.skills_not_loaded,
                 calls: calls.length, reminded: calls[1].p.includes("## Skills not loaded") && calls[1].p.includes("b"),
                 stripped: calls.every(c => !("skills" in c.o)) }));"###,
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "failed", "{out}");
    assert_eq!(v["miss"], serde_json::json!(["b"]), "{out}");
    assert_eq!(v["calls"], 2, "exactly one re-dispatch: {out}");
    assert_eq!(v["reminded"], true, "{out}");
    assert_eq!(
        v["stripped"], true,
        "agent() must never see opts.skills: {out}"
    );
    assert!(
        v["summary"]
            .as_str()
            .unwrap()
            .contains("skills not loaded after one reminder: b"),
        "{out}"
    );
}

/// A completed executor may already have pushed, published or created
/// tickets: it is never re-run over its attestation, neither by `safeAgent`
/// nor by `dispatch()`'s own retries.
#[test]
fn it_an_executor_missing_a_skill_is_downgraded_without_being_re_dispatched() {
    let out = eval_safe_agent(
        &format!(r#"[ {{ status: "completed", summary: "pushed", skills_loaded: [{OK_A}] }} ]"#),
        r#"const r = await safeAgent("P", { label: "push", skills: ["a", "b"], skillsRetry: false });
               console.log(JSON.stringify([r.status, r.signals.skills_not_loaded, calls.length, "skillsRetry" in calls[0].o]));"#,
    );
    assert_eq!(out, r#"["needs_review",["b"],1,false]"#);

    let out = eval_safe_agent(
        &format!(r#"[ {{ status: "completed", summary: "pushed", skills_loaded: [{OK_A}] }} ]"#),
        r#"const r = await dispatch("P", { label: "push", skills: ["a", "b"], skillsRetry: false }, 3);
               console.log(JSON.stringify([r.status, calls.length]));"#,
    );
    assert_eq!(
        out, r#"["needs_review",1]"#,
        "dispatch() must not retry a skills downgrade"
    );
}

#[test]
fn it_safe_agent_accepts_a_reply_that_covers_every_skill() {
    let out = eval_safe_agent(
        &format!(
            r#"[ {{ status: "completed", summary: "s1", skills_loaded: [{OK_B}] }},
                    {{ status: "completed", summary: "s2", skills_loaded: [{OK_B}, {OK_A}] }} ]"#
        ),
        r#"const r = await safeAgent("P", { label: "code", skills: ["a", "b"] });
               console.log(JSON.stringify([r.status, r.summary, calls.length]));"#,
    );
    assert_eq!(out, r#"["completed","s2",2]"#, "fixed by the reminder");

    let out = eval_safe_agent(
        &format!(r#"[ {{ status: "completed", summary: "s1", skills_loaded: [{OK_A}] }} ]"#),
        r#"const r = await safeAgent("P", { label: "code", skills: ["a"] });
               console.log(JSON.stringify([r.status, calls.length]));"#,
    );
    assert_eq!(out, r#"["completed",1]"#, "complete on the first try");
}

#[test]
fn it_safe_agent_leaves_failed_replies_and_skill_free_dispatches_alone() {
    let out = eval_safe_agent(
        r#"[ { status: "failed", summary: "build broke" } ]"#,
        r#"const r = await safeAgent("P", { label: "code", skills: ["a"] });
               console.log(JSON.stringify([r.status, r.summary, calls.length]));"#,
    );
    assert_eq!(out, r#"["failed","build broke",1]"#);

    let out = eval_safe_agent(
        r#"[ { status: "completed", summary: "no skills" } ]"#,
        r#"const o = { label: "note" };
               const r = await safeAgent("P", o);
               console.log(JSON.stringify([r.status, calls.length, calls[0].o === o]));"#,
    );
    assert_eq!(out, r#"["completed",1,true]"#);
}

// C6 (Lot 3, Capitalisation): extends `reviewer_stage_requires_signals_schema`
// — the reviewer schema's `signals` object must also require
// `unevaluated_checks` (a control that could not run), alongside
// `has_blocking_findings`, and declare `has_harness_gap`. The outer
// `required: ["status","summary","signals"]` (asserted above) is untouched.
#[test]
fn reviewer_schema_requires_unevaluated_checks() {
    let (_ws, st) = init_run_in_workspace(
        "revchecks",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = emit(&st).unwrap();
    assert!(js.contains("const REPLY_REVIEWER ="));
    assert!(
        js.contains("\"required\":[\"has_blocking_findings\",\"unevaluated_checks\"]"),
        "reviewer signals schema must require unevaluated_checks alongside has_blocking_findings"
    );
    assert!(
        js.contains("\"unevaluated_checks\":"),
        "signals schema must declare unevaluated_checks"
    );
    assert!(
        js.contains("\"minimum\":0"),
        "unevaluated_checks must be a non-negative integer"
    );
    assert!(
        js.contains("\"has_harness_gap\":{\"type\":\"boolean\"}"),
        "signals schema must declare has_harness_gap"
    );
    // The outer required list is untouched by this change.
    assert!(js.contains("\"required\":[\"status\",\"summary\",\"signals\"]"));
}

// C6 (Lot 3, Capitalisation): a stage declaring `merge_findings:` gets (a)
// the mechanical-verdict contract injected into its OWN prompt, naming
// each producer explicitly via `--label` (every axis's file is named
// `findings.json`, so a filename-derived label would collide), and (b) the
// post-stage guard that pauses the run if its own `outputs.findings` never
// materializes. `on_harness_gap: pause` adds the companion `sig()` guard.
#[test]
fn merge_findings_injects_contract_and_guard() {
    const MERGEFIND: &str = include_str!(
        "../fixtures/workflow_emit/merge_findings_injects_contract_and_guard_mergefind.yaml"
    );
    let (_ws, st) = init_run_in_workspace("mergefind", MERGEFIND, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("aidp findings merge"),
        "must inject the mechanical-merge instruction into the synthesis stage's prompt"
    );
    assert!(
            js.contains("--label review_correctness=") && js.contains("--label review_security="),
            "must name each producer axis explicitly via --label (every axis shares the filename findings.json): {js}"
        );
    assert!(
            js.contains("if (!results[\"review\"].outputs || !results[\"review\"].outputs.findings) return pauseInfo(\"review\", results[\"review\"]);"),
            "must emit the post-stage guard for a missing merged findings output"
        );
    assert!(
            js.contains("if (sigNum(\"review\", \"unevaluated_checks\") > 0) return pauseInfo(\"review\", results[\"review\"]);"),
            "on_harness_gap: pause must emit the sigNum()-based guard reading the required field"
        );
    // review_contracts(): named controls for the dimension axes + synthesis.
    assert!(js.contains("C-diff-read"));
    assert!(js.contains("C-merge-tool-invoked"));
    // LOC-42: the synthesis stage must be told to read the merge
    // command's own `degraded_axes` field rather than guess a degraded
    // axis from free text, and told that only a lied-about tally (not a
    // missing/invalid axis file) still exits the command non-zero.
    assert!(
        js.contains("degraded_axes"),
        "the synthesis stage's contract must name the degraded_axes envelope field: {js}"
    );
}

// Reproduces the review-perf-findings-missing incident: an axis (e.g.
// `review_perf`) replied `completed` with an `outputs.findings` path that
// was never written, and nothing caught it until the synthesis stage's
// `aidp findings merge` failed two stages later. The Workflow sandbox has
// no filesystem access, so the only place a fabricated path can be caught
// live is inside the axis's own turn — this asserts the axis prompt
// actually carries that instruction, and that it survives even a
// 0-findings review (the exact case the incident's axis rationalized
// skipping the write for).
#[test]
fn axis_contract_requires_a_readback_before_completed() {
    const MERGEFIND: &str = include_str!("../fixtures/workflow_emit/axis_contract_requires_a_readback_before_completed_mergefind.yaml");
    let (_ws, st) = init_run_in_workspace("mergefind", MERGEFIND, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("use the Read tool to re-open the `findings.json` you just wrote"),
        "an axis stage (correctness/security/performance/maintainability) must be told to \
             verify its own output before claiming completed: {js}"
    );
    assert!(
        js.contains("even with 0 findings"),
        "the readback instruction must explicitly survive the 0-findings case, since that is \
             the exact case the incident's axis rationalized skipping the write for: {js}"
    );
}

// F-6: reproduces the exact bypass the finding named — a schema-valid
// reviewer reply can carry `unevaluated_checks: 2` (required) while
// omitting `has_harness_gap` (optional) entirely. No Node harness exists
// to run `sigNum()` against real reply data (see the plan's Risques
// section), so this proves the emitted guard reads the required field
// directly, with no remaining textual dependency on the optional one.
#[test]
fn harness_gap_guard_reads_the_required_unevaluated_checks_field_directly() {
    const HGFIND: &str = include_str!("../fixtures/workflow_emit/harness_gap_guard_reads_the_required_unevaluated_checks_field_directly_hgfind.yaml");
    let (_ws, st) = init_run_in_workspace("hgfind", HGFIND, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
            js.contains("if (sigNum(\"review\", \"unevaluated_checks\") > 0) return pauseInfo(\"review\", results[\"review\"]);"),
            "the guard must read the required unevaluated_checks field directly:\n{js}"
        );
    assert!(
        !js.contains("sig(\"review\", \"has_harness_gap\")"),
        "the guard itself must no longer depend on the optional has_harness_gap field:\n{js}"
    );
}

// LOC-16 incident: `on_harness_gap: pause` on a stage that ALSO feeds a
// `correction_loop` used to pause the run unconditionally on
// `unevaluated_checks > 0`, one step before `correction_loop`'s own
// `when: has_blocking_findings` could ever be evaluated — stranding a
// real blocking finding outside the very machinery built to fix it, and
// needlessly halting a run with ZERO blocking findings just because one
// review axis could not re-run its own tests (a routine sandbox
// limitation, not something a human needs to act on). The immediate
// pause must disappear for a loop-linked stage; the loop's own
// exhaustion guard (`unconverged_correction_loop_pauses_after_the_loop`)
// remains the checkpoint for a still-unresolved blocking finding.
#[test]
fn harness_gap_guard_is_skipped_for_a_stage_feeding_a_correction_loop() {
    const LOOPED_HGFIND: &str = include_str!("../fixtures/workflow_emit/harness_gap_guard_is_skipped_for_a_stage_feeding_a_correction_loop_looped_hgfind.yaml");
    let (_ws, st) = init_run_in_workspace("loopedhgfind", LOOPED_HGFIND, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
            !js.contains(
                "if (sigNum(\"review\", \"unevaluated_checks\") > 0) return pauseInfo(\"review\", results[\"review\"]);"
            ),
            "a stage feeding a correction_loop must not pause on a harness gap alone — \
             the loop's own when:/exhaustion guard is the checkpoint:\n{js}"
        );
    // The loop's own machinery must still be present and untouched.
    assert!(
        js.contains("sig(\"review\", \"has_blocking_findings\")"),
        "the loop must still gate on has_blocking_findings:\n{js}"
    );
}

// The counterpart: a template with NO correction loop to fall back on
// (`audit`, `review`) must keep the unconditional harness-gap pause —
// there, it IS the human checkpoint (see `soft_pause_stages`'s doc
// comment on why `loop_linked_stages` is deliberately narrower than the
// full `soft` set `pause_check` uses for `needs_review`).
#[test]
fn harness_gap_guard_still_pauses_a_reviewer_stage_with_no_loop() {
    const HGFIND_NO_LOOP: &str = include_str!("../fixtures/workflow_emit/harness_gap_guard_still_pauses_a_reviewer_stage_with_no_loop_hgfind_no_loop.yaml");
    let (_ws, st) = init_run_in_workspace("hgfindnoloop", HGFIND_NO_LOOP, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
            js.contains(
                "if (sigNum(\"review\", \"unevaluated_checks\") > 0) return pauseInfo(\"review\", results[\"review\"]);"
            ),
            "a reviewer stage with no correction loop must keep the unconditional harness-gap pause:\n{js}"
        );
}

/// The names of every stage the compiled plan marks loop-linked, loop bodies included.
fn loop_linked_names(pipe: &Pipeline, state: &RunState) -> BTreeSet<String> {
    let plan =
        crate::exec_plan::compile_exec_plan(pipe, state, &DiskPlanSource).expect("plan compiles");
    let mut names = BTreeSet::new();
    let mut pending: Vec<&crate::exec_plan::Step> = plan.steps.iter().collect();
    while let Some(step) = pending.pop() {
        match step {
            crate::exec_plan::Step::Stage(spec) if spec.loop_linked => {
                names.insert(spec.name.clone());
            }
            crate::exec_plan::Step::Parallel { stages } => {
                names.extend(
                    stages
                        .iter()
                        .filter(|s| s.loop_linked)
                        .map(|s| s.name.clone()),
                );
            }
            crate::exec_plan::Step::Loop { body, .. } => pending.extend(body.iter()),
            _ => {}
        }
    }
    names
}

/// A loop whose body does not re-produce the output its `when:` reads still exempts the source stage.
#[test]
fn loop_linked_stages_inserts_source_even_without_producer() {
    const LOOP_NO_PRODUCER: &str = include_str!("../fixtures/workflow_emit/loop_linked_stages_inserts_source_even_without_producer_loop_no_producer.yaml");
    let (_ws, st) =
        init_run_in_workspace("loopnoproducer", LOOP_NO_PRODUCER, serde_json::json!({}));
    let pipe = pipeline::load(&st.pipeline_path).unwrap();
    let linked = loop_linked_names(&pipe, &st);
    assert!(
        linked.contains("review"),
        "the plan must mark the source stage loop-linked even without a producer: {:?}",
        linked
    );
    // refix doesn't produce findings, so there should be no body producer in the set
    assert!(
        !linked.contains("refix"),
        "refix is not a producer (doesn't output findings), so it must not be in the set: {:?}",
        linked
    );
}

// LOC-43: harness_gap_guard() must skip the pause for a stage that feeds
// a correction_loop whose body does not re-produce the `when:` output.
#[test]
fn harness_gap_guard_is_skipped_for_a_loop_whose_body_never_reproduces_the_output() {
    const LOOP_NO_PRODUCER_PAUSE: &str = include_str!("../fixtures/workflow_emit/harness_gap_guard_is_skipped_for_a_loop_whose_body_never_reproduces_the_output_loop_no_producer_pause.yaml");
    let (_ws, st) = init_run_in_workspace(
        "loopnoproducerpause",
        LOOP_NO_PRODUCER_PAUSE,
        serde_json::json!({}),
    );
    let js = emit(&st).unwrap();
    assert!(
            !js.contains(
                "if (sigNum(\"review\", \"unevaluated_checks\") > 0) return pauseInfo(\"review\", results[\"review\"]);"
            ),
            "harness_gap_guard must skip the pause for a stage feeding a loop, \
             even when the loop body never re-produces the output:\n{js}"
        );
    // The loop's condition must still be present
    assert!(
        js.contains("sig(\"review\", \"has_blocking_findings\")"),
        "the loop must still gate on has_blocking_findings:\n{js}"
    );
}

// LOC-43: a loop whose body never re-produces the `when:` output must not
// emit the exhaustion guard or results redirect, since the condition can
// never change (it only depends on the source stage's outputs, which are
// never refreshed in the loop). Without a producer, emitting a guard would
// recreate the non-resumable pause.
#[test]
fn loop_without_producer_emits_no_exhaustion_guard() {
    const LOOP_NO_PRODUCER_EMIT: &str = include_str!("../fixtures/workflow_emit/loop_without_producer_emits_no_exhaustion_guard_loop_no_producer_emit.yaml");
    let (_ws, st) = init_run_in_workspace(
        "loopnoproduceremit",
        LOOP_NO_PRODUCER_EMIT,
        serde_json::json!({}),
    );
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("did not converge"),
        "loop without a producer must not emit the exhaustion guard:\n{js}"
    );
    assert!(
        !js.contains("results[\"review\"] = results["),
        "loop without a producer must not emit a results redirect:\n{js}"
    );
}

/// A loop whose body re-produces the output links both the source stage and the producer.
#[test]
fn loop_linked_stages_is_unchanged_for_a_loop_with_a_producer() {
    const LOOP_WITH_PRODUCER: &str = include_str!("../fixtures/workflow_emit/loop_linked_stages_is_unchanged_for_a_loop_with_a_producer_loop_with_producer.yaml");
    let (_ws, st) = init_run_in_workspace(
        "loopwithproducer",
        LOOP_WITH_PRODUCER,
        serde_json::json!({}),
    );
    let pipe = pipeline::load(&st.pipeline_path).unwrap();
    let linked = loop_linked_names(&pipe, &st);
    assert!(
        linked.contains("review"),
        "the plan must mark the source stage loop-linked: {:?}",
        linked
    );
    assert!(
        linked.contains("rereview"),
        "the plan must mark the producing stage loop-linked: {:?}",
        linked
    );
}

// F-11: `Stage.tools` is honestly documented as not yet wired into the
// emitter (see `pipeline::ToolsSpec`'s doc comment) — pin that contract so
// a future PARTIAL wiring is forced to touch this test explicitly rather
// than silently drifting. `run_id` is randomized per `run::init` call
// (see `run::new_run_id`), so it is normalized out of both scripts before
// comparing — everything else must be byte-identical.
#[test]
fn stage_tools_field_does_not_change_emitted_js() {
    const WITHOUT: &str = include_str!(
        "../fixtures/workflow_emit/stage_tools_field_does_not_change_emitted_js_without.yaml"
    );
    const WITH: &str = include_str!(
        "../fixtures/workflow_emit/stage_tools_field_does_not_change_emitted_js_with.yaml"
    );
    let (_ws, st_without) = init_run_in_workspace("tools-flag", WITHOUT, serde_json::json!({}));
    let js_without = {
        let js = emit(&st_without).unwrap();
        let js = js.replace(&st_without.run_id, "RUNID");
        // Replace the aidp-tools-flag-XXXXX pattern (with any random suffix) with a normalized form
        let re = regex::Regex::new(r"aidp-tools-flag-[A-Za-z0-9]+").unwrap();
        re.replace_all(&js, "aidp-tools-flag-NORMALIZED")
            .to_string()
    };
    let (_ws, st_with) = init_run_in_workspace("tools-flag", WITH, serde_json::json!({}));
    let js_with = {
        let js = emit(&st_with).unwrap();
        let js = js.replace(&st_with.run_id, "RUNID");
        // Replace the aidp-tools-flag-XXXXX pattern (with any random suffix) with a normalized form
        let re = regex::Regex::new(r"aidp-tools-flag-[A-Za-z0-9]+").unwrap();
        re.replace_all(&js, "aidp-tools-flag-NORMALIZED")
            .to_string()
    };
    assert_eq!(
        js_without, js_with,
        "F-11: declaring tools: on a stage must not change the emitted script — it stays inert"
    );
}

// WI-P3-1: a loop whose `when:` is not the canonical signal shape must FAIL
// emit loudly instead of silently compiling to `&& false` (never iterating).
#[test]
fn loop_with_malformed_when_fails_emit() {
    const BAD: &str =
        include_str!("../fixtures/workflow_emit/loop_with_malformed_when_fails_emit_bad.yaml");
    let (_ws, st) = init_run_in_workspace("badloop", BAD, serde_json::json!({}));
    let err = emit(&st).unwrap_err().to_string();
    assert!(
        err.contains("bad_loop"),
        "error must name the offending loop: {err}"
    );
    assert!(
        err.contains("when:"),
        "error must mention the when: shape: {err}"
    );
}

// Intra-stage fan-out: static `items:` emit a shard parallel() + a merge
// agent under the stage's canonical name.
#[test]
fn fanout_static_items_emit_shards_and_merge() {
    const FO: &str =
        include_str!("../fixtures/workflow_emit/fanout_static_items_emit_shards_and_merge_fo.yaml");
    let (_ws, st) = init_run_in_workspace("fostatic", FO, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("const __sh_tests = shards([\"unit tests\", \"functional tests\"], 4);"),
        "static shards must be baked in:\n{js}"
    );
    assert!(
            js.contains(
                "const __fo_tests = await parallel(__sh_tests.map((__s, __i) => () => __pl_tests.run(() => safeAgent("
            ),
            "shards must fan out via parallel(), each dispatch admission-gated through pool():\n{js}"
        );
    assert!(
        js.contains("shardNote(__s, __i, __sh_tests.length)"),
        "each shard prompt gets a shard note"
    );
    assert!(
        js.contains("mergeNote(\"Run the full test suite once.\")"),
        "merge hint must be forwarded"
    );
    assert!(
        js.contains("fanin(__fo_tests, __sh_tests)"),
        "merge prompt must embed shard envelopes"
    );
    assert!(
        js.contains("results[\"tests\"] = await safeAgent("),
        "merge result lands under the canonical stage name"
    );
    assert!(
        js.contains("if (__sh_tests.length === 1)"),
        "single-shard runs must short-circuit the merge agent:\n{js}"
    );
    assert!(
        js.contains("function shards("),
        "fan-out helpers must be emitted"
    );
    // The non-fan-out stage stays a plain call.
    assert!(js.contains("results[\"code\"] = await safeAgent("));
}

// C8 (Lot 3, Capitalisation): `pool(...)` is emitted exactly when a
// consumer exists (fan_out here), and every shard dispatch goes through
// it — the wrapping this chantier's spec requires around the existing
// `parallel(__sh_id.map(...))` shard dispatch.
#[test]
fn pool_is_emitted_when_a_consumer_exists_and_wraps_every_shard() {
    const FO: &str = include_str!("../fixtures/workflow_emit/pool_is_emitted_when_a_consumer_exists_and_wraps_every_shard_fo.yaml");
    let (_ws, st) = init_run_in_workspace("poolfanout", FO, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("function pool("),
        "pool() must be defined when a fan_out consumer exists:\n{js}"
    );
    assert!(
        js.contains("__pl_tests.run("),
        "every shard dispatch must be admission-gated through pool.run():\n{js}"
    );
}

// C8 (Lot 3, Capitalisation): the mirror image — no fan_out, no
// plan_tasks, and (crucially) no adjacent independent-stage group either
// (the `needs` chain below makes `a`→`b` strictly sequential), so nothing
// ever calls `pool(...)`. POOL_HELPERS must stay absent: this is the test
// that proves a `pool(` reference can never outrun its definition
// (no ReferenceError), by proving the converse never happens either.
#[test]
fn pool_is_not_emitted_when_nothing_consumes_it() {
    const SEQ: &str = include_str!(
        "../fixtures/workflow_emit/pool_is_not_emitted_when_nothing_consumes_it_seq.yaml"
    );
    let (_ws, st) = init_run_in_workspace("nopool", SEQ, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("function pool("),
        "pool() must not be defined without a consumer:\n{js}"
    );
    assert!(
        !js.contains("pool("),
        "pool(...) must never be called without a consumer:\n{js}"
    );
}

// C8 (Lot 3, Capitalisation): the admission-budget block this chantier
// adds (`POOL_HELPERS`, with its own private `__poolBud`/`__poolSem`)
// duplicates rather than promotes `bud()`/`sem()` — Lot 2 §4 owns that
// promotion, and `bud()`/`sem()` must not move a single physical line
// before it lands (two chantiers relocating the same line produce a
// non-mechanical merge conflict). Checked against the Rust source
// constants themselves, not the compiled JS.
#[test]
fn it_bud_and_sem_are_not_moved() {
    assert!(
        RETRY_HELPERS.starts_with("function bud() {"),
        "bud() must still be the line that opens RETRY_HELPERS's raw string:\n{RETRY_HELPERS}"
    );
    assert!(
        PLAN_HELPERS.contains("function sem(n) {"),
        "sem() must still be defined inside PLAN_HELPERS's raw string:\n{PLAN_HELPERS}"
    );
}

// C8 (Lot 3, Capitalisation): `settings.on_item_failure: continue` is
// compile-time (settings are known at compile time, not read from
// `args`), so it must change what gets EMITTED, not just what runs — the
// per-shard `return pauseInfo` line must be textually absent, replaced by
// a log-only line, while the stage's own final envelope (the merge
// dispatch's outcome) keeps the ordinary hard checkpoint.
#[test]
fn it_on_item_failure_continue_does_not_return_pauseinfo() {
    const FO: &str = include_str!(
        "../fixtures/workflow_emit/on_item_failure_continue_does_not_return_pauseinfo_fo.yaml"
    );
    let (_ws, st) = init_run_in_workspace("poolcontinue", FO, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        !js.contains("for (const __r of __fo_tests) { if (isPause(__r, false)) return pauseInfo("),
        "on_item_failure: continue must not early-return per shard:\n{js}"
    );
    assert!(
        js.contains("shard not completed (on_item_failure: continue"),
        "a not-completed shard must still be logged, not silently dropped:\n{js}"
    );
    // The merge dispatch's own outcome keeps the hard checkpoint — only
    // the per-item loop is relaxed.
    assert!(
            js.contains(
                "if (isPause(results[\"tests\"], false)) return pauseInfo(\"tests\", results[\"tests\"]);"
            ),
            "the stage's own merge outcome must still halt on failure:\n{js}"
        );
}

// C8 (Lot 3, Capitalisation): the five templates declaring neither
// `plan_tasks:` nor `settings.max_stage_retries` (the limit case this
// chantier must not break) — three fan out, two have no fan_out at all
// but DO have an adjacent independent-stage group (the one dispatch
// `pool` did not previously guard: see `emit_parallel_group`). `fold`
// defaults to `true` everywhere (none declares it), preserving pre-C8
// shard-folding behaviour; the only textual delta versus pre-C8 is the
// `pool(...)`/`.run(...)` wrapping around whichever consumer(s) each one
// actually has.
/// Pre-cross every composite boundary the run's effective pipeline
/// declares, so a test built around real templates can assert on a
/// full, uncut script regardless of where a `type: pipeline` stage sits
/// — the composite-boundary stop is orthogonal to what these tests
/// actually verify (fan-out/loop/plan_tasks emission further downstream).
fn cross_all_composite_boundaries(rst: &mut RunState) {
    let pipe = pipeline::load(&rst.pipeline_path).expect("reload effective pipeline for crossing");
    let mut names: Vec<String> = pipeline::flat_stages(&pipe)
        .iter()
        .filter_map(|st| st.composite_entry.clone())
        .collect();
    names.sort();
    names.dedup();
    rst.crossed_boundaries = names;
}

#[test]
fn it_fan_out_fold_defaults_to_true_and_preserves_shard_count() {
    let skill_root = aidp_testkit::fixtures::skill_root();
    let tpl_dir = skill_root.join("templates");
    let fixture_path = &aidp_testkit::fixtures::template_inputs();
    let fixture: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display())),
    )
    .expect("template-inputs.json must be valid JSON");

    for id in [
        "dev-tests-csharp",
        "cheat-stack-test",
        "gdpr-anonymisation-test",
        "knowledge-init",
        "run-tests-dev",
    ] {
        // Lot T (R-ARC-1) archived `dev-tests-csharp` under
        // `templates/legacy/` (superseded by dev-tests-csharp-split,
        // orphaned otherwise — see `pipeline::ARCHIVE_DIR`); its `id:`
        // and content are unchanged, so it stays loadable by this
        // explicit path even though `pipeline::discover` no longer lists
        // it.
        let path = if id == "dev-tests-csharp" {
            tpl_dir.join("legacy").join(format!("{id}.yaml"))
        } else {
            tpl_dir.join(format!("{id}.yaml"))
        };
        let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
        assert!(
            !pipe.stages.iter().any(|st| st.plan_tasks.is_some()),
            "{id} is supposed to be the plan_tasks-free limit case"
        );
        assert_eq!(
            pipe.settings.max_stage_retries, 0,
            "{id} is supposed to be the max_stage_retries-free limit case"
        );
        let fanout_stage_ids: Vec<String> = pipeline::flat_stages(&pipe)
            .iter()
            .filter(|st| st.fan_out.is_some())
            .map(|st| {
                assert!(
                    st.fan_out.as_ref().unwrap().fold,
                    "{id}/{}: fold must default to true",
                    st.name
                );
                ident(&st.name)
            })
            .collect();

        let inputs = fixture
            .get(id)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let ws_guard = tempfile::tempdir().unwrap();
        let ws = ws_guard.path().to_path_buf();
        std::fs::create_dir_all(&ws).unwrap();
        let mut rst =
            run::init(&path, &ws, inputs, None).unwrap_or_else(|e| panic!("run::init({id}): {e}"));
        cross_all_composite_boundaries(&mut rst);
        let js = emit(&rst).unwrap_or_else(|e| panic!("emit({id}): {e}"));

        for sid in &fanout_stage_ids {
            assert!(
                js.contains(&format!("__pl_{sid}.run(")),
                "{id}/{sid}: fan-out shard dispatch must be wrapped through pool():\n{js}"
            );
        }
        // Whatever consumer(s) this template has (fan_out and/or an
        // adjacent parallel group), pool() itself must be defined.
        assert!(
            js.contains("function pool("),
            "{id}: expected a pool() consumer (fan_out or an adjacent parallel group):\n{js}"
        );
    }
    // The fold=true branch's log message (and thus its folding logic) is
    // untouched by C8 — only the fold=false hard-truncate branch is new.
    assert!(FANOUT_HELPERS.contains("fan-out: folding "));
    assert!(FANOUT_HELPERS.contains("fan-out: dropping "));
}

// Dynamic fan-out: `from: stages.plan.signals.shards` reads the shard list
// at runtime and injects the fan-out contract into the producer's prompt.
#[test]
fn fanout_dynamic_from_reads_signal_and_injects_contract() {
    const FO: &str = include_str!(
        "../fixtures/workflow_emit/fanout_dynamic_from_reads_signal_and_injects_contract_fo.yaml"
    );
    let (_ws, st) = init_run_in_workspace("fodyn", FO, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("shards(sigArr(\"plan\", \"shards\"), 3)"),
        "dynamic shards read the upstream signal:\n{js}"
    );
    assert!(
        js.contains("Fan-out contract (consumer: `code`)"),
        "producer prompt must carry the contract"
    );
    assert!(
        js.contains("signals.shards"),
        "contract names the signal property"
    );
    assert!(
        js.contains("file-disjoint"),
        "contract requires disjoint work packages"
    );
}

// A fan-out stage is never merged into an adjacent parallel group, and its
// shards pause-check before the merge runs.
#[test]
fn fanout_breaks_grouping_and_checks_shard_pauses() {
    const FO: &str = include_str!(
        "../fixtures/workflow_emit/fanout_breaks_grouping_and_checks_shard_pauses_fo.yaml"
    );
    let (_ws, st) = init_run_in_workspace("fogroup", FO, serde_json::json!({}));
    let js = emit(&st).unwrap();
    // a, b, c are mutually independent and adjacent, but b's fan-out splits them.
    assert!(
        !js.contains("parallel: a, b"),
        "fan-out stage must not join a neighbour group:\n{js}"
    );
    assert!(
        js.contains(
            "for (const __r of __fo_b) { if (isPause(__r, false)) return pauseInfo(\"b\", __r); }"
        ),
        "shard pauses must halt before the merge"
    );
}

// Config errors fail emit loudly.
#[test]
fn fanout_validation_errors() {
    const BOTH: &str = include_str!("../fixtures/workflow_emit/fanout_validation_errors_both.yaml");
    let (_ws, st) = init_run_in_workspace("foboth", BOTH, serde_json::json!({}));
    let err = emit(&st).unwrap_err().to_string();
    assert!(
        err.contains("not both"),
        "items+from must be rejected: {err}"
    );

    const BADFROM: &str =
        include_str!("../fixtures/workflow_emit/fanout_validation_errors_badfrom.yaml");
    let (_ws, st2) = init_run_in_workspace("fobadfrom", BADFROM, serde_json::json!({}));
    let err2 = emit(&st2).unwrap_err().to_string();
    assert!(
        err2.contains("stages.<stage>.signals.<prop>"),
        "outputs-shaped from must be rejected: {err2}"
    );
}

// -----------------------------------------------------------------------
// Plan-driven stages (`plan_tasks:`)
// -----------------------------------------------------------------------

const PLANNED: &str = include_str!("../fixtures/workflow_emit/planned.yaml");

/// Two `plan_tasks:` stages splitting one plan by `kinds:` — the shape where
/// one stage claims several tasks and the other claims exactly one.
const SPLIT_KINDS: &str = include_str!("../fixtures/workflow_emit/split_kinds.yaml");

/// Write a `plan.json` into the `plan` stage dir of an initialized run.
fn write_plan(st: &RunState, body: &str) {
    let dir = st.run_dir.join("plan");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plan.json"), body).unwrap();
}

fn plan_task(id: &str, kind: &str, needs: &[&str], files: &[&str]) -> String {
    format!(
        r#"{{"id":"{id}","title":"do {id}","kind":"{kind}","prompt":"instruction for {id}","needs":{},"files":{}}}"#,
        serde_json::to_string(needs).unwrap(),
        serde_json::to_string(files).unwrap()
    )
}

// T02 (suite): the `awaiting_plan` early return carries `results` too. It can
// only be asserted on a fixture whose stage actually declares `plan_tasks:`,
// since `emit_plan_stage` is the sole emitter of that literal.
#[test]
fn awaiting_plan_carries_results() {
    let (_ws, st) = init_run_in_workspace("plan-results", PLANNED, serde_json::json!({}));
    let js = compile(&st).unwrap().script;
    assert!(
        js.lines()
            .any(|line| line.contains("status: \"awaiting_plan\"")
                && line.contains("results: results")),
        "awaiting_plan literal must carry results: results on the same line:
{js}"
    );
}

// Pass 1: the plan does not exist yet, so the script stops at the stage and
// NOTHING downstream is compiled — the tail is not knowable until the plan is.
// The planner's own prompt must already carry the plan.json contract.
#[test]
fn it_plan_tasks_pass1_awaits_the_plan_and_compiles_no_tail() {
    let (_ws, st) = init_run_in_workspace("plan-pass1", PLANNED, serde_json::json!({}));
    let e = compile(&st).unwrap();
    let js = &e.script;
    assert!(
        js.contains("results[\"plan\"] = await safeAgent("),
        "the planner still runs"
    );
    assert!(
        js.contains("return { status: \"awaiting_plan\", at: \"implement\", producer: \"plan\""),
        "must pause for the plan:\n{js}"
    );
    assert!(
        !js.contains("__tk_implement["),
        "no task may be dispatched on pass 1"
    );
    assert!(
        !js.contains("results[\"review\"] = await"),
        "the tail must not be compiled before the plan exists:\n{js}"
    );
    assert!(e.plan_schedules.is_empty());
    let a = e.awaiting_plan.expect("awaiting_plan must be reported");
    assert_eq!(a.stage, "implement");
    assert_eq!(a.producer, "plan");
    assert!(a.plan_path.ends_with("plan.json"), "got {}", a.plan_path);
    // The contract that makes the plan executable is injected into the
    // producer's prompt, not left to the planner persona doc.
    assert!(js.contains("Plan contract — write an EXECUTABLE plan (consumer: `implement`)"));
    assert!(js.contains("aidp-plan.v1"));
    assert!(js.contains("`files` is the write set"));
    assert!(
        js.contains("up to 3 run concurrently"),
        "the cap must reach the planner"
    );
}

// Pass 2: with the plan on disk, the stage expands into one dispatch per task
// (deps wired), a concurrency cap, a merge agent under the canonical stage
// name — and the rest of the pipeline is finally compiled.
#[test]
fn plan_tasks_pass2_expands_the_scheduled_graph() {
    let (_ws, st) = init_run_in_workspace("plan-pass2", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"schema":"aidp-plan.v1","summary":"s","shared_files":["src/Program.cs"],"tasks":[{},{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"]),
            plan_task("T3", "tests", &["T1"], &["tests/A.tests.cs"])
        ),
    );
    let e = compile(&st).unwrap();
    let js = &e.script;
    // The cap is the stage's `max` bounded by the graph's width: 3 tasks, but
    // only T1‖T2 can ever run at once, so a 3rd slot would be dead weight.
    assert!(
        js.contains("const __sm_implement = pool(2, 50000);"),
        "cap must follow the width:\n{js}"
    );
    assert!(js.contains("__tk_implement[\"T1\"] = runTask(\"T1\", [], [], __tk_implement"));
    assert!(js.contains("__tk_implement[\"T2\"] = runTask(\"T2\", [], [], __tk_implement"));
    assert!(
        js.contains("__tk_implement[\"T3\"] = runTask(\"T3\", [\"T1\"], [], __tk_implement"),
        "declared deps must be wired, and stay free of the (empty here) mutex list:\n{js}"
    );
    assert!(js.contains(
        "const __tr_implement = await settle(__tk_implement, [\"T1\", \"T2\", \"T3\"]);"
    ));
    assert!(
        js.contains("results[\"implement\"] = await safeAgent("),
        "merge lands on the stage name"
    );
    assert!(js.contains("Plan merge — you are the LAST step of this stage"));
    assert!(
        js.contains("Run the full suite once."),
        "merge_hint must reach the merge agent"
    );
    assert!(
        js.contains("taskin(__tr_implement)"),
        "merge must see the task envelopes"
    );
    assert!(
        js.contains("function runTask("),
        "plan helpers must be emitted"
    );
    // Per-task envelopes are recorded. `needs_review` on one task must NOT
    // halt the stage (that's the merge sub-agent's job to reconcile) — only
    // `blocked`/`failed`/`ask_user_questions` do, which is what `soft: true`
    // means here.
    assert!(js.contains("results[\"implement\" + \"#\" + __t.id] = __t.result"));
    assert!(js.contains("if (isPause(__t.result, true)) return pauseInfo(\"implement\" + \"#\" + __t.id, __t.result)"));
    // The tail is now compiled.
    assert!(
        js.contains("results[\"review\"] = await safeAgent("),
        "the tail must follow the tasks"
    );
    assert!(e.awaiting_plan.is_none());
    // Task prompts are self-contained: ownership, hands-off list, ordering.
    assert!(js.contains("Plan task `T3`"));
    assert!(js.contains("Files you own (your write set)"));
    assert!(js.contains("Hands off — the merge step owns these"));
    assert!(js.contains("after `T1` — dependency declared by the plan"));
    // …and the schedule is reported for persistence.
    let sched = &e.plan_schedules[0];
    assert_eq!(sched.stage, "implement");
    assert_eq!(sched.tasks.len(), 3);
    assert_eq!(sched.waves.len(), 2);
}

// A plan task returning `needs_review` — e.g. it deferred a shared-file
// change to `follow_ups`, per §7.1 — must NOT strand the run one step
// before the merge sub-agent built to reconcile exactly that. Mirrors
// `needs_review_is_a_pause` / `soft_pause_stages` for the per-task loop:
// `blocked`/`failed`/`ask_user_questions` still hard-pause (a task that is
// genuinely stuck still needs a human), only `needs_review` is relaxed —
// and the merge stage's own final envelope (last line) keeps the ordinary
// hard checkpoint, since `implement` is not itself a reviewer.
#[test]
fn plan_task_needs_review_does_not_strand_the_run_before_merge() {
    let (_ws, st) = init_run_in_workspace("plan-soft-task", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"schema":"aidp-plan.v1","summary":"s","shared_files":["src/Program.cs"],"tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"]),
        ),
    );
    let js = compile(&st).unwrap().script;
    assert!(
            js.contains(
                "if (isPause(__t.result, true)) return pauseInfo(\"implement\" + \"#\" + __t.id, __t.result)"
            ),
            "a plan task's needs_review must be soft — the merge agent decides:\n{js}"
        );
    // The merge dispatch's own envelope keeps the hard checkpoint: `implement`
    // is a plain executor stage, not a reviewer, so it is absent from `soft`.
    assert!(
            js.contains("if (isPause(results[\"implement\"], false)) return pauseInfo(\"implement\", results[\"implement\"])"),
            "the merge stage's own needs_review must still halt the run:\n{js}"
        );
}

// C4(a): the plan-driven stage's base prompt must be encoded exactly once in
// the emitted script, as a JS const, and every task dispatch plus the merge
// must reference it rather than re-embedding it. This is a pure encoding
// change: the byte size of the emitted script must strictly decrease versus
// a pre-dedup baseline (originally 30053 bytes; a real byte count, not a
// structural budget, so it drifts upward whenever the emitted script
// legitimately grows — most recently C9, Lot 3 Capitalisation, whose
// `mutex` parameter on every `runTask(...)` call plus the file-lock wait
// loop it added to `PLAN_HELPERS::runTask` moved it to 30593 bytes; this
// fixture has no write conflict, so none of that growth is the new
// "Serialized after" task-prompt subsection), and the prefix-identity guard
// (`pass2_leaves_the_dispatch_prefix_byte_identical`) must keep passing
// unmodified.
#[test]
fn plan_base_prompt_is_emitted_once() {
    let (_ws, st) = init_run_in_workspace("plan-base-once", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"schema":"aidp-plan.v1","summary":"s","shared_files":["src/Program.cs"],"tasks":[{},{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"]),
            plan_task("T3", "tests", &["T1"], &["tests/A.tests.cs"])
        ),
    );
    let e = compile(&st).unwrap();
    let js = &e.script;

    let base_const_count = js.matches("const __base_implement = ").count();
    assert_eq!(
        base_const_count, 1,
        "the stage's base prompt must be emitted as exactly one const:\n{js}"
    );

    // The opening of the `## Environment` block (part of the stage prompt)
    // must appear exactly once in the whole script — proof it is not
    // re-embedded per task or in the merge.
    // Scope the check to the `implement` stage's own block (other stages —
    // `plan`, `review` — legitimately carry their own `## Environment`
    // section in their own prompts).
    let start = js
        .find("phase(\"implement\")")
        .expect("implement stage must be emitted");
    let end = js[start..]
        .find("phase(\"review\")")
        .expect("review stage must follow");
    let implement_block = &js[start..start + end];
    let env_marker = "## Environment";
    let env_count = implement_block.matches(env_marker).count();
    assert_eq!(
            env_count, 1,
            "the implement stage's `## Environment` block must appear only once in its block:\n{implement_block}"
        );

    // Every task dispatch and the merge dispatch must reference the const.
    assert!(js.contains("__base_implement + "));
    let base_ref_count = js.matches("__base_implement + ").count();
    assert_eq!(
        base_ref_count, 4,
        "expected 3 task dispatches + 1 merge to reference __base_implement:\n{js}"
    );

    // The encoding change must actually shrink the script versus the
    // pre-change snapshot (measured on the same fixture before this task's
    // edit: 30053 bytes; re-baselined to 31469 by the Lot 3 refix run —
    // F-1/F-3/F-7/F-8 add a pattern family plus the neutralize/role-shift
    // logic to guard.rs's always-emitted `emit_js()`, and F-2/F-6 add
    // `sigNum` plus the `cancelled` checks to the always-emitted HELPERS
    // block — all legitimate, constant-size growth of the shared blocks
    // this test's `base_ref_count == 4` above already proves are reused,
    // not re-embedded per task). Re-baselined to 32632 when `up()` learned
    // to forward a repaired stage's `needs_review` report (`__REVIEWED`),
    // again constant-size growth of the always-emitted HELPERS block.
    // Re-baselined to 34511 when LOC-50 added `__answered`/`selfAnswered`
    // (self-answered questions no longer pause), same constant-size growth.
    assert!(
        js.len() < 40000,
        "expected workflow.js to stay de-duplicated (base_ref_count == 4 above already proves \
             the const is reused, not re-embedded; this is a coarse drift guard), got {} bytes",
        js.len()
    );
}

// The two passes only work because resuming replays the prefix from cache, and
// the cache keys on each `agent()` call's (prompt, opts). So every dispatch
// BEFORE the plan-driven stage must compile **byte-identically** on both
// passes — including the planner's own prompt, whose injected contract quotes
// the plan path that only exists on pass 2.
#[test]
fn pass2_leaves_the_dispatch_prefix_byte_identical() {
    let (_ws, st) = init_run_in_workspace("plan-cache", PLANNED, serde_json::json!({}));
    let p1 = compile(&st).unwrap().script;
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"])
        ),
    );
    let p2 = compile(&st).unwrap().script;
    // Everything from the first stage up to and including the planner dispatch.
    let head = |s: &str| {
        let start = s.find("  phase(").expect("a body must be emitted");
        let end = s[start..]
            .find("if (isPause(results[\"plan\"]")
            .expect("plan must dispatch");
        s[start..start + end].to_string()
    };
    assert_eq!(
            head(&p1),
            head(&p2),
            "pass 2 changed a cached dispatch — resume would re-run the pipeline instead of replaying it"
        );
}

// LOC-47 (cascade of LOC-44): the test above only proves the prefix UP TO
// `plan`'s own dispatch, on a pass 1 where `plan.json` does not exist yet.
// The two real runs the ticket cites never hit that shape — they recompile
// a LATER `aidp workflow emit` of the SAME run, where `plan.json` is
// already on disk on BOTH passes (like a resumed run crossing a further
// boundary), and a `knowledge_sync`-style write lands on disk BETWEEN
// those two emits (like LOC-44's own fixture). Neither existing test
// combines both conditions, which is exactly the blind spot that let
// `plan` (an `agent: planner` stage, already in LOC-44's scope) become a
// real Workflow cache-miss, forcing a second real LLM call that rewrote
// `plan.json` with different tasks — which a later recompile then baked
// into DIFFERENT `implement` task dispatches (T1..T8 relabelled or
// outright reworded, as observed in both journal.jsonl). Fixing LOC-44
// (freezing `RunState::knowledge_snapshot` once per run) closes this at
// the root: `plan`'s dispatch never drifts, so it is never a cache-miss,
// so `plan.json` is never rewritten a second time, so nothing downstream
// has a reason to differ either.
#[test]
fn plan_json_present_across_a_knowledge_write_keeps_implement_dispatches_identical() {
    let (_ws, mut st) = init_run_in_workspace(
        "plan-implement-knowledge-cascade",
        PLANNED,
        serde_json::json!({}),
    );
    let kdir = st.knowledge_dir();
    std::fs::create_dir_all(&kdir).unwrap();
    std::fs::write(kdir.join("overview.md"), "# Overview\n").unwrap();

    // Unlike `pass2_leaves_the_dispatch_prefix_byte_identical`, `plan.json`
    // already exists BEFORE the first compile — both passes below walk all
    // the way through `implement`'s task dispatches, which pass 1 of that
    // other test never reaches.
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"])
        ),
    );

    // Pass 1: the run's first real emit — freeze the knowledge snapshot,
    // exactly what `aidp workflow emit` does (`main.rs`) right before
    // calling `compile`.
    run::ensure_knowledge_snapshot(&mut st).unwrap();
    let e1 = compile(&st).unwrap();
    assert!(
        e1.awaiting_plan.is_none(),
        "plan.json already exists on disk, this pass must compile straight through"
    );

    // Between the two emits, a `knowledge_sync`-style stage does its real
    // job and writes a new transverse note — the same sequence LOC-44's
    // own fixture uses, now combined with an already-materialized
    // `plan.json` instead of an empty knowledge base with no plan stage.
    std::fs::write(kdir.join("operations.md"), "# Operations\n").unwrap();

    // Pass 2: a later re-emit of the SAME run. `ensure_knowledge_snapshot`
    // must be a no-op — the snapshot captured on pass 1 is reused as-is.
    run::ensure_knowledge_snapshot(&mut st).unwrap();
    let e2 = compile(&st).unwrap();

    assert_eq!(
        e1.script, e2.script,
        "pass 2 changed the compiled script after an intervening knowledge write — \
             resume would treat `plan` as a cache-miss and re-run it for real, and \
             everything `aidp workflow emit` compiles from its (possibly rewritten) \
             plan.json — starting with `implement`'s task dispatches — with it (LOC-47)"
    );

    // Pin the ticket's own symptom directly, not just the whole-script
    // diff above: both `implement` task dispatch labels (`task_label`'s
    // `"<stage> · <id> <title>"` format) must each still appear exactly
    // once, unrelabelled and unreworded.
    for label in ["implement · T1 do T1", "implement · T2 do T2"] {
        let needle = format!("\"{label}\"");
        assert_eq!(
            e2.script.matches(&needle).count(),
            1,
            "expected exactly one dispatch labelled {label:?} — a different count means \
                 `implement`'s tasks were recompiled from a plan.json this test never wrote:\n{}",
            e2.script
        );
    }
}

const FEATURE_WRITER_PLANNED: &str =
    include_str!("../fixtures/workflow_emit/feature_writer_planned.yaml");

// The `awaiting_plan` round-trip of a run whose feature-layer writer runs
// before `plan` (every dev-* pipeline composing knowledge-update). Pass 1
// is compiled before the note exists (`create`); the writer then creates
// it; pass 2 must still carry `create`, byte for byte, or resume re-runs
// the writer and — through its new summary — `plan`, which rewrites the
// plan.json pass 2 was compiled from (run LOC-24/20260923-090055-f920).
#[test]
fn pass2_keeps_the_feature_writer_and_plan_dispatches_identical_after_the_note_is_written() {
    let (_ws, mut st) = init_run_in_workspace(
        "feature-writer-plan-cache",
        FEATURE_WRITER_PLANNED,
        serde_json::json!({"feature": "Auto-exclusion temporaire du joueur"}),
    );
    let features_dir = crate::knowledge::features_dir(&st.knowledge_dir());
    std::fs::create_dir_all(&features_dir).unwrap();

    run::ensure_knowledge_snapshot(&mut st).unwrap();
    let e1 = compile(&st).unwrap();
    assert!(
        e1.awaiting_plan.is_some(),
        "pass 1 must stop at `implement`"
    );
    assert!(
        e1.script.contains("this is a NEW feature"),
        "sanity: pass 1 must tell the writer to create the note"
    );

    // Between the passes: the writer creates its note, the planner writes plan.json.
    std::fs::write(
        features_dir.join("auto-exclusion-temporaire.md"),
        "---
aidp: knowledge-feature
feature: Auto-exclusion temporaire du joueur
slug: auto-exclusion-temporaire
status: active
---

# Auto-exclusion temporaire
",
    )
    .unwrap();
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"])
        ),
    );

    run::ensure_knowledge_snapshot(&mut st).unwrap();
    let e2 = compile(&st).unwrap();
    let head = |s: &str| {
        let start = s.find("  phase(").expect("a body must be emitted");
        let end = s[start..]
            .find("if (isPause(results[\"plan\"]")
            .expect("plan must dispatch");
        s[start..start + end].to_string()
    };
    assert_eq!(
            head(&e1.script),
            head(&e2.script),
            "pass 2 changed the feature writer's or the planner's dispatch after the note was              written — resume would re-run both instead of replaying them"
        );

    // Not vacuous: without the frozen decision, pass 2 does drift.
    let mut unsnapshotted = st.clone();
    unsnapshotted.knowledge_snapshot = None;
    let live = compile(&unsnapshotted).unwrap().script;
    assert_ne!(head(&e1.script), head(&live));
}

// A composite (`type: pipeline`) entry stage forces the same kind of pass
// boundary as `plan_tasks:`, but crossing it is an explicit persisted
// marker (`state.crossed_boundaries`) rather than a real-world artifact —
// see the module-level comment on the composite-entry branch in `compile`.
// This pins both halves: pass 1 stops before the composite's stages, and
// everything compiled before the boundary stays byte-identical once
// crossed, or `Workflow{resumeFromRunId}` would re-run it.
// LOC-48: `max_stages_per_pass: 1` pins this fixture (`code` + the
// composite's single stage `r`, 2 total) below the default budget (10),
// which would otherwise merge it into one pass and never reach the
// boundary this test exists to pin.
#[test]
fn composite_boundary_dispatch_prefix_stays_byte_identical_across_crossing() {
    const CHILD: &str = include_str!("../fixtures/workflow_emit/composite_boundary_dispatch_prefix_stays_byte_identical_across_crossing_child.yaml");
    const PARENT: &str = include_str!("../fixtures/workflow_emit/composite_boundary_dispatch_prefix_stays_byte_identical_across_crossing_parent.yaml");
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("revchild.yaml"), CHILD).unwrap();
    let p = ws.join("parent.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let mut st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();

    let e1 = compile(&st).unwrap();
    let a = e1
        .awaiting_composite
        .as_ref()
        .expect("must stop at the composite boundary");
    assert_eq!(a.stage, "review__r");
    assert_eq!(a.composite, "review");
    // The awaiting_composite log line legitimately names the stage it stopped
    // before — check for an actual DISPATCH (an assignment into `results`),
    // not the bare substring, which the diagnostic message alone would satisfy.
    assert!(
        !e1.script.contains("results[\"review__r\"] ="),
        "pass 1 must not dispatch past the boundary:\n{}",
        e1.script
    );

    st.crossed_boundaries.push("review".to_string());
    let e2 = compile(&st).unwrap();
    assert!(
        e2.awaiting_composite.is_none(),
        "the boundary is crossed, pass 2 must compile straight through"
    );
    assert!(
        e2.script.contains("results[\"review__r\"] ="),
        "pass 2 must dispatch the composite's stages"
    );

    // Everything before the boundary — `code`'s own dispatch — must
    // compile byte-identically on both passes.
    let head = |s: &str| {
        let start = s.find("  phase(\"code\")").expect("code must dispatch");
        let end = s[start..]
            .find("if (isPause(results[\"code\"]")
            .expect("code must pause-check");
        s[start..start + end].to_string()
    };
    assert_eq!(
        head(&e1.script),
        head(&e2.script),
        "pass 2 changed a cached dispatch before the boundary"
    );
}

// A stage that is both a composite's entry AND (via inheritance) a gate
// must produce `awaiting_composite` first, `awaiting_gate` only once
// crossed — never the other order, and never both in the same pass.
// LOC-48: `max_stages_per_pass: 1` pins this fixture below the default
// budget, same reason as `composite_boundary_dispatch_prefix_...` above.
#[test]
fn it_composite_boundary_is_checked_before_gate() {
    const CHILD: &str = include_str!(
        "../fixtures/workflow_emit/composite_boundary_is_checked_before_gate_child.yaml"
    );
    const PARENT: &str = include_str!(
        "../fixtures/workflow_emit/composite_boundary_is_checked_before_gate_parent.yaml"
    );
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("gatechild.yaml"), CHILD).unwrap();
    let p = ws.join("parent.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let mut st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();

    // Pass 1: the composite boundary wins over the inherited gate.
    let e1 = compile(&st).unwrap();
    assert_eq!(
        e1.awaiting_composite.as_ref().map(|a| a.composite.as_str()),
        Some("review"),
        "the composite boundary must be checked before the inherited gate"
    );
    assert!(
        !e1.script.contains("awaiting_gate"),
        "pass 1 must not even discover the gate yet:\n{}",
        e1.script
    );
    assert!(e1.gates.is_empty(), "no gate is reachable before crossing");

    // Pass 2: once crossed, the same stage is an ordinary gated stage.
    st.crossed_boundaries.push("review".to_string());
    let e2 = compile(&st).unwrap();
    assert!(
        e2.awaiting_composite.is_none(),
        "the boundary is already crossed"
    );
    assert_eq!(e2.gates, vec!["review__r".to_string()]);
    assert!(
        e2.script.contains(
            "if (!__approved.includes(\"review__r\")) return { status: \"awaiting_gate\""
        ),
        "the inherited gate guard must now be emitted:\n{}",
        e2.script
    );
}

// LOC-48: under the DEFAULT budget (`DEFAULT_MAX_STAGES_PER_PASS`, 10), a
// composite small enough to fit alongside its neighbours must merge
// straight into the pass instead of forcing its own `awaiting_composite` —
// the counterpart to the two tests above, which pin the "still stops"
// side with an explicit tiny budget.
#[test]
fn composite_boundary_merges_small_composites_under_default_budget() {
    const CHILD_A: &str = include_str!("../fixtures/workflow_emit/composite_boundary_merges_small_composites_under_default_budget_child_a.yaml");
    const CHILD_B: &str = include_str!("../fixtures/workflow_emit/composite_boundary_merges_small_composites_under_default_budget_child_b.yaml");
    const PARENT: &str = include_str!("../fixtures/workflow_emit/composite_boundary_merges_small_composites_under_default_budget_parent.yaml");
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("smallchild_a.yaml"), CHILD_A).unwrap();
    std::fs::write(ws.join("smallchild_b.yaml"), CHILD_B).unwrap();
    let p = ws.join("parent.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();

    let e = compile(&st).unwrap();
    assert!(
        e.awaiting_composite.is_none(),
        "two 1-stage composites (4 stages total) must fit under the default \
             budget ({DEFAULT_MAX_STAGES_PER_PASS}) and compile straight through:\n{}",
        e.script
    );
    assert!(
        e.script.contains("results[\"extra_a__s\"] ="),
        "the first composite's stage must be dispatched in the same pass:\n{}",
        e.script
    );
    assert!(
        e.script.contains("results[\"extra_b__s\"] ="),
        "the second composite's stage must be dispatched in the same pass:\n{}",
        e.script
    );
    assert!(
        e.script.contains("results[\"pr\"] ="),
        "the pass must reach past both composites:\n{}",
        e.script
    );
}

// The flip side: a composite big enough to blow the budget on its own
// must still stop, even though nothing was accumulated before it — this
// is what keeps LOC-48 from regressing PR #51's original fix (one huge
// composite must not be swept wholesale into a single Workflow pass).
#[test]
fn composite_boundary_still_fires_for_a_composite_that_alone_exceeds_the_budget() {
    const CHILD: &str = include_str!("../fixtures/workflow_emit/composite_boundary_still_fires_for_a_composite_that_alone_exceeds_the_budget_child.yaml");
    const PARENT: &str = include_str!("../fixtures/workflow_emit/composite_boundary_still_fires_for_a_composite_that_alone_exceeds_the_budget_parent.yaml");
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("bigchild.yaml"), CHILD).unwrap();
    let p = ws.join("parent.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();

    let e = compile(&st).unwrap();
    assert_eq!(
        e.awaiting_composite.as_ref().map(|a| a.composite.as_str()),
        Some("big"),
        "a 3-stage composite must still stop under a budget of 2, even as \
             the pipeline's very first stage (nothing accumulated before it):\n{}",
        e.script
    );
}

// LOC-48's acceptance criterion, proven end to end on the real template:
// compile `templates/dev-aidp.yaml` step by step exactly as the v7 driver
// does (compile → cross a composite boundary or supply the plan → compile
// again), counting how many `aidp workflow emit` calls a `track: standard`
// run actually needs. Measured: 3 `compile()` passes under the default
// budget (+1 real `Workflow` invocation for `pr`'s own unconditional
// `gate: true`, uncounted here — 4 total, the ticket's target) versus 5
// passes (6 total) under a budget of 1, which reproduces pre-LOC-48's
// unconditional per-composite stop on this template.
#[test]
fn it_dev_aidp_composite_budget_reduces_workflow_passes_end_to_end() {
    let skill_root = aidp_testkit::fixtures::skill_root();
    let tpl_path = skill_root.join("templates").join("dev-aidp.yaml");
    let pipe = pipeline::load(&tpl_path).expect("load templates/dev-aidp.yaml");

    // Walks `pipe` (as loaded from `path`, with `settings` already
    // overridden by the caller) to a fully-resolved compile, feeding a
    // trivial plan.json the moment `implement`'s `plan_tasks:` asks for
    // one and crossing every composite boundary `compile()` actually
    // decides to stop for. Returns the number of `compile()` calls
    // needed — i.e. the number of real `aidp workflow emit` +
    // `Workflow(resumeFromRunId)` round-trips, not counting a runtime
    // gate pause (`pr`'s `gate: true`, unconditional and untouched by
    // this ticket — see SKILL.md's `awaiting_gate` branch).
    fn count_passes(path: &std::path::Path, ws: &std::path::Path) -> usize {
        std::fs::create_dir_all(ws).unwrap();
        let mut st = run::init(
                path,
                ws,
                serde_json::json!({"feature": "loc48-budget-test", "prompt": "measure the number of Workflow passes"}),
                None,
            )
            .unwrap_or_else(|e| panic!("run::init: {e}"));

        let mut passes = 0usize;
        loop {
            passes += 1;
            assert!(passes <= 20, "runaway loop — compile() never resolved");
            let e = compile(&st).unwrap_or_else(|e| panic!("compile pass {passes}: {e}"));
            if let Some(a) = e.awaiting_plan {
                assert_eq!(
                    a.stage, "implement",
                    "dev-aidp.yaml's only plan_tasks: stage is `implement`"
                );
                let dir = st.run_dir.join("plan");
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                        dir.join("plan.json"),
                        format!(
                            r#"{{"schema":"aidp-plan.v1","summary":"s","shared_files":[],"tasks":[{}]}}"#,
                            plan_task("T1", "code", &[], &["src/x.rs"])
                        ),
                    )
                    .unwrap();
                continue;
            }
            if let Some(a) = e.awaiting_composite {
                st.crossed_boundaries.push(a.composite);
                continue;
            }
            // Fully resolved: everything after this compiles straight
            // through in one script (`pr`'s own `gate: true` pauses it
            // at runtime, one more real invocation not counted here).
            return passes;
        }
    }

    let ws_after_guard = tempfile::tempdir().unwrap();
    let ws_after = ws_after_guard.path().to_path_buf();
    let after = count_passes(&tpl_path, &ws_after);
    assert!(
        after < 6,
        "default settings.max_stages_per_pass ({DEFAULT_MAX_STAGES_PER_PASS}) must need \
             fewer than the pre-LOC-48 baseline (6 real invocations incl. the pr gate \
             pause) — got {after} compile() passes"
    );
    assert!(
        after <= 4,
        "ticket target is close to 4 compile() passes on dev-aidp.yaml \
             (track: standard) — got {after}"
    );

    // Contrast: a budget of 1 forces a boundary at every composite,
    // reproducing pre-LOC-48's unconditional behaviour on this template
    // (every composite here has >=1 preceding stage, so it always
    // exceeds a budget of 1) — this is the "before" this ticket improved
    // on, pinned so a future change to the default can see the delta it
    // is trading against. Written to a scratch dir, NOT `templates/`
    // (that's a shared discovery root every other test scans too, and
    // this file would collide with the real `id: dev-aidp`). Since LOC-27
    // a `uses:` absolute path outside every composition root is refused,
    // so every other template is copied next to the pinned file instead:
    // the pinned file's own directory is a discovery root, and the
    // fragments (and their own nested `uses:`) resolve there by id.
    let mut pinned = pipe.clone();
    pinned.settings.max_stages_per_pass = Some(1);
    let real_templates_dir = skill_root.join("templates");
    let scratch_guard = tempfile::tempdir().unwrap();
    let scratch = scratch_guard.path().to_path_buf();
    std::fs::create_dir_all(&scratch).unwrap();
    for entry in std::fs::read_dir(&real_templates_dir).unwrap() {
        let p = entry.unwrap().path();
        let is_yaml = p.extension().and_then(|e| e.to_str()) == Some("yaml");
        if is_yaml && p.file_name().and_then(|n| n.to_str()) != Some("dev-aidp.yaml") {
            std::fs::copy(&p, scratch.join(p.file_name().unwrap())).unwrap();
        }
    }
    let pinned_path = scratch.join("dev-aidp-pinned.yaml");
    std::fs::write(
        &pinned_path,
        serde_yaml::to_string(&pinned).expect("serialize pinned pipeline"),
    )
    .unwrap();
    let ws_before_guard = tempfile::tempdir().unwrap();
    let ws_before = ws_before_guard.path().to_path_buf();
    let before = count_passes(&pinned_path, &ws_before);
    assert!(
        before > after,
        "budget:1 (unconditional-boundary baseline) must need strictly more \
             passes than the default — before={before}, after={after}"
    );
}

const PLANNED_SKILLS_BY_KIND: &str =
    include_str!("../fixtures/workflow_emit/planned_skills_by_kind.yaml");

/// Return the one-line JS string-literal body of `const <name> = "...";`.
fn const_text<'a>(js: &'a str, name: &str) -> &'a str {
    let marker = format!("const {name} = ");
    let start = js
        .find(&marker)
        .unwrap_or_else(|| panic!("`{name}` must be emitted:\n{js}"))
        + marker.len();
    let end = js[start..]
        .find(";\n")
        .unwrap_or_else(|| panic!("`{name}` const must be terminated:\n{js}"));
    &js[start..start + end]
}

// T09 — a task's prompt only loads the skills its own kind is allowed
// (opt-in via `plan_tasks.skills_by_kind`); the merge dispatch always keeps
// the stage's full list, and skills not mentioned for any kind stay common.
#[test]
fn skills_by_kind_prunes_the_task_prompt() {
    let (_ws, st) = init_run_in_workspace(
        "plan-skills-by-kind",
        PLANNED_SKILLS_BY_KIND,
        serde_json::json!({}),
    );
    write_plan(
        &st,
        &format!(
            r#"{{"schema":"aidp-plan.v1","summary":"s","tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "tests", &[], &["tests/A.tests.cs"])
        ),
    );
    let e = compile(&st).unwrap();
    let js = &e.script;

    let code_base = const_text(js, "__base_implement_code");
    assert!(code_base.contains("`skill-a-x1`"), "{code_base}");
    assert!(!code_base.contains("`skill-b-x1`"), "{code_base}");

    let tests_base = const_text(js, "__base_implement_tests");
    assert!(tests_base.contains("`skill-b-x1`"), "{tests_base}");
    assert!(!tests_base.contains("`skill-a-x1`"), "{tests_base}");

    // Merge dispatches on the stage's full, unreduced base.
    let merge_base = const_text(js, "__base_implement");
    assert!(merge_base.contains("`skill-a-x1`"), "{merge_base}");
    assert!(merge_base.contains("`skill-b-x1`"), "{merge_base}");

    // The task dispatches must reference their own kind's variant.
    assert!(js.contains("__base_implement_code + "));
    assert!(js.contains("__base_implement_tests + "));
}

// T09 — a hypothetical upstream restriction (e.g. a future `skills_by`
// that already trimmed the stage's skill list) composes with
// `skills_by_kind` as a further restriction: the result stays non-empty
// and stays a subset of what came in, because `restrict_skills_for_kind`
// only ever removes, never adds.
#[test]
fn skills_by_kind_composes_with_skills_by_without_emptying() {
    // Simulates an already-reduced upstream list (as a future `skills_by`
    // mechanism would hand this function).
    let upstream: Vec<String> = vec!["a".to_string(), "b".to_string()];
    let mut by_kind = BTreeMap::new();
    by_kind.insert("code".to_string(), vec!["a".to_string()]);
    by_kind.insert("tests".to_string(), vec!["b".to_string()]);

    let restricted = restrict_skills_for_kind(&upstream, &by_kind, "code");
    assert!(!restricted.is_empty(), "must not empty a non-empty input");
    assert!(
        restricted.iter().all(|s| upstream.iter().any(|u| u == s)),
        "result must stay a subset of the upstream list: {restricted:?}"
    );
    assert!(restricted.contains("a"));
    assert!(!restricted.contains("b"));
}

// The stage prompt is v7-only now: no leftover v4 mechanism (PreToolUse hook,
// agent-session.lock) and no dead CLI commands the runtime removed.
#[test]
fn emitted_prompt_has_no_v4_markers() {
    let (_ws, st) = init_run_in_workspace(
        "prompt-v7-only",
        DEVISH,
        serde_json::json!({"track": "standard"}),
    );
    let js = compile(&st).unwrap().script;
    for marker in [
        "agent-session.lock",
        "PreToolUse",
        "aidp orchestrate",
        "run stage-start",
        "run stage-end",
    ] {
        assert!(
            !js.contains(marker),
            "emitted script must not contain {marker:?}:\n{js}"
        );
    }
    assert!(
        js.contains("AIDP_RUN="),
        "emitted prompt must still carry the AIDP_RUN= env block"
    );
}

// The reason `files` exists: two concurrent writers of one file are ordered
// by aidp, the extra dep reaches the emitted graph, and the run logs why.
#[test]
fn plan_tasks_write_conflicts_are_serialized_in_the_emitted_graph() {
    let (_ws, st) = init_run_in_workspace("plan-conflict", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/Shared.cs"]),
            plan_task("T2", "code", &[], &["src/Shared.cs"])
        ),
    );
    let e = compile(&st).unwrap();
    let js = &e.script;
    // C9 (Lot 3, Capitalisation): a write conflict with no declared `needs`
    // must land purely on the new `mutex` argument — `deps` stays empty, so
    // a failure in T1 cannot cancel T2 through `runTask`'s `deps` loop.
    assert!(
        js.contains("__tk_implement[\"T2\"] = runTask(\"T2\", [], [\"T1\"], __tk_implement"),
        "the inserted ordering must reach the graph as a mutex, not a dep:\n{js}"
    );
    assert!(
        js.contains("`T2` runs after `T1` (both write src/Shared.cs)"),
        "the decision must be logged, not silent:\n{js}"
    );
    assert!(
        js.contains("ordering inserted by `aidp`: both tasks write `src/Shared.cs`"),
        "and explained to the task itself:\n{js}"
    );
    assert!(
        js.contains("Serialized after (file-lock only — not a dependency)"),
        "the task prompt must say plainly this is not a dependency:\n{js}"
    );
    assert_eq!(e.plan_schedules[0].serializations.len(), 1);
    assert_eq!(
        e.plan_schedules[0]
            .tasks
            .iter()
            .find(|t| t.id == "T2")
            .unwrap()
            .deps,
        Vec::<String>::new(),
        "the persisted schedule must not carry the mutex edge on `deps` either"
    );
    assert_eq!(
        e.plan_schedules[0]
            .tasks
            .iter()
            .find(|t| t.id == "T2")
            .unwrap()
            .mutex_after,
        vec!["T1".to_string()]
    );
}

// A plan that resolves to one task costs exactly what a plain stage costs:
// no semaphore, no merge agent, and the task owns the stage's outputs.
#[test]
fn plan_tasks_single_task_short_circuits_the_merge() {
    let (_ws, st) = init_run_in_workspace("plan-single", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"])
        ),
    );
    let js = compile(&st).unwrap().script;
    assert!(
        js.contains("results[\"implement\"] = await safeAgent("),
        "the sole task IS the stage"
    );
    assert!(
        !js.contains("__tk_implement["),
        "no scheduler for a single task:\n{js}"
    );
    assert!(!js.contains("__sm_implement"), "no concurrency cap needed");
    assert!(
        js.contains("resolved to a SINGLE task"),
        "and it is told it owns the outputs"
    );
    assert!(
        js.contains("results[\"review\"] = await safeAgent("),
        "the tail still follows"
    );
}

// …but cheap must not mean unaccounted. The shortcut used to write only the
// stage key, so the `<stage>#<task_id>` marker reconcile folds into the plan
// schedule never existed: a single-task run that had really changed files
// reported `0/1 task(s) succeeded` and left the task `pending`.
#[test]
fn plan_tasks_single_task_still_records_its_per_task_outcome() {
    let (_ws, st) = init_run_in_workspace("plan-single-acct", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"])
        ),
    );
    let js = compile(&st).unwrap().script;
    assert!(
        js.contains("results[\"implement\" + \"#\" + \"T1\"] = results[\"implement\"];"),
        "the sole task's outcome must be recorded under `<stage>#<task_id>`:\n{js}"
    );
    // Aliased, not copied, and aliased *before* the token fold — so the fold
    // reaches the task envelope too and reconcile can attribute the spend.
    let alias = js.find("results[\"implement\" + \"#\" + \"T1\"]").unwrap();
    let fold = js.find("withTokens(results[\"implement\"]").unwrap();
    assert!(
        alias < fold,
        "alias must precede withTokens so the task inherits the delta:\n{js}"
    );
    // And the run log says 1/1 rather than going silent on the task count.
    assert!(
        js.contains("logTasks(\"implement\", [{ id: \"T1\", result: results[\"implement\"] }]);"),
        "a single task must still be counted in the run log:\n{js}"
    );
}

// The same shortcut is reached when a `kinds:` filter leaves a stage exactly
// one claimed task — the accounting marker must be there too, not just on the
// "plan happens to have one task" path.
#[test]
fn plan_tasks_single_claimed_task_records_its_outcome_too() {
    let (_ws, st) = init_run_in_workspace("plan-split-acct", SPLIT_KINDS, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{},{},{}]}}"#,
            plan_task("C1", "code", &[], &["src/A.cs"]),
            plan_task("C2", "code", &[], &["src/B.cs"]),
            plan_task("V1", "tests", &["C1"], &["tests/A.cs"])
        ),
    );
    let js = compile(&st).unwrap().script;
    assert!(
        js.contains("results[\"tests\" + \"#\" + \"V1\"] = results[\"tests\"];"),
        "the lone claimed task of a multi-task plan is still a task:\n{js}"
    );
    // The multi-task sibling keeps recording per task, so both stages of the
    // same plan account for their work — they must not diverge.
    assert!(js.contains("results[\"code\" + \"#\" + __t.id] = __t.result"));
}

// `kinds:` lets two stages split the plan; each stage only compiles its own.
#[test]
fn plan_tasks_kinds_split_the_plan_across_stages() {
    let (_ws, st) = init_run_in_workspace("plan-split", SPLIT_KINDS, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{},{},{}]}}"#,
            plan_task("C1", "code", &[], &["src/A.cs"]),
            plan_task("C2", "code", &[], &["src/B.cs"]),
            plan_task("V1", "tests", &["C1"], &["tests/A.cs"])
        ),
    );
    let e = compile(&st).unwrap();
    let js = &e.script;
    assert!(
        js.contains("__tk_code[\"C1\"] = runTask("),
        "code stage claims C1/C2"
    );
    assert!(js.contains("__tk_code[\"C2\"] = runTask("));
    assert!(
        !js.contains("__tk_code[\"V1\"]"),
        "the tests task is not the code stage's"
    );
    // One claimed task in the tests stage → single-task short circuit there.
    assert!(js.contains("results[\"tests\"] = await safeAgent("));
    assert_eq!(e.plan_schedules.len(), 2);
    // The cross-stage dependency is surfaced rather than silently dropped.
    let tests = e
        .plan_schedules
        .iter()
        .find(|s| s.stage == "tests")
        .unwrap();
    assert!(
        tests.warnings.iter().any(|w| w.contains("does not claim")),
        "{:?}",
        tests.warnings
    );
}

// Routing a plan across `kinds:`-filtered stages can silently drop work if the
// planner emits a kind nobody claims. Say it, once.
#[test]
fn plan_tasks_unclaimed_kinds_are_reported() {
    const TESTS_ONLY: &str = include_str!(
        "../fixtures/workflow_emit/plan_tasks_unclaimed_kinds_are_reported_tests_only.yaml"
    );
    let (_ws, st) = init_run_in_workspace("plan-orphans", TESTS_ONLY, serde_json::json!({}));
    write_plan(
        &st,
        &format!(
            r#"{{"tasks":[{},{},{}]}}"#,
            plan_task("V1", "tests", &[], &["tests/a.cs"]),
            plan_task("C1", "code", &[], &["src/a.cs"]),
            plan_task("D1", "docs", &[], &["README.md"])
        ),
    );
    let js = compile(&st).unwrap().script;
    assert!(
        js.contains("results[\"tests\"] = await safeAgent("),
        "the one claimed task runs"
    );
    assert!(
        !js.contains("Plan task `C1`"),
        "an unclaimed task must not be dispatched"
    );
    assert!(
        js.contains("2 task(s) will NOT run")
            && js.contains("C1 (code)")
            && js.contains("D1 (docs)"),
        "unclaimed tasks must be reported in the run log:\n{js}"
    );
}

// A malformed plan fails emit loudly, naming the stage and what to fix — the
// plan is machine input, so a silent degradation would run the wrong work.
#[test]
fn malformed_plan_fails_emit_with_the_stage_named() {
    let (_ws, st) = init_run_in_workspace("plan-bad", PLANNED, serde_json::json!({}));
    write_plan(
        &st,
        r#"{"tasks":[{"id":"T1","title":"t","kind":"code","prompt":"p"}]}"#,
    );
    let err = compile(&st).unwrap_err().to_string();
    assert!(
        err.contains("implement"),
        "must name the consuming stage: {err}"
    );
    assert!(
        err.contains("files"),
        "must name the offending field: {err}"
    );

    // Prose instead of JSON is the likeliest planner slip.
    write_plan(&st, "# Plan\n\n1. do the thing\n");
    let err2 = compile(&st).unwrap_err().to_string();
    assert!(err2.contains("not valid JSON"), "got: {err2}");
}

#[test]
fn plan_tasks_config_errors() {
    // `from` must point at an artifact, not a signal.
    const BADFROM: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_badfrom.yaml");
    let (_ws, st) = init_run_in_workspace("plan-badfrom", BADFROM, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("stages.<stage>.outputs.<name>"), "got: {e}");

    // The producer must declare that output.
    const NOOUT: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_noout.yaml");
    let (_ws, st) = init_run_in_workspace("plan-noout", NOOUT, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("declares outputs"), "got: {e}");

    // `outputs: [plan.md, plan.json]` resolves `outputs.plan` to the PROSE
    // file — refuse it instead of parsing Markdown as a task graph.
    const PROSE: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_prose.yaml");
    let (_ws, st) = init_run_in_workspace("plan-prose", PROSE, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("must be JSON"), "got: {e}");
    assert!(e.contains("plan.md"), "must name what it resolved to: {e}");

    // Two stages claiming everything would run every task twice.
    const DOUBLE: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_double.yaml");
    let (_ws, st) = init_run_in_workspace("plan-double", DOUBLE, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("claim EVERY plan task"), "got: {e}");

    // Overlapping kinds, same problem.
    const OVERLAP: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_overlap.yaml");
    let (_ws, st) = init_run_in_workspace("plan-overlap", OVERLAP, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("must be disjoint"), "got: {e}");

    // A two-pass stage cannot live inside a loop body.
    const INLOOP: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_inloop.yaml");
    let (_ws, st) = init_run_in_workspace("plan-inloop", INLOOP, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("not supported in a loop body"), "got: {e}");

    // fan_out and plan_tasks are two ways to shard the same stage.
    const BOTH: &str = include_str!("../fixtures/workflow_emit/plan_tasks_config_errors_both.yaml");
    let (_ws, st) = init_run_in_workspace("plan-both", BOTH, serde_json::json!({}));
    let e = compile(&st).unwrap_err().to_string();
    assert!(e.contains("mutually exclusive"), "got: {e}");
}

// Composition namespaces the producer, so a plan-driven stage pulled in via
// `type: pipeline` must have its `from:` rewritten with it.
// LOC-48: `max_stages_per_pass: 1` pins this fixture below the default
// budget (10) — `phase` alone is 2 stages (`plan` + `build`), which the
// default would happily merge into pass 1 and never stop for.
#[test]
fn plan_tasks_from_survives_composition() {
    const CHILD: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_from_survives_composition_child.yaml");
    const PARENT: &str =
        include_str!("../fixtures/workflow_emit/plan_tasks_from_survives_composition_parent.yaml");
    let ws_guard = tempfile::tempdir().unwrap();
    let ws = ws_guard.path().to_path_buf();
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("planchild.yaml"), CHILD).unwrap();
    let p = ws.join("parent.yaml");
    std::fs::write(&p, PARENT).unwrap();
    let mut st = run::init(&p, &ws, serde_json::json!({}), None).unwrap();
    // Pass 1 stops at the composite boundary — `phase__plan` (the
    // namespaced producer) is ALSO `phase`'s entry stage, and the
    // composite check is routed before plan_tasks ever gets a look.
    let e0 = compile(&st).unwrap();
    assert_eq!(
        e0.awaiting_composite.as_ref().map(|a| a.composite.as_str()),
        Some("phase"),
        "must stop at the composite boundary before the namespaced producer"
    );
    st.crossed_boundaries.push("phase".to_string());
    // Pass 2 (now past the boundary) resolves the namespaced producer's
    // artifact path.
    let e = compile(&st).unwrap();
    let a = e.awaiting_plan.expect("must await the namespaced plan");
    assert_eq!(a.producer, "phase__plan");
    assert!(
        a.plan_path
            .replace('\\', "/")
            .contains("/phase__plan/plan.json"),
        "path must follow the namespaced producer: {}",
        a.plan_path
    );
    // Pass 3 expands under the namespaced consumer.
    let dir = st.run_dir.join("phase__plan");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plan.json"),
        format!(
            r#"{{"tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"])
        ),
    )
    .unwrap();
    let js = compile(&st).unwrap().script;
    assert!(
        js.contains("__tk_phase__build[\"T1\"] = runTask("),
        "got:\n{js}"
    );
}

// WI-P3-1: a loop with no `when:` is unconditional → iterate to max_iterations
// (previously fell through to the disable-the-loop branch).
#[test]
fn loop_without_when_iterates_to_max() {
    const NOWHEN: &str =
        include_str!("../fixtures/workflow_emit/loop_without_when_iterates_to_max_nowhen.yaml");
    let (_ws, st) = init_run_in_workspace("nowhenloop", NOWHEN, serde_json::json!({}));
    let js = emit(&st).unwrap();
    assert!(
        js.contains("__it_spin < 3 && true"),
        "when-less loop must iterate to max unconditionally:\n{js}"
    );
}

// `compile_with` takes an already-loaded `Pipeline` instead of loading it from
// `state.pipeline_path` itself. Compiling the same pipeline both ways must
// produce byte-identical scripts.
#[test]
fn compile_with_matches_compile_on_the_same_pipeline() {
    let (_ws, st) = init_run_in_workspace("compile-with-parity", DEVISH, serde_json::json!({}));
    let from_disk = compile(&st).unwrap().script;
    let pipe: Pipeline = serde_yaml::from_str(DEVISH).unwrap();
    let from_memory = compile_with(&pipe, &st).unwrap().script;
    assert_eq!(
        from_disk, from_memory,
        "compile_with must compile an in-memory Pipeline identically to compile()'s own disk load"
    );
}

/// `compile_with` must work from the pipeline in hand: with every pipeline file
/// on disk corrupted, a reload would fail the compile.
#[test]
fn compile_with_does_not_reload_the_pipeline() {
    let (ws, st) = init_run_in_workspace("compile-with-no-reload", DEVISH, serde_json::json!({}));
    let pipe: Pipeline = serde_yaml::from_str(DEVISH).unwrap();
    let unparsable_pipeline = ": not a pipeline [";
    std::fs::write(ws.path().join("pipe.yaml"), unparsable_pipeline).unwrap();
    std::fs::write(
        st.run_dir.join("effective-pipeline.yaml"),
        unparsable_pipeline,
    )
    .unwrap();

    compile_with(&pipe, &st).expect("compile_with must not reload the pipeline from disk");
    compile_with(&pipe, &st).expect("compile_with must not reload the pipeline from disk");
}

/// A `PlanSource` that hands back an in-memory `Plan`, never touching disk —
/// proves `emit_plan_stage` reads through the injected source rather than
/// `plan::load`.
struct FakePlanSource(plan::Plan);

impl PlanSource for FakePlanSource {
    fn load(&self, _path: &std::path::Path) -> anyhow::Result<Option<plan::Plan>> {
        Ok(Some(self.0.clone()))
    }
}

// `compile_with_source` with an in-memory plan: pass 2 compiles directly, no
// `awaiting_plan`, and no `plan.json` ever touches disk.
#[test]
fn compile_with_source_schedules_an_in_memory_plan_with_no_disk_write() {
    let (_ws, st) = init_run_in_workspace(
        "compile-with-source-fake-plan",
        PLANNED,
        serde_json::json!({}),
    );
    let pipe: Pipeline = serde_yaml::from_str(PLANNED).unwrap();
    let plan_json = format!(
        r#"{{"tasks":[{},{},{}]}}"#,
        plan_task("T1", "code", &[], &["src/A.cs"]),
        plan_task("T2", "code", &[], &["src/B.cs"]),
        plan_task("T3", "code", &[], &["src/C.cs"]),
    );
    let plan = plan::parse(&plan_json).unwrap();
    let source = FakePlanSource(plan);

    let e = compile_with_source(&pipe, &st, &source).unwrap();
    assert!(
        e.awaiting_plan.is_none(),
        "an in-memory plan must let pass 2 compile directly, no awaiting_plan"
    );
    assert!(
        !st.run_dir.join("plan").join("plan.json").is_file(),
        "the fake plan source must never touch disk"
    );
    assert!(e.script.contains("pool("), "got:\n{}", e.script);
    for id in ["T1", "T2", "T3"] {
        let needle = format!("__tk_implement[\"{id}\"] = runTask(");
        assert!(
            e.script.contains(&needle),
            "expected a dispatch for task `{id}`, got:\n{}",
            e.script
        );
    }
}

// An unparseable `when:` no longer only prints a warning — it is also
// collected into `Emitted.warnings`, so a caller can surface it without
// scraping stderr.
#[test]
fn unparseable_when_surfaces_in_emitted_warnings() {
    const BADWHEN: &str = include_str!(
        "../fixtures/workflow_emit/unparseable_when_surfaces_in_emitted_warnings_badwhen.yaml"
    );
    let (_ws, st) = init_run_in_workspace("badwhen", BADWHEN, serde_json::json!({}));
    let e = compile(&st).unwrap();
    assert!(
        e.warnings
            .iter()
            .any(|w| w.contains("second") && w.contains("when")),
        "an unparseable `when:` must be collected into Emitted.warnings, got: {:?}",
        e.warnings
    );
}

const PLANNED_SKILLED_PIPE: &str = include_str!("../fixtures/skills/planned_skilled.yaml");

fn executor_reply_missing_skill_b() -> String {
    format!(
        r#"[ {{ status: "completed", summary: "pushed", outputs: {{ report: "/abs/report.json" }}, follow_ups: ["check CI"], skills_loaded: [{OK_A}] }} ]"#
    )
}

#[test]
fn it_executor_missing_a_skill_keeps_its_reply_and_does_not_pause() {
    let out = eval_safe_agent(
        &executor_reply_missing_skill_b(),
        r#"const r = await safeAgent("P", { label: "push", skills: ["a", "b"], skillsRetry: false });
               console.log(JSON.stringify({ status: r.status, outputs: r.outputs, follow_ups: r.follow_ups,
                 miss: r.signals.skills_not_loaded, downgraded: r.signals.skills_downgraded,
                 summary: r.summary, calls: calls.length }));"#,
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "needs_review", "{out}");
    assert_eq!(v["outputs"]["report"], "/abs/report.json", "{out}");
    assert_eq!(v["follow_ups"], serde_json::json!(["check CI"]), "{out}");
    assert_eq!(v["miss"], serde_json::json!(["b"]), "{out}");
    assert_eq!(v["downgraded"], true, "{out}");
    assert_eq!(v["calls"], 1, "{out}");
    assert!(
        v["summary"]
            .as_str()
            .unwrap()
            .ends_with("[skills not loaded: b]"),
        "{out}"
    );

    let pause = eval_pause_helpers(
        r#"const downgraded = { status: "needs_review", signals: { skills_not_loaded: ["b"], skills_downgraded: true } };
               const genuine = { status: "needs_review", signals: {} };
               console.log(JSON.stringify([isPause(downgraded, false), isPause(genuine, false)]));"#,
    );
    assert_eq!(pause, "[false,true]");
}

#[test]
fn it_executor_needs_review_reply_missing_a_skill_is_flagged_but_not_marked_downgraded() {
    let out = eval_safe_agent(
        r#"[ { status: "needs_review", summary: "unsure", skills_loaded: [] } ]"#,
        r#"const r = await safeAgent("P", { label: "push", skills: ["a"], skillsRetry: false });
               console.log(JSON.stringify([r.status, r.signals.skills_not_loaded, "skills_downgraded" in r.signals]));"#,
    );
    assert_eq!(out, r#"["needs_review",["a"],false]"#);
}

#[test]
fn it_already_failed_reply_is_returned_unchanged_for_an_executor_with_skills() {
    let out = eval_safe_agent(
        r#"[ { status: "failed", summary: "build broke" } ]"#,
        r#"const r = await safeAgent("P", { label: "push", skills: ["a"], skillsRetry: false });
               console.log(JSON.stringify([r.status, r.summary, calls.length]));"#,
    );
    assert_eq!(out, r#"["failed","build broke",1]"#);
}

#[test]
fn it_read_only_role_missing_a_skill_still_fails_after_one_retry() {
    let out = eval_safe_agent(
        r#"[ { status: "completed", summary: "s1", skills_loaded: [] },
             { status: "completed", summary: "s2", skills_loaded: [] } ]"#,
        r#"const r = await safeAgent("P", { label: "study", skills: ["a"] });
               console.log(JSON.stringify([r.status, r.signals.skills_not_loaded, calls.length]));"#,
    );
    assert_eq!(out, r#"["failed",["a"],2]"#);
}

#[test]
fn it_downgraded_summary_never_exceeds_the_envelope_limit() {
    let out = eval_safe_agent(
        r#"[ { status: "completed", summary: "x".repeat(1200), skills_loaded: [] } ]"#,
        r#"const r = await safeAgent("P", { label: "push", skills: ["a"], skillsRetry: false });
               console.log(JSON.stringify([r.status, r.summary.length <= 1200]));"#,
    );
    assert_eq!(out, r#"["needs_review",true]"#);
}

#[test]
fn it_plan_task_missing_a_skill_does_not_prevent_the_merge_dispatch() {
    let (_ws, st) = init_run_in_workspace(
        "plan-skill-downgrade",
        PLANNED_SKILLED_PIPE,
        serde_json::json!({}),
    );
    write_plan(
        &st,
        &format!(
            r#"{{"schema":"aidp-plan.v1","summary":"s","shared_files":[],"tasks":[{},{}]}}"#,
            plan_task("T1", "code", &[], &["src/A.cs"]),
            plan_task("T2", "code", &[], &["src/B.cs"]),
        ),
    );
    let script = compile(&st)
        .unwrap()
        .script
        .replace("export const meta", "const meta");
    let harness = format!(
        r#"const calls = [];
const log = () => {{}};
const phase = () => {{}};
const args = {{}};
const parallel = async (thunks) => Promise.all(thunks.map(thunk => thunk()));
async function agent(promptText, agentOpts) {{
  calls.push(promptText);
  const skills = [{{ name: "a", applied_rule: "r" }}];
  if (!promptText.includes("Plan task `T1`")) skills.push({{ name: "b", applied_rule: "r" }});
  return {{ status: "completed", summary: "done", skills_loaded: skills, outputs: {{}}, signals: {{ has_blocking_findings: false, unevaluated_checks: 0 }} }};
}}
(async () => {{
{script}
}})().then(result => console.log(JSON.stringify({{
  merged: calls.some(text => text.includes("Plan merge")),
  status: result && result.status,
  t1: result && result.results && result.results["implement#T1"] && result.results["implement#T1"].status
}})));
"#
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plan_skills.js");
    std::fs::write(&file, harness).unwrap();
    let out = std::process::Command::new("node")
        .arg(&file)
        .output()
        .expect("node must be installed to run this test");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["merged"], true, "{text}");
    assert_eq!(v["status"], "done", "{text}");
}

#[path = "js_syntax_tests.rs"]
mod js_syntax_tests;

/// Drives `render_seq_stage` directly through an in-memory `EmitCtx`, without `run init` or disk.
mod render_seq_stage_in_memory {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const PIPELINE_YAML: &str = include_str!("../fixtures/workflow_emit/ctx.yaml");
    const STAGE_NAME: &str = "build";
    const INDENT: &str = "  ";
    const SCRATCH_PREFIX: &str = "aidp-emit";
    const EXPECTED_LINE_COUNT: usize = 5;
    const RUN_ID: &str = "ctx-run";
    const PIPELINE_PATH: &str = "/ctx.yaml";
    const PIPELINE_ID: &str = "ctx-test";
    const WORKSPACE_ROOT: &str = "/ws";
    const RUN_DIR: &str = "/ws/.aidp/runs/ctx-run";
    const STARTED_AT: &str = "2024-01-01T00:00:00Z";

    /// Builds a `RunState` that never touches the filesystem.
    fn in_memory_state() -> RunState {
        RunState {
            run_id: RUN_ID.to_string(),
            pipeline_path: PathBuf::from(PIPELINE_PATH),
            pipeline_id: PIPELINE_ID.to_string(),
            workspace_root: PathBuf::from(WORKSPACE_ROOT),
            run_dir: PathBuf::from(RUN_DIR),
            started_at: STARTED_AT.to_string(),
            ..Default::default()
        }
    }

    /// Lists the scratch entries left in the system temp directory by earlier emits.
    fn scratch_entries() -> BTreeSet<String> {
        std::fs::read_dir(tempfile::env::temp_dir())
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with(SCRATCH_PREFIX))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Proves one executor stage renders exactly five statements and leaves no scratch on disk.
    #[test]
    fn render_seq_stage_emits_five_lines() {
        let before = scratch_entries();
        let pipeline: Pipeline =
            serde_yaml::from_str(PIPELINE_YAML).expect("fixture pipeline parses");
        let state = in_memory_state();
        let ctx = EmitCtx::new(&pipeline, &state, &DiskPlanSource).expect("context builds");
        let mut out = EmitOut::default();
        let plan = crate::exec_plan::compile_exec_plan(&pipeline, &state, &DiskPlanSource)
            .expect("plan compiles");
        let Some(crate::exec_plan::Step::Stage(spec)) = plan.steps.first() else {
            panic!("expected a single stage step");
        };

        render_seq_stage(&ctx, spec, INDENT, &mut out).expect("stage renders");

        let lines: Vec<&str> = out
            .body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(lines.len(), EXPECTED_LINE_COUNT, "{lines:#?}");
        assert_eq!(lines[0], format!("phase(\"{STAGE_NAME}\");"));
        assert_eq!(lines[1], format!("const __b_{STAGE_NAME} = spent();"));
        assert!(lines[2].starts_with(&format!("results[\"{STAGE_NAME}\"] = await ")));
        assert_eq!(
            lines[3],
            format!("withTokens(results[\"{STAGE_NAME}\"], spent() - __b_{STAGE_NAME});")
        );
        assert_eq!(
            lines[4],
            format!(
                "if (isPause(results[\"{STAGE_NAME}\"], false)) return pauseInfo(\"{STAGE_NAME}\", results[\"{STAGE_NAME}\"]);"
            )
        );
        assert_eq!(scratch_entries(), before);
    }
}

fn emit_graphify_pipe(tag: &str, with_graph: bool) -> (String, Vec<String>) {
    let (_ws, mut st) = init_run_in_workspace(
        tag,
        GRAPHIFY_PIPE,
        serde_json::Value::Object(Default::default()),
    );
    st.knowledge_graph = with_graph.then(|| {
        let mut graph = crate::kg::KnowledgeGraphState::skipped(SKIPPED_GRAPH_REASON);
        graph.graph_json = Some(GRAPH_JSON_PATH.to_string());
        graph
    });
    let emitted = compile(&st).unwrap();
    (emitted.script, emitted.warnings)
}

/// No graph: the skill is absent from prompt and options, a trace is in the warnings.
#[test]
fn graphify_skill_is_dropped_without_a_knowledge_graph() {
    let (js, warnings) = emit_graphify_pipe(GRAPHIFY_OFF_TAG, false);
    assert!(!js.contains(GRAPHIFY_PROMPT_CALL), "{js}");
    assert!(!js.contains(GRAPHIFY_OPTION_ENTRY), "{js}");
    assert!(js.contains(RUST_SKILL_ENTRY), "{js}");
    assert!(
        warnings.iter().any(|w| w.contains(PLAIN_STAGE)),
        "{warnings:?}"
    );
}

/// With a graph the skill stays in the prompt and in the options.
#[test]
fn graphify_skill_is_kept_with_a_knowledge_graph() {
    let (js, warnings) = emit_graphify_pipe(GRAPHIFY_ON_TAG, true);
    assert!(js.contains(GRAPHIFY_PROMPT_CALL), "{js}");
    assert!(js.contains(GRAPHIFY_OPTION_ENTRY), "{js}");
    assert!(warnings.is_empty(), "{warnings:?}");
}
