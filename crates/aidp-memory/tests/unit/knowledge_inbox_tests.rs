use super::*;
use crate::models::enums::ProposalOutcome;

const REPO: &str = "Mini";
const TODAY: &str = "2026-11-11";
const OLD: &str = "---\ntype: note\ndomain: unity\ntags: []\nagents: []\nstatus: active\nimportance: 0.5\nupdated: 2026-01-01\nsource: skill:x\n---\n# Ancienne\n\nVieux texte.\n\n## Evidence\n- 2026-01-01 skill:x\n";

fn inbox_note(extra: &str) -> String {
    format!("---\ntype: note\ndomain: unity\ntags: [a]\nagents: []\nstatus: proposed\nimportance: 0.6\nupdated: 2026-02-02\nsource: run:r-1\n{extra}---\n# Neuve\n\nTexte propose.\n\n## Evidence\n- 2026-02-02 run:r-1\n")
}

fn vault_with(files: &[(&str, String)]) -> tempfile::TempDir {
    let vault = tempfile::tempdir().unwrap();
    for (rel, content) in files {
        let path = vault.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    vault
}

fn inbox(vault: &tempfile::TempDir) -> KnowledgeInbox {
    KnowledgeInbox::new(vault.path(), REPO).with_today(TODAY)
}

fn read(vault: &tempfile::TempDir, rel: &str) -> String {
    std::fs::read_to_string(vault.path().join(rel)).unwrap()
}

#[test]
fn the_list_shows_the_pending_notes_only() {
    let vault = vault_with(&[
        (
            "Knowledge/_inbox/b.md",
            inbox_note(
                "action: add\nscope: global\ncandidate_of: \"[[Knowledge/_global/unity/x]]\"\n",
            ),
        ),
        ("Knowledge/_inbox/a.md", inbox_note("")),
        ("Knowledge/_inbox/_rejected/old.md", inbox_note("")),
        ("Knowledge/_inbox/readme.txt", "x".into()),
    ]);
    let entries = inbox(&vault).list().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path, "Knowledge/_inbox/a.md");
    assert_eq!(entries[0].action, "add");
    assert_eq!(entries[1].title, "Neuve");
    assert_eq!(entries[1].domain, "unity");
    assert_eq!(
        entries[1].candidate_of.as_deref(),
        Some("Knowledge/_global/unity/x.md")
    );
    assert!(inbox(&tempfile::tempdir().unwrap())
        .list()
        .unwrap()
        .is_empty());
}

#[test]
fn approving_an_add_moves_the_note_active_without_inbox_fields() {
    let vault = vault_with(&[(
        "Knowledge/_inbox/rule.md",
        inbox_note("action: add\nscope: project\nrepo: Other\ncandidate_of: \"[[Knowledge/x]]\"\n"),
    )]);
    let decision = inbox(&vault).approve("Knowledge/_inbox/rule.md").unwrap();
    assert_eq!(decision.outcome, Some(ProposalOutcome::Added));
    assert_eq!(decision.path, "Knowledge/Mini/notes/unity/rule.md");
    let moved = read(&vault, &decision.path);
    assert!(moved.contains("status: active\n"));
    assert!(moved.contains("updated: 2026-11-11\n"));
    for gone in ["action:", "scope:", "repo:", "candidate_of:"] {
        assert!(!moved.contains(gone), "{gone}");
    }
    assert!(!vault.path().join("Knowledge/_inbox/rule.md").exists());
}

#[test]
fn approving_uses_the_given_repo_when_the_note_names_none() {
    let vault = vault_with(&[("Knowledge/_inbox/rule.md", inbox_note(""))]);
    let decision = inbox(&vault).approve("rule").unwrap();
    assert_eq!(decision.path, "Knowledge/Mini/notes/unity/rule.md");
}

#[test]
fn approving_a_global_note_goes_to_the_global_folder() {
    let vault = vault_with(&[("Knowledge/_inbox/rule.md", inbox_note("scope: global\n"))]);
    let decision = inbox(&vault).approve("rule.md").unwrap();
    assert_eq!(decision.path, "Knowledge/_global/unity/rule.md");
}

#[test]
fn approving_never_overwrites_a_note() {
    let vault = vault_with(&[
        ("Knowledge/_inbox/rule.md", inbox_note("scope: global\n")),
        ("Knowledge/_global/unity/rule.md", OLD.into()),
    ]);
    let error = inbox(&vault).approve("rule").unwrap_err();
    assert!(matches!(error, MemoryError::Conflict(_)));
    assert_eq!(read(&vault, "Knowledge/_global/unity/rule.md"), OLD);
    assert!(vault.path().join("Knowledge/_inbox/rule.md").exists());
}

