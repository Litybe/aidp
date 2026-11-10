use super::*;

const PARENT: &str = "GH-227";
const BUG: &str = "GH-228";
const SCENE: &str = "Assets/Tests/Playtest/Scenes/PlaytestArena.unity";
const NO_PLAYER: &str = "FAIL scenario 3: no player in the scene (PlaytestArena)";
const NATIVE_LOST: &str = "ERROR InputSettings has lost its native object (run 12)";
const FIDELITY_EXIT: &str = "headless fidelity check exited with code 1 on sheet idle.png";
const LAYOUT_ERROR: &str = "error: layout mismatch at /tmp/run-17/sheet.png frame 4";

fn report(ticket: &str, run: &str, text: &str) -> ReportText {
    ReportText {
        ticket: ticket.to_string(),
        run_id: run.to_string(),
        text: text.to_string(),
    }
}

fn itself(ticket: &str) -> String {
    ticket.to_string()
}

fn bug_fix(files: &[&str], subject: &str) -> DoctorBugFix {
    DoctorBugFix {
        bug: BUG.to_string(),
        commit: "abc".to_string(),
        subject: subject.to_string(),
        commits: vec![format!("abc {subject}")],
        files: files
            .iter()
            .map(|path| DoctorFileChange {
                path: path.to_string(),
                added: Some(1),
                deleted: Some(1),
            })
            .collect(),
        stat: String::new(),
    }
}

#[test]
fn a_bug_is_named_by_its_branch_or_by_its_commit() {
    assert_eq!(
        bug_key_in("Merge pull request #12 from Litybe/fix/GH-228").as_deref(),
        Some(BUG)
    );
    assert_eq!(
        bug_key_in("Merge branch 'fix/GH-228' into feat/GH-227").as_deref(),
        Some(BUG)
    );
    assert_eq!(
        bug_key_in("fix(GH-229): le joueur manque (#13)").as_deref(),
        Some("GH-229")
    );
    assert_eq!(bug_key_in("feat(GH-227): la scène de test"), None);
    assert_eq!(bug_key_in("fix: sans clé"), None);
}

#[test]
fn only_files_under_the_test_infrastructure_folders_are_infrastructure() {
    assert!(is_infra_path(SCENE));
    assert!(is_infra_path("Packages/manifest.json"));
    assert!(is_infra_path("ProjectSettings/InputManager.asset"));
    assert!(!is_infra_path("Assets/Scripts/Player.cs"));
    assert!(!is_infra_path("Assets/Tests/Playtest"));
    assert!(!is_infra_path("Packages/../Assets/Scripts/Player.cs"));
    assert!(!is_infra_path("/Packages/manifest.json"));
    assert!(!is_infra_path("C:/Packages/manifest.json"));
}

#[test]
fn numstat_lines_become_file_changes_and_a_binary_file_has_no_counts() {
    let changes = file_changes("3\t1\tAssets/A.cs\n-\t-\tAssets/b.png\n");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].added, Some(3));
    assert_eq!(changes[0].deleted, Some(1));
    assert_eq!(changes[1].path, "Assets/b.png");
    assert_eq!(changes[1].added, None);
}

#[test]
fn a_line_is_normalized_without_its_path_its_numbers_or_its_ids() {
    assert_eq!(
        normalize_line("ERROR: Run 17 failed at /tmp/run-17/sheet.png frame 4 (deadbeef01)"),
        "error run # failed at <path> frame # <id>"
    );
    assert_eq!(
        normalize_line(LAYOUT_ERROR),
        normalize_line("error: layout mismatch at /tmp/run-99/sheet.png frame 12")
    );
}

#[test]
fn the_known_errors_are_recognized_by_name() {
    let text = format!("{NO_PLAYER}\n{NATIVE_LOST}\n{FIDELITY_EXIT}\nall good\n");
    let keys: Vec<String> = signatures(&text).into_iter().map(|(key, _)| key).collect();
    assert!(keys.contains(&"no_player_in_scene".to_string()), "{keys:?}");
    assert!(
        keys.contains(&"input_settings_lost_native_object".to_string()),
        "{keys:?}"
    );
    assert!(keys.contains(&"fidelity_exit_1".to_string()), "{keys:?}");
    assert_eq!(keys.len(), 3);
}

