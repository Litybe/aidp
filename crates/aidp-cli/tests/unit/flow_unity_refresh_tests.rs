use super::*;
use aidp_gateway::unity_status::UnityProcess;
use serde_json::json;
use std::time::Duration;

const PROJECT_VERSION_FILE: &str = "ProjectSettings/ProjectVersion.txt";
const HEARTBEAT_LIVE: &str =
    r#"{"at":"2026-11-09T10:00:00Z","state":"idle","staleAfterSeconds":15}"#;

struct Editors(Vec<UnityProcess>);

impl ProcessSource for Editors {
    fn unity_processes(&self) -> Vec<UnityProcess> {
        self.0.clone()
    }
}

fn editor_on(path: &Path) -> Editors {
    Editors(vec![UnityProcess {
        pid: 7,
        command_line: format!("Unity.exe -projectpath {}", path.to_string_lossy()),
    }])
}

fn unity_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let version = dir.path().join(PROJECT_VERSION_FILE);
    std::fs::create_dir_all(version.parent().unwrap()).unwrap();
    std::fs::write(version, "m_EditorVersion: 6000").unwrap();
    dir
}

fn environment<'a>(processes: &'a dyn ProcessSource, timeout_millis: u64) -> Environment<'a> {
    Environment {
        processes,
        timeout: Duration::from_millis(timeout_millis),
        poll: Duration::from_millis(10),
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_plain_repository_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let editors = editor_on(dir.path());
    let report = run(dir.path(), &environment(&editors, 100));
    assert!(report.skipped);
    assert_eq!(report.reason.as_deref(), Some(REASON_NOT_UNITY_PROJECT));
    assert!(!std::fs::exists(dir.path().join(".aidp")).unwrap());
}

#[test]
fn a_closed_editor_is_skipped_without_a_request() {
    let dir = unity_project();
    let none = Editors(Vec::new());
    let report = run(dir.path(), &environment(&none, 100));
    assert!(report.skipped);
    assert_eq!(report.reason.as_deref(), Some(REASON_EDITOR_CLOSED));
    assert!(!std::fs::exists(dir.path().join(".aidp")).unwrap());
}

/// The verb drops `{}` in the requests folder and reads the result the bridge writes for that id.
#[test]
fn the_compile_result_of_the_request_is_reported() {
    let dir = unity_project();
    let editors = editor_on(dir.path());
    let workspace = dir.path().to_path_buf();
    let bridge = std::thread::spawn(move || {
        let requests = dir_of(&workspace, &COMPILE_REQUESTS_DIR);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = first_json(&requests) {
                let id = request.file_stem().unwrap().to_string_lossy().to_string();
                assert_eq!(
                    std::fs::read_to_string(&request).unwrap(),
                    EMPTY_REQUEST_BODY
                );
                write(
                    &result_file(&workspace, &id),
                    &json!({ "id": id, "ok": false, "errors": [{ "file": "Assets/A.cs", "line": 3, "message": "CS0103" }], "warnings": 2, "durationMs": 840 }).to_string(),
                );
                return id;
            }
            assert!(Instant::now() < deadline, "no request written");
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    let report = run(dir.path(), &environment(&editors, 5000));
    let id = bridge.join().unwrap();

    assert_eq!(report.request_id.as_deref(), Some(id.as_str()));
    assert!(id.starts_with(REQUEST_ID_PREFIX));
    assert!(!report.ok);
    assert!(!report.blocked);
    assert!(!report.skipped);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.warnings, 2);
    assert_eq!(report.duration_ms, 840);
    assert_eq!(report.message, "compile gate failed: 1 error(s)");
}

#[test]
fn a_silent_editor_is_blocked_after_the_timeout_with_its_heartbeat() {
    let dir = unity_project();
    let editors = editor_on(dir.path());
    let live = chrono::Utc::now().to_rfc3339();
    write(
        &heartbeat_file(dir.path()),
        &json!({ "at": live, "state": "idle" }).to_string(),
    );

    let report = run(dir.path(), &environment(&editors, 50));

    assert!(report.blocked);
    assert!(!report.skipped);
    assert!(report.message.starts_with(BLOCKED_UNITY_EDITOR));
    assert!(report.message.contains("no compile result"));
    assert_eq!(report.heartbeat["state"], "idle");
}

#[test]
fn a_stale_heartbeat_blocks_before_the_timeout() {
    let dir = unity_project();
    let editors = editor_on(dir.path());
    write(&heartbeat_file(dir.path()), HEARTBEAT_LIVE);

    let started = Instant::now();
    let report = run(dir.path(), &environment(&editors, 60_000));

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(report.blocked);
    assert!(report.message.contains("heartbeat"));
    assert_eq!(report.heartbeat["at"], "2026-11-09T10:00:00Z");
}

#[test]
fn heartbeat_staleness_depends_on_the_state() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-09T10:01:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let fresh = json!({ "at": "2026-11-09T10:00:50Z", "state": "idle" });
    let stale = json!({ "at": "2026-11-09T10:00:00Z", "state": "compiling" });
    let reloading = json!({ "at": "2026-11-09T10:00:00Z", "state": "reloading" });
    let long_reload = json!({ "at": "2026-11-09T09:50:00Z", "state": "reloading" });

    assert_eq!(stale_heartbeat_age(&fresh, now), None);
    assert_eq!(stale_heartbeat_age(&stale, now), Some(60));
    assert_eq!(stale_heartbeat_age(&reloading, now), None);
    assert_eq!(stale_heartbeat_age(&long_reload, now), Some(660));
    assert_eq!(stale_heartbeat_age(&Value::Null, now), None);
    assert_eq!(stale_heartbeat_age(&json!({ "at": "garbage" }), now), None);
}

#[test]
fn a_result_without_errors_is_ok() {
    let report = from_result(
        "refresh-1".to_string(),
        &json!({ "ok": true, "errors": [], "warnings": 0, "durationMs": 12 }),
    );
    assert!(report.ok);
    assert_eq!(report.message, "compile gate ok");
    let text = serde_json::to_value(&report).unwrap();
    assert_eq!(text["request_id"], "refresh-1");
    assert_eq!(text["blocked"], false);
    assert!(text.get("reason").is_none());
}

fn dir_of(workspace: &Path, parts: &[&str; 3]) -> PathBuf {
    parts
        .iter()
        .fold(workspace.to_path_buf(), |path, part| path.join(part))
}

fn first_json(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == JSON_EXTENSION))
}