#[test]
fn approving_an_update_rewrites_its_target_and_archives_the_inbox_note() {
    let vault = vault_with(&[
        (
            "Knowledge/_inbox/rule.md",
            inbox_note(
                "action: update\nscope: global\ntarget: \"[[Knowledge/_global/unity/old]]\"\n",
            ),
        ),
        ("Knowledge/_global/unity/old.md", OLD.into()),
    ]);
    let decision = inbox(&vault).approve("rule").unwrap();
    assert_eq!(decision.outcome, Some(ProposalOutcome::Updated));
    assert_eq!(decision.path, "Knowledge/_global/unity/old.md");
    let rewritten = read(&vault, "Knowledge/_global/unity/old.md");
    assert!(rewritten.contains("# Neuve\n\nTexte propose.\n"));
    assert!(rewritten.contains("- 2026-01-01 skill:x\n- 2026-11-11 run:r-1 (approved)\n"));
    assert_eq!(
        read(
            &vault,
            "Knowledge/_history/_global__unity__old__2026-01-01.md"
        ),
        OLD
    );
    assert!(vault
        .path()
        .join("Knowledge/_inbox/_applied/rule.md")
        .exists());
    assert!(!vault.path().join("Knowledge/_inbox/rule.md").exists());
}

#[test]
fn approving_a_supersede_marks_the_target() {
    let vault = vault_with(&[
        (
            "Knowledge/_inbox/rule.md",
            inbox_note(
                "action: supersede\nscope: global\ntarget: \"[[Knowledge/_global/unity/old]]\"\n",
            ),
        ),
        ("Knowledge/_global/unity/old.md", OLD.into()),
    ]);
    let decision = inbox(&vault).approve("rule").unwrap();
    assert_eq!(decision.outcome, Some(ProposalOutcome::Superseded));
    let old = read(&vault, "Knowledge/_global/unity/old.md");
    assert!(old.contains("status: superseded\n"));
    assert!(old.contains("superseded_by: \"[[Knowledge/_global/unity/rule]]\"\n"));
    assert!(read(&vault, "Knowledge/_global/unity/rule.md").contains("status: active\n"));
}

#[test]
fn rejecting_moves_the_note_to_the_rejected_folder() {
    let vault = vault_with(&[
        ("Knowledge/_inbox/rule.md", inbox_note("")),
        ("Knowledge/_inbox/_rejected/rule.md", "deja la".into()),
    ]);
    let decision = inbox(&vault).reject("Knowledge/_inbox/rule.md").unwrap();
    assert_eq!(decision.outcome, None);
    assert_eq!(decision.path, "Knowledge/_inbox/_rejected/rule-2.md");
    assert!(!vault.path().join("Knowledge/_inbox/rule.md").exists());
    assert_eq!(
        read(&vault, "Knowledge/_inbox/_rejected/rule.md"),
        "deja la"
    );
}

#[test]
fn unknown_and_escaping_names_are_refused() {
    let vault = vault_with(&[("Knowledge/_inbox/rule.md", inbox_note(""))]);
    assert!(matches!(
        inbox(&vault).approve("missing").unwrap_err(),
        MemoryError::NotFound(_)
    ));
    assert!(inbox(&vault).reject("..").is_err());
    assert!(inbox(&vault).reject("").is_err());
    let outside = vault_with(&[("secret.md", inbox_note(""))]);
    let escaped = format!("..\\..\\{}", outside.path().join("secret.md").display());
    assert!(inbox(&vault).approve(&escaped).is_err());
    assert!(outside.path().join("secret.md").exists());
}

#[test]
fn a_repo_written_in_the_note_never_chooses_the_destination() {
    let vault = vault_with(&[(
        "Knowledge/_inbox/rule.md",
        inbox_note("action: add\nscope: project\nrepo: ../../../escape\n"),
    )]);
    let decision = inbox(&vault).approve("rule").unwrap();
    assert_eq!(decision.path, "Knowledge/Mini/notes/unity/rule.md");
    assert!(!vault.path().join("../escape").exists());
}

#[test]
fn a_supersede_with_an_unusable_target_writes_nothing() {
    for target in [
        "[[Knowledge/_global/unity/ghost]]",
        "[[Knowledge/_inbox/other]]",
        "[[Knowledge/Other/notes/unity/x]]",
    ] {
        let vault = vault_with(&[
            (
                "Knowledge/_inbox/rule.md",
                inbox_note(&format!(
                    "action: supersede\nscope: global\ntarget: \"{target}\"\n"
                )),
            ),
            ("Knowledge/Other/notes/unity/x.md", OLD.to_string()),
            ("Knowledge/_inbox/other.md", OLD.to_string()),
        ]);
        assert!(inbox(&vault).approve("rule").is_err(), "{target}");
        assert!(!vault
            .path()
            .join("Knowledge/_global/unity/rule.md")
            .exists());
        assert!(vault.path().join("Knowledge/_inbox/rule.md").exists());
    }
}
