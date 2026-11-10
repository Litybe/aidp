use super::*;

const TRANSCRIPT: &str = include_str!("../fixtures/sessions/transcript.jsonl");
const PROJECT: &str = "pixelhood";
const SESSION: &str = "abcdef1234567890";

fn journal() -> SessionJournal {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, TRANSCRIPT).unwrap();
    read_transcript(&path).unwrap()
}

#[test]
fn a_transcript_gives_prompts_files_commits_and_final_reply() {
    let journal = journal();
    assert_eq!(journal.session_id, SESSION);
    assert_eq!(
        journal.prompts,
        ["Corrige le dash du joueur", "Oui, committe"]
    );
    assert_eq!(journal.prompt_count, 2);
    assert_eq!(journal.files, ["D:/game/Assets/Player.cs"]);
    assert_eq!(journal.commits, ["fix(player): dash conserve la vitesse"]);
    assert_eq!(journal.last_reply.as_deref(), Some("Commit fait."));
    assert_eq!(journal.branch.as_deref(), Some("fix/dash"));
    assert_eq!(journal.started.as_deref(), Some("2026-11-08T10:15:14.000Z"));
    assert_eq!(journal.ended.as_deref(), Some("2026-11-08T10:16:10.000Z"));
}

#[test]
fn the_note_path_is_stable_and_dated() {
    assert_eq!(
        journal().relative_path(),
        Path::new("2026-11").join("2026-11-08-abcdef12.md")
    );
}

#[test]
fn the_note_lists_the_sections() {
    let note = journal().render(PROJECT);
    assert!(note.starts_with(&format!("---\nsession: {SESSION}\nproject: {PROJECT}\n")));
    assert!(note.contains("# Session Corrige le dash du joueur\n"));
    assert!(note.contains("## Commits\n\n- fix(player): dash conserve la vitesse\n"));
}

#[test]
fn a_heredoc_and_a_here_string_give_the_subject() {
    let heredoc = "git commit -q -F - <<'EOF'\nfeat: sujet\n\ncorps\nEOF";
    assert_eq!(commit_subject(heredoc).as_deref(), Some("feat: sujet"));
    let here_string = "git commit -m @'\nfix: autre\n'@";
    assert_eq!(commit_subject(here_string).as_deref(), Some("fix: autre"));
    assert_eq!(commit_subject("git status"), None);
}

#[test]
fn a_pipeline_session_and_a_single_question_are_not_kept() {
    let mut pipeline = journal();
    pipeline.prompts[0] = format!("{PIPELINE_PROMPT_PREFIX}test`");
    assert!(!pipeline.worth_keeping());
    let question = SessionJournal {
        started: Some("2026-11-08T10:15:14.000Z".to_string()),
        prompts: vec!["Une question".to_string()],
        prompt_count: 1,
        ..Default::default()
    };
    assert!(!question.worth_keeping());
    assert!(journal().worth_keeping());
}
