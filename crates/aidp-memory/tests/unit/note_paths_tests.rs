use super::*;

#[test]
fn global_and_project_notes_have_their_own_folders() {
    assert_eq!(
        note_rel_path(ProposalScope::Global, "Mini", Domain::Unity, "coyote"),
        "Knowledge/_global/unity/coyote.md"
    );
    assert_eq!(
        note_rel_path(ProposalScope::Project, "Mini", Domain::PixelArt, "outline"),
        "Knowledge/Mini/notes/pixel-art/outline.md"
    );
}

#[test]
fn knowledge_paths_accept_both_spellings_and_refuse_escapes() {
    assert_eq!(
        knowledge_rel_path("_global/unity/a.md").unwrap(),
        "Knowledge/_global/unity/a.md"
    );
    assert_eq!(
        knowledge_rel_path("Knowledge\\_global\\unity\\a.md").unwrap(),
        "Knowledge/_global/unity/a.md"
    );
    for raw in ["../a.md", "_global/../../a.md", "/etc/a.md", "", "C:/a.md"] {
        assert!(knowledge_rel_path(raw).is_err(), "{raw}");
    }
}

#[test]
fn wikilinks_drop_the_extension_and_are_quoted() {
    assert_eq!(
        wikilink("Knowledge/_global/unity/a.md"),
        "\"[[Knowledge/_global/unity/a]]\""
    );
}

#[test]
fn free_path_numbers_a_name_already_taken() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(free_path(dir.path(), "note"), dir.path().join("note.md"));
    std::fs::write(dir.path().join("note.md"), "x").unwrap();
    assert_eq!(free_path(dir.path(), "note"), dir.path().join("note-2.md"));
    std::fs::write(dir.path().join("note-2.md"), "x").unwrap();
    assert_eq!(free_path(dir.path(), "note"), dir.path().join("note-3.md"));
}

#[test]
fn vault_relative_uses_forward_slashes() {
    let root = Path::new("vault");
    let path = absolute(root, "Knowledge/_inbox/a.md");
    assert_eq!(vault_relative(root, &path), "Knowledge/_inbox/a.md");
}

#[test]
fn note_targets_are_limited_to_the_global_and_repo_notes() {
    for ok in [
        "Knowledge/_global/unity/a.md",
        "Knowledge/Mini/notes/unity/a.md",
    ] {
        assert!(ensure_note_target(ok, "Mini").is_ok(), "{ok}");
    }
    for bad in [
        "Knowledge/Other/notes/unity/a.md",
        "Knowledge/Mini/unity/a.md",
        "Knowledge/_inbox/a.md",
        "Knowledge/_history/a.md",
        "Knowledge/_global/_x/a.md",
        "Knowledge/Mini/notes/_x/a.md",
        "Knowledge/_global/unity/a.txt",
        "Knowledge/_global",
        "Knowledge/_glossary.md",
    ] {
        assert!(ensure_note_target(bad, "Mini").is_err(), "{bad}");
    }
}
