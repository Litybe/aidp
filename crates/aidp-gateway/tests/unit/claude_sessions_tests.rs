use super::*;
use std::fs;

const PROJECT_A: &str = "C:\\CODE\\alpha";
const PROJECT_B: &str = "C:\\CODE\\beta";

fn write_transcript(home: &Path, project: &str, id: &str, body: &str) {
    let dir = slug_dir(home, project);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{id}.jsonl")), body).unwrap();
}

fn user_line(text: &str) -> String {
    format!("{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n")
}

#[test]
fn slug_replaces_non_alphanumerics() {
    assert_eq!(slug_for("C:\\CODE\\AIDP\\TODO"), "C--CODE-AIDP-TODO");
    assert_eq!(
        slug_for("C:\\Users\\a.vuk-ext\\.claude"),
        "C--Users-a-vuk-ext--claude"
    );
}

#[test]
fn list_returns_only_requested_project() {
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), PROJECT_A, "aaa-1", &user_line("hello alpha"));
    write_transcript(home.path(), PROJECT_B, "bbb-1", &user_line("hello beta"));
    let sessions = list(home.path(), PROJECT_A);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "aaa-1");
    assert_eq!(sessions[0].first_prompt, "hello alpha");
}

#[test]
fn first_prompt_skips_bad_lines_and_truncates() {
    let home = tempfile::tempdir().unwrap();
    let long = "x".repeat(100);
    let body = format!(
        "not json\n{{\"type\":\"assistant\"}}\n{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{long}\"}}]}}}}\n"
    );
    write_transcript(home.path(), PROJECT_A, "s1", &body);
    assert_eq!(
        list(home.path(), PROJECT_A)[0].first_prompt.chars().count(),
        60
    );
}

#[test]
fn list_of_missing_directory_is_empty() {
    let home = tempfile::tempdir().unwrap();
    assert!(list(home.path(), PROJECT_A).is_empty());
}

#[test]
fn find_unknown_id_is_not_found() {
    let home = tempfile::tempdir().unwrap();
    write_transcript(home.path(), PROJECT_A, "aaa-1", &user_line("hi"));
    assert!(matches!(
        find(home.path(), PROJECT_A, "zzz"),
        Err(SessionError::NotFound(_))
    ));
    assert!(find(home.path(), PROJECT_A, "aaa-1").is_ok());
    assert!(matches!(
        find(home.path(), PROJECT_B, "aaa-1"),
        Err(SessionError::NotFound(_))
    ));
}

#[test]
fn find_rejects_traversal_id() {
    let home = tempfile::tempdir().unwrap();
    let too_long = "a".repeat(65);
    for id in ["../x", "a/b", "a\\b", "", too_long.as_str()] {
        assert!(
            matches!(
                find(home.path(), PROJECT_A, id),
                Err(SessionError::NotFound(_))
            ),
            "{id}"
        );
    }
}
