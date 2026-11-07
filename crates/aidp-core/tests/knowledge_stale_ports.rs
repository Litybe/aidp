use aidp_core::git::RecordingGit;
use aidp_core::knowledge::FeatureNote;
use aidp_core::knowledge_stale::{classify, stale_with, NoteFiles};
use aidp_testkit::TempWorkspace;
use std::path::PathBuf;

fn note(slug: &str, updated: &str, files: Vec<&str>) -> FeatureNote {
    FeatureNote {
        path: PathBuf::from(format!("{slug}.md")),
        slug: slug.to_string(),
        name: slug.replace('-', " "),
        aliases: Vec::new(),
        status: "active".into(),
        tickets: Vec::new(),
        updated: Some(updated.to_string()),
        area: None,
        files: files.into_iter().map(String::from).collect(),
        valid_until: None,
        superseded_by: None,
        findings: Vec::new(),
        bytes: 0,
        guard_off: false,
    }
}

fn write_note(features_dir: &std::path::Path, n: &FeatureNote) {
    let files_list = n
        .files
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!(
        "---\nfeature: {}\nslug: {}\nupdated: {}\nfiles: [{}]\n---\n\n# {}\n",
        n.name,
        n.slug,
        n.updated.as_deref().unwrap_or(""),
        files_list,
        n.name
    );
    std::fs::write(features_dir.join(format!("{}.md", n.slug)), body).unwrap();
}

#[test]
fn stale_with_recording_git_equals_classify_and_counts_seven_git_calls() {
    let temp_ws = TempWorkspace::new("knowledge-stale-ports");
    let ws = temp_ws.path().to_path_buf();
    let knowledge_dir = ws.join("Knowledge");
    let features = aidp_core::knowledge::features_dir(&knowledge_dir);
    std::fs::create_dir_all(&features).unwrap();

    let note_a = note(
        "feature-a",
        "2026-01-01",
        vec!["src/a1.rs", "src/a2.rs", "src/a3.rs"],
    );
    let note_b = note(
        "feature-b",
        "2026-02-01",
        vec!["src/b1.rs", "src/b2.rs", "src/b3.rs"],
    );
    write_note(&features, &note_a);
    write_note(&features, &note_b);

    let all_files = [
        "src/a1.rs",
        "src/a2.rs",
        "src/a3.rs",
        "src/b1.rs",
        "src/b2.rs",
        "src/b3.rs",
    ];
    for f in all_files {
        let full = ws.join(f);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, "fn x() {}").unwrap();
    }

    let dates: std::collections::BTreeMap<String, String> = [
        ("src/a1.rs", "2026-03-01T00:00:00+00:00"),
        ("src/a2.rs", "2026-03-05T00:00:00+00:00"),
        ("src/a3.rs", "2026-03-10T00:00:00+00:00"),
        ("src/b1.rs", "2026-04-01T00:00:00+00:00"),
        ("src/b2.rs", "2026-04-05T00:00:00+00:00"),
        ("src/b3.rs", "2026-04-10T00:00:00+00:00"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let git = RecordingGit {
        usable: true,
        last_commit_dates: std::sync::Mutex::new(dates.clone()),
        ..RecordingGit::new()
    };

    let out = stale_with(&knowledge_dir, &ws, &git);

    let notes = aidp_core::knowledge::list(&knowledge_dir);
    let expected_input: Vec<NoteFiles> = notes
        .into_iter()
        .map(|n| {
            let files: Vec<(String, Option<String>)> = n
                .files
                .iter()
                .map(|f| (f.clone(), dates.get(f).cloned()))
                .collect();
            (n, files)
        })
        .collect();
    let expected = classify(&expected_input);

    assert_eq!(out.len(), expected.len());
    for (a, b) in out.iter().zip(expected.iter()) {
        assert_eq!(a.slug, b.slug);
        assert_eq!(a.days_behind, b.days_behind);
        assert_eq!(a.newest_change, b.newest_change);
        assert_eq!(a.missing_files, b.missing_files);
    }

    let total_calls = git
        .calls
        .is_usable
        .load(std::sync::atomic::Ordering::SeqCst)
        + git
            .calls
            .last_commit_date
            .load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(total_calls, 7, "1 is_usable + 6 last_commit_date");

    let _ = std::fs::remove_dir_all(&ws);
}
