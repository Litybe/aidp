use super::*;

use std::path::PathBuf;

fn rules(mode: &str) -> ScopeRules {
    ScopeRules {
        mode: mode.into(),
        stage_dir: PathBuf::from("/run/stageX"),
        workspace_root: PathBuf::from("/repo"),
        allowed: vec![],
        denied: vec![],
    }
}

#[test]
fn production_excludes_tests_cross_platform() {
    let r = rules("production");
    assert!(r.check(Path::new("/repo/tests/Foo.cs")).is_err());
    let rw = ScopeRules {
        workspace_root: PathBuf::from(r"C:\repo"),
        ..rules("production")
    };
    assert!(rw.check(Path::new(r"C:\repo\tests\Foo.cs")).is_err());
    assert!(r.check(Path::new("/repo/docs/readme.md")).is_err());
    assert!(r.check(Path::new("/repo/src/Service.cs")).is_ok());
    assert!(r.check(Path::new("/repo/tests_backup/config.json")).is_ok());
}

#[test]
fn tests_mode_matches_dir_or_test_filename_only() {
    let r = rules("tests");
    assert!(r.check(Path::new("/repo/tests/FooTests.cs")).is_ok());
    let rw = ScopeRules {
        workspace_root: PathBuf::from(r"C:\repo"),
        ..rules("tests")
    };
    assert!(rw.check(Path::new(r"C:\repo\tests\Foo.cs")).is_ok());
    assert!(r.check(Path::new("/repo/src/FooTests.cs")).is_ok());
    assert!(r.check(Path::new("/repo/src/FooSpec.ts")).is_ok());
    assert!(r.check(Path::new("/repo/src/latest.cs")).is_err());
}

fn tmp_rules(mode: &str) -> (tempfile::TempDir, ScopeRules) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("repo");
    let sd = ws.join("run").join("stage");
    std::fs::create_dir_all(&sd).unwrap();
    let r = ScopeRules {
        mode: mode.into(),
        stage_dir: sd,
        workspace_root: ws,
        allowed: vec![],
        denied: vec![],
    };
    (tmp, r)
}

#[test]
fn stage_dir_only_refuses_nonexistent_escape() {
    let (_t, r) = tmp_rules("stage_dir_only");
    assert!(r
        .check(&r.stage_dir.join("../../../../../../evil.json"))
        .is_err());
    assert!(r.check(&r.stage_dir.join("../sibling.json")).is_err());
}

#[test]
fn stage_dir_only_accepts_nonexistent_inside() {
    let (_t, r) = tmp_rules("stage_dir_only");
    assert!(r.check(&r.stage_dir.join("new.json")).is_ok());
    assert!(r.check(&r.stage_dir.join("a/../b.json")).is_ok());
    assert!(r.check(&r.stage_dir.join("deep/er/c.json")).is_ok());
}

#[test]
fn normalize_lexical_folds_and_refuses_climb() {
    assert_eq!(
        normalize_lexical(Path::new("/a/./b/../c")),
        Some(PathBuf::from("/a/c"))
    );
    assert_eq!(normalize_lexical(Path::new("/a/../..")), None);
    assert_eq!(normalize_lexical(Path::new("../x")), None);
}

#[test]
fn production_and_tests_are_anchored_to_workspace() {
    for mode in ["production", "tests"] {
        let (t, r) = tmp_rules(mode);
        let outside = t.path().join("elsewhere/tests/FooTests.cs");
        assert!(r.check(&outside).is_err(), "{mode}: outside accepted");
        let escape = r.workspace_root.join("../x/tests/FooTests.cs");
        assert!(r.check(&escape).is_err(), "{mode}: escape accepted");
    }
    let (_t, r) = tmp_rules("production");
    assert!(r.check(&r.workspace_root.join("src/a.rs")).is_ok());
    assert!(r.check(Path::new("src/a.rs")).is_ok());
    let (_t, r) = tmp_rules("tests");
    assert!(r.check(&r.workspace_root.join("tests/a_test.rs")).is_ok());
}

#[test]
fn invalid_globs_error_naming_the_pattern() {
    let m = compile_globs("denied", &["[unclosed".to_string()])
        .unwrap_err()
        .to_string();
    assert!(m.contains("scope.denied") && m.contains("[unclosed"), "{m}");
    let m = compile_globs("allowed", &["ok/**".into(), "a[".into()])
        .unwrap_err()
        .to_string();
    assert!(m.contains("scope.allowed") && m.contains("a["), "{m}");
    assert_eq!(
        compile_globs("allowed", &["src/**".into()]).unwrap().len(),
        1
    );
}

#[test]
fn classify_reports_paths_no_rule_accepts_with_the_closest_reason() {
    let rules_list = vec![
        ("build".to_string(), rules("production")),
        ("tests_stage".to_string(), rules("tests")),
    ];
    let paths = vec![
        "src/service/handler.cs".to_string(),
        "tests/HandlerTests.cs".to_string(),
        "docs/architecture.md".to_string(),
    ];
    let violations = classify(&paths, &rules_list);
    assert_eq!(violations.len(), 1, "got: {violations:?}");
    assert_eq!(violations[0].path, "docs/architecture.md");
    assert!(
        violations[0].reason.contains("not in tests"),
        "got: {}",
        violations[0].reason
    );
}

#[test]
fn classify_with_no_rules_flags_every_path_honestly() {
    let violations = classify(&["src/a.rs".to_string()], &[]);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].reason.contains("no scope rules"));
}