#[test]
fn a_line_without_an_error_and_a_clean_count_are_no_signature() {
    assert!(signatures("all scenarios passed\n0 errors, 0 warnings\n").is_empty());
}

#[test]
fn an_error_found_in_two_tickets_is_a_shared_pattern() {
    let reports = [
        report("GH-331", "run-a", &format!("{NO_PLAYER}\nother")),
        report("GH-333", "run-b", NO_PLAYER),
        report("GH-336", "run-c", "everything passed"),
    ];
    let shared = shared_patterns(&reports, &itself);
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].key, "no_player_in_scene");
    assert_eq!(shared[0].tickets, vec!["GH-331", "GH-333"]);
    assert_eq!(shared[0].runs, vec!["run-a", "run-b"]);
    assert!(shared[0].sample.contains("no player in the scene"));
}

#[test]
fn an_error_in_one_ticket_only_is_not_shared_even_over_several_runs() {
    let reports = [
        report("GH-227", "run-a", NO_PLAYER),
        report("GH-227", "run-b", NO_PLAYER),
        report("GH-227", "run-c", NO_PLAYER),
    ];
    assert!(shared_patterns(&reports, &itself).is_empty());
}

#[test]
fn a_bug_counts_for_its_parent_so_a_ticket_and_its_bugs_are_one_ticket() {
    let reports = [
        report(PARENT, "run-a", NO_PLAYER),
        report(BUG, "run-b", NO_PLAYER),
    ];
    let family = |ticket: &str| {
        if ticket == BUG {
            PARENT.to_string()
        } else {
            ticket.to_string()
        }
    };
    assert!(shared_patterns(&reports, &family).is_empty());
    assert_eq!(shared_patterns(&reports, &itself).len(), 1);
}

#[test]
fn a_generic_error_is_shared_once_its_run_specific_parts_are_normalized() {
    let reports = [
        report(
            "GH-1",
            "run-a",
            "error: layout mismatch at /tmp/run-17/sheet.png frame 4",
        ),
        report(
            "GH-2",
            "run-b",
            "error: layout mismatch at /var/run-3/sheet.png frame 9",
        ),
    ];
    let shared = shared_patterns(&reports, &itself);
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].key, "error layout mismatch at <path> frame #");
}

#[test]
fn the_most_shared_pattern_comes_first() {
    let reports = [
        report("GH-1", "a", &format!("{NO_PLAYER}\n{NATIVE_LOST}")),
        report("GH-2", "b", &format!("{NO_PLAYER}\n{NATIVE_LOST}")),
        report("GH-3", "c", NO_PLAYER),
    ];
    let shared = shared_patterns(&reports, &itself);
    assert_eq!(shared[0].key, "no_player_in_scene");
    assert_eq!(shared[0].tickets.len(), 3);
    assert_eq!(shared[1].tickets.len(), 2);
}

#[test]
fn a_cause_is_new_unless_an_earlier_bug_touched_the_file_for_the_same_reason() {
    let earlier = [bug_fix(
        &[SCENE],
        "fix(GH-228): le joueur manque dans la scène PlaytestArena",
    )];
    assert!(!differs_from_previous(
        SCENE,
        "le joueur manque dans la scène PlaytestArena",
        &earlier
    ));
    assert!(differs_from_previous(
        SCENE,
        "la caméra du harnais ignore le viewport",
        &earlier
    ));
    assert!(differs_from_previous(
        "Assets/Scripts/Player.cs",
        "le joueur manque dans la scène PlaytestArena",
        &earlier
    ));
}

#[test]
fn a_cause_with_no_word_cannot_be_proven_different() {
    assert!(!differs_from_previous(SCENE, "ok", &[]));
}
