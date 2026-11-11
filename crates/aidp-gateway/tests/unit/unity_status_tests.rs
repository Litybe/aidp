use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn process(pid: u32, command_line: &str) -> UnityProcess {
    UnityProcess {
        pid,
        command_line: command_line.to_string(),
    }
}

struct CountingSource(Arc<AtomicUsize>);

impl ProcessSource for CountingSource {
    fn unity_processes(&self) -> Vec<UnityProcess> {
        self.0.fetch_add(1, Ordering::SeqCst);
        vec![process(1, "Unity.exe -projectpath D:/x")]
    }
}

#[test]
fn project_path_is_read_quoted_or_bare_whatever_the_case() {
    assert_eq!(
        project_path_of(
            r#""D:\Program Files\Unity.exe" -projectPath "D:\My Game\proj" -batchmode"#
        )
        .as_deref(),
        Some(r"D:\My Game\proj")
    );
    assert_eq!(
        project_path_of("Unity.exe -PROJECTPATH D:/game -logFile x").as_deref(),
        Some("D:/game")
    );
    assert_eq!(project_path_of("Unity.exe -batchmode"), None);
}

#[test]
fn editor_matches_ignoring_case_separators_and_trailing_slash() {
    let processes = vec![
        process(10, r"Unity.exe -projectpath D:\Other\proj"),
        process(
            21156,
            r#"Unity.exe -projectpath "D:\CLAUDE_UNITY\MixAndJam-CelesteMovement\""#,
        ),
    ];

    let found = find_editor(&processes, "d:/claude_unity/mixandjam-celestemovement");

    assert_eq!(found.map(|p| p.pid), Some(21156));
    assert!(find_editor(&processes, "D:/nope").is_none());
}

#[test]
fn parses_one_object_an_array_and_nothing_from_powershell() {
    let one = r#"{"ProcessId":7,"CommandLine":"Unity.exe -projectpath D:/a"}"#;
    let many = r#"[{"ProcessId":7,"CommandLine":"a"},{"ProcessId":8,"CommandLine":null}]"#;

    assert_eq!(
        parse_cim_json(one),
        vec![process(7, "Unity.exe -projectpath D:/a")]
    );
    assert_eq!(parse_cim_json(many), vec![process(7, "a")]);
    assert!(parse_cim_json("").is_empty());
}

#[test]
fn parses_ps_lines_of_unity_only() {
    let text =
        "  42 /opt/unity/Editor/Unity -projectpath /home/me/p\n  43 /usr/bin/bash -c unity\n";

    assert_eq!(
        parse_ps(text),
        vec![process(
            42,
            "/opt/unity/Editor/Unity -projectpath /home/me/p"
        )]
    );
}

#[test]
fn listing_is_cached_until_the_ttl_expires() {
    let calls = Arc::new(AtomicUsize::new(0));
    let cached = CachedProcesses::new(
        Box::new(CountingSource(calls.clone())),
        Duration::from_secs(60),
    );

    cached.list();
    cached.list();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let expired = CachedProcesses::new(Box::new(CountingSource(calls.clone())), Duration::ZERO);
    expired.list();
    expired.list();
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

fn write_result(project: &Path, parts: &[&str; 3], name: &str, body: &str, age_secs: u64) {
    let dir = results_dir(project, parts);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    let time = SystemTime::now() - Duration::from_secs(age_secs);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn last_results_are_the_newest_files_or_null() {
    let dir = tempfile::tempdir().unwrap();
    assert!(last_compile(dir.path()).is_null());
    assert!(last_playtest(dir.path()).is_null());

    write_result(
        dir.path(),
        &COMPILE_RESULTS,
        "old.json",
        r#"{"id":"old","ok":false}"#,
        100,
    );
    write_result(
        dir.path(),
        &COMPILE_RESULTS,
        "new.json",
        r#"{"id":"new","ok":true,"errors":[]}"#,
        5,
    );
    write_result(
        dir.path(),
        &PLAYTEST_RESULTS,
        "p.json",
        r#"{"id":"pt","passed":true,"durationMs":9}"#,
        5,
    );
    write_result(dir.path(), &PLAYTEST_RESULTS, "bad.json", "not json", 1);

    assert_eq!(last_compile(dir.path())["id"], "new");
    assert_eq!(last_compile(dir.path())["ok"], true);
    // An unreadable newest file is skipped.
    assert_eq!(last_playtest(dir.path())["id"], "pt");
}

#[test]
fn last_playtest_keeps_id_and_verdict_and_stamps_the_time() {
    let dir = tempfile::tempdir().unwrap();
    write_result(
        dir.path(),
        &PLAYTEST_RESULTS,
        "p.json",
        r#"{"id":"pt","passed":false,"durationMs":9}"#,
        5,
    );

    let value = last_playtest(dir.path());

    assert_eq!(value["id"], "pt");
    assert_eq!(value["passed"], false);
    assert!(value["finishedAt"].as_str().unwrap().ends_with('Z'));
    assert!(value.get("durationMs").is_none());
}

#[test]
fn status_has_the_c3_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().replace('\\', "/");
    let processes = vec![process(21156, &format!("Unity.exe -projectpath {path}"))];
    let holder = ResourceHolder {
        flow_key: "f".into(),
        ticket_key: "GH-214".into(),
        step_id: "implement".into(),
        since: "2026-11-06T10:00:00.000Z".into(),
    };

    let value = status("p1", &path, &processes, Some(&holder), &["g".to_string()]);

    assert_eq!(value["editor"]["running"], true);
    assert_eq!(value["editor"]["pid"], 21156);
    assert_eq!(value["editor"]["projectPath"], path.as_str());
    assert_eq!(value["lock"]["held"], true);
    assert_eq!(value["lock"]["holder"]["ticketKey"], "GH-214");
    assert_eq!(value["lock"]["waiting"][0], "g");
    assert!(value["lastCompile"].is_null());
    assert!(value["lastPlaytest"].is_null());
    assert!(value["editor"]["heartbeat"].is_null());

    let idle = status("p1", &path, &[], None, &[]);
    assert_eq!(idle["editor"]["running"], false);
    assert!(idle["editor"]["pid"].is_null());
    assert_eq!(idle["lock"]["held"], false);
    assert!(idle["lock"]["holder"].is_null());
}

#[test]
fn heartbeat_is_read_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let editor_dir = dir.path().join(".aidp").join("editor");
    std::fs::create_dir_all(&editor_dir).unwrap();
    std::fs::write(
        editor_dir.join("heartbeat.json"),
        r#"{"at":"2026-11-09T10:00:02.120Z","state":"idle","staleAfterSeconds":15}"#,
    )
    .unwrap();

    let heartbeat = last_heartbeat(dir.path());
    assert_eq!(heartbeat["state"], "idle");
    assert_eq!(heartbeat["staleAfterSeconds"], 15);

    let path = dir.path().to_string_lossy().replace('\\', "/");
    let value = status("p1", &path, &[], None, &[]);
    assert_eq!(
        value["editor"]["heartbeat"]["at"],
        "2026-11-09T10:00:02.120Z"
    );
}