fn run_git(dir: &Path, args: &[&str]) {
    let st = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git must be on PATH for this test");
    assert!(st.success(), "git {args:?} failed in {}", dir.display());
}

fn init_git_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "aidp tests"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "chore: initial commit"]);
}

fn write_pipeline(dir: &Path, yaml: &str) -> PathBuf {
    let p = dir.join("pipe.yaml");
    std::fs::write(&p, yaml).unwrap();
    p
}

fn base_run_state(
    run_id: &str,
    workspace_root: PathBuf,
    pipeline_path: PathBuf,
    base_sha: Option<String>,
    stage_name: &str,
) -> RunState {
    crate::run::RunState {
        run_id: run_id.into(),
        pipeline_path,
        pipeline_id: "test".into(),
        run_dir: workspace_root.join(".run"),
        workspace_root,
        started_at: "t".into(),
        base_sha,
        stages: vec![crate::run::StageState {
            name: stage_name.into(),
            status: StageStatus::Success,
            stage_dir: PathBuf::from("/unused"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn it_a_repo_wide_stage_makes_the_union_universal_and_says_so() {
    let dir = std::env::temp_dir().join(format!("aidp-scope-repowide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init_git_repo(&dir);
    let base = crate::gitlog::head(&dir).expect("HEAD must resolve");
    let pipeline_path = write_pipeline(
        &dir,
        "version: 5\nid: repo-wide-test\nstages:\n  - { name: code, agent: executor, scope: { mode: repo_wide } }\n",
    );
    let state = base_run_state("r-repowide", dir.clone(), pipeline_path, Some(base), "code");

    let report = verify_run_report(&state).expect("verify_run_report must succeed");
    assert!(
        report.union_universal,
        "repo_wide stage must set union_universal"
    );
    assert!(
        report.uninspected_stages.is_empty(),
        "a non-worktree stage must not be reported as uninspected: {:?}",
        report.uninspected_stages
    );
    assert!(
        report.violations.is_empty(),
        "nothing changed after init: {:?}",
        report.violations
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_a_worktree_isolated_stage_with_no_surviving_tree_is_named_uninspected() {
    let dir = std::env::temp_dir().join(format!("aidp-scope-worktree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init_git_repo(&dir);
    let base = crate::gitlog::head(&dir).expect("HEAD must resolve");
    let dev_angular = aidp_testkit::fixtures::template("dev-angular");
    assert!(
        dev_angular.is_file(),
        "templates/dev-angular.yaml must exist at {}",
        dev_angular.display()
    );
    let state = base_run_state(
        "r-worktree",
        dir.clone(),
        dev_angular,
        Some(base),
        "implement",
    );

    let report = verify_run_report(&state).expect("verify_run_report must succeed");
    assert_eq!(
        report.uninspected_stages,
        vec!["implement".to_string()],
        "an isolation:worktree stage with no surviving tree must be named, not silently skipped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_verify_run_is_a_pure_projection_of_verify_run_report_violations() {
    let dir = std::env::temp_dir().join(format!("aidp-scope-projection-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init_git_repo(&dir);
    let base = crate::gitlog::head(&dir).expect("HEAD must resolve");
    let pipeline_path = write_pipeline(
        &dir,
        "version: 5\nid: projection-test\nstages:\n  - { name: code, agent: executor, scope: { mode: tests } }\n",
    );
    std::fs::write(dir.join("src_violation.rs"), "fn x() {}").unwrap();
    let state = base_run_state("r-proj", dir.clone(), pipeline_path, Some(base), "code");

    let via_run = verify_run(&state).expect("verify_run must succeed");
    let via_report = verify_run_report(&state).expect("verify_run_report must succeed");
    assert_eq!(via_run.len(), via_report.violations.len());
    assert!(
        via_run.iter().any(|v| v.path.ends_with("src_violation.rs")),
        "got: {via_run:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_scope_json_returns_the_five_keys_and_creates_no_file() {
    let (_t, _r) = tmp_rules("stage_dir_only");
    let dir = std::env::temp_dir().join(format!("aidp-scope-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init_git_repo(&dir);
    let pipeline_path = write_pipeline(
        &dir,
        "version: 5\nid: scope-json-test\nstages:\n  - { name: code, agent: executor, scope: { mode: repo_wide } }\n",
    );
    let state = base_run_state("r-scope-json", dir.clone(), pipeline_path, None, "code");

    let v = scope_json(&state, "code").expect("scope_json must succeed");
    assert_eq!(v.stage, "code");
    assert_eq!(v.mode, "repo_wide");

    let scope_dir = state.stage_dir("code", crate::pipeline::Placement::Run);
    assert!(
        !scope_dir.join("scope.json").exists(),
        "scope_json must not write scope.json"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_write_scope_json_writes_the_file() {
    let dir = std::env::temp_dir().join(format!("aidp-write-scope-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    init_git_repo(&dir);
    let pipeline_path = write_pipeline(
        &dir,
        "version: 5\nid: write-scope-json-test\nstages:\n  - { name: code, agent: executor, scope: { mode: repo_wide } }\n",
    );
    let state = base_run_state(
        "r-write-scope-json",
        dir.clone(),
        pipeline_path,
        None,
        "code",
    );

    let v = scope_json(&state, "code").expect("scope_json must succeed");
    let path = write_scope_json(&state, "code", &v).expect("write_scope_json must succeed");
    assert!(path.exists(), "scope.json must exist at {}", path.display());
    let written: crate::models::scope::ResolvedScope =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written, v);

    let _ = std::fs::remove_dir_all(&dir);
}
