//! Characterization test that freezes the `aidp ticket` command lifecycle:
//! creation, status transitions, rendering, plan dispatch decisions, and index generation.
//!
//! Each test is independent and covers one aspect of the ticket system's contract:
//! - Scenario 1: unknown service rejection (both stdout JSON and stderr text)
//! - Scenario 2: render yields exact body content after frontmatter
//! - Scenario 3: illegal transitions are rejected without side effects
//! - Scenario 4: unknown status is rejected (distinct from illegal transition error)
//! - Scenario 5: transition journal format (from/to/actor/reason projection)
//! - Scenario 6: plan command yields launches and skipped reasons in both formats
//! - Scenario 7: reindex regenerates board and service indexes with correct sections

use crate::common;
use aidp_testkit::snapshot_tree;
use common::*;
use std::fs;

/// Scenario 1: `aidp ticket new --service <unknown>` is rejected on stderr only.
/// The command's own `run()` doc states the error is returned unrendered so
/// `aidp-cli`'s `main` is the sole place that prints it, on stderr — stdout
/// carries nothing for a failed command.
#[test]
fn new_rejects_an_unknown_service_on_stderr_only() {
    let ws = workspace();

    // Do NOT seed the vault with the "nope" service
    fs::create_dir_all(ws.child("vault")).expect("Failed to create vault");

    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("new")
        .arg("--service")
        .arg("nope")
        .arg("--summary")
        .arg("This should fail");

    let output = cmd.output().expect("Failed to run ticket new");
    assert_eq!(
        output.status.code(),
        Some(1),
        "ticket new: unknown service should exit 1"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout_str.is_empty(),
        "stdout should be empty on failure, got: {}",
        stdout_str
    );

    let stderr_str = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr_str.starts_with("Error: unknown service "),
        "stderr should start with 'Error: unknown service ', got: {}",
        stderr_str
    );
    assert!(
        stderr_str.contains("pass --allow-unknown-service to bypass"),
        "stderr should contain the bypass instruction, got: {}",
        stderr_str
    );
}

/// Scenario 2: `aidp ticket render <KEY>` outputs exactly the body content after frontmatter,
/// with exactly one trailing newline, containing neither `aidp:` nor `status:`.
#[test]
fn render_prints_the_raw_body_byte_for_byte() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create LOC-1 with a known body
    let ticket_path = ws.child("vault").join("TODO").join("demo").join("LOC-1.md");
    fs::create_dir_all(ticket_path.parent().unwrap()).expect("Failed to create TODO/demo");

    let content = "---\naidp: ticket\nkey: LOC-1\nsource: local\ntype: Task\nsummary: Test ticket\nstatus: todo\npriority: Medium\nservice: demo\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nThis is the ticket body.\n\nIt can have multiple lines.\n";
    fs::write(&ticket_path, content).expect("Failed to write LOC-1.md");

    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("render")
        .arg("LOC-1");

    let output = cmd.output().expect("Failed to run ticket render");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ticket render: should exit 0"
    );

    let expected_body = "This is the ticket body.\n\nIt can have multiple lines.\n";

    assert_eq!(
        output.stdout,
        expected_body.as_bytes(),
        "render output should be exact body content with one trailing newline"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout_str.contains("aidp:"),
        "render should not include frontmatter fields"
    );
    assert!(
        !stdout_str.contains("status:"),
        "render should not include status field"
    );
}

/// Scenario 3: Illegal status transition (from todo to done, which is not in the matrix)
/// is rejected without any state changes. Asserts snapshot before and after are identical.
#[test]
fn an_illegal_transition_leaves_no_trace_on_disk() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create LOC-1
    let ticket_path = ws.child("vault").join("TODO").join("demo").join("LOC-1.md");
    fs::create_dir_all(ticket_path.parent().unwrap()).expect("Failed to create TODO/demo");
    fs::write(
        &ticket_path,
        "---\naidp: ticket\nkey: LOC-1\nsource: local\ntype: Task\nsummary: Test\nstatus: todo\npriority: Medium\nservice: demo\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nBody\n",
    )
    .expect("Failed to write LOC-1.md");

    // Create the demo directory for snapshotting (avoid picking up config.yaml from parent TODO)
    let demo_ticket_dir = ws.child("vault").join("TODO").join("demo");

    // Record the snapshot before the transition
    let snapshot_before = snapshot_tree(&demo_ticket_dir);

    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("status")
        .arg("LOC-1")
        .arg("done");

    let output = cmd.output().expect("Failed to run ticket status");
    assert_eq!(
        output.status.code(),
        Some(4),
        "illegal transition should exit 4 (policy refusal)"
    );

    let stderr_str = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr_str.contains("not a legal transition"),
        "error message should mention 'not a legal transition'"
    );
    assert!(
        stderr_str.contains("from `todo` the allowed targets are"),
        "error message should list allowed targets with backticks"
    );

    // Take snapshot after the transition
    let snapshot_after = snapshot_tree(&demo_ticket_dir);

    // Assert they are identical
    assert_eq!(
        snapshot_before, snapshot_after,
        "illegal transition should not modify any files on disk"
    );
}

/// Scenario 4: Unknown status is rejected with a distinct error message
/// (different from "not a legal transition"). Snapshot before/after must be identical.
#[test]
fn an_unknown_status_is_refused_by_the_parser_not_the_matrix() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create LOC-1
    let ticket_path = ws.child("vault").join("TODO").join("demo").join("LOC-1.md");
    fs::create_dir_all(ticket_path.parent().unwrap()).expect("Failed to create TODO/demo");
    fs::write(
        &ticket_path,
        "---\naidp: ticket\nkey: LOC-1\nsource: local\ntype: Task\nsummary: Test\nstatus: todo\npriority: Medium\nservice: demo\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nBody\n",
    )
    .expect("Failed to write LOC-1.md");

    // Create the demo directory for snapshotting (avoid picking up config.yaml from parent TODO)
    let demo_ticket_dir = ws.child("vault").join("TODO").join("demo");

    // Record the snapshot before the transition
    let snapshot_before = snapshot_tree(&demo_ticket_dir);

    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("status")
        .arg("LOC-1")
        .arg("zzz");

    let output = cmd.output().expect("Failed to run ticket status");
    assert_eq!(
        output.status.code(),
        Some(aidp_core::ExitStatus::Usage.code() as i32),
        "unknown status should be refused by clap with the usage exit code"
    );

    let stderr_str = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr_str.contains("invalid value 'zzz'"),
        "error message should name the invalid value"
    );
    assert!(
        stderr_str.contains(
            "[possible values: todo, queued, in-progress, in-review, done, blocked, cancelled]"
        ),
        "error message should list the possible values"
    );
    assert!(
        !stderr_str.contains("not a legal transition"),
        "error message should NOT mention 'not a legal transition' for an unknown status"
    );

    // Take snapshot after the transition attempt
    let snapshot_after = snapshot_tree(&demo_ticket_dir);

    // Assert they are identical
    assert_eq!(
        snapshot_before, snapshot_after,
        "unknown status should not modify any files on disk"
    );
}

/// Scenario 5: Transition journal format projection.
/// Create LOC-1, move it to in-progress with a reason, then read the ledger in json-compact format.
/// Assert the projection (from, to, actor, reason) matches expected values.
#[test]
fn the_ledger_projects_an_assertable_sequence() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create LOC-1
    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("new")
        .arg("--service")
        .arg("demo")
        .arg("--summary")
        .arg("Ledger test");

    let output = cmd.output().expect("Failed to run ticket new");
    assert_eq!(output.status.code(), Some(0), "ticket new should succeed");

    // Parse the key from the JSON output
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ticket new output should be JSON");
    let key = parsed["key"]
        .as_str()
        .expect("ticket new should output a key field");

    // Move to in-progress with a reason
    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("status")
        .arg(key)
        .arg("in-progress")
        .arg("--reason")
        .arg("go");

    let output = cmd.output().expect("Failed to run ticket status");
    assert_eq!(
        output.status.code(),
        Some(0),
        "transition to in-progress should succeed"
    );

    // Read the log in json-compact format
    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("log")
        .arg(key)
        .arg("--format")
        .arg("json-compact");

    let output = cmd.output().expect("Failed to run ticket log");
    assert_eq!(output.status.code(), Some(0), "ticket log should succeed");

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("log output should be valid JSON");

    assert!(parsed.is_array(), "log output should be a JSON array");

    let events = parsed.as_array().unwrap();
    assert_eq!(
        events.len(),
        2,
        "should have exactly 2 events: create + transition"
    );

    // Event 0: creation
    // from and reason are omitted when null (skip_serializing_if)
    let e0 = &events[0];
    assert!(
        e0.get("from").is_none(),
        "from field should be absent on creation event (skip_serializing_if)"
    );
    assert_eq!(e0["to"].as_str().unwrap(), "todo");
    assert_eq!(e0["actor"].as_str().unwrap(), "cli");
    assert!(
        e0.get("reason").is_none(),
        "reason field should be absent on creation event (skip_serializing_if)"
    );

    // Event 1: transition
    let e1 = &events[1];
    assert_eq!(
        e1["from"].as_str().unwrap(),
        "todo",
        "from should be the previous status"
    );
    assert_eq!(
        e1["to"].as_str().unwrap(),
        "in-progress",
        "to should be the new status"
    );
    assert_eq!(
        e1["actor"].as_str().unwrap(),
        "human",
        "actor should be human for CLI status command"
    );
    assert_eq!(
        e1["reason"].as_str().unwrap(),
        "go",
        "reason should be the provided reason"
    );

    // Never assert ts (nanosecond precision, non-deterministic)
}

/// Scenario 6: `aidp ticket plan` with json-compact and text formats.
/// Create two tickets: one without template (skipped), one with template (launched).
/// Check that plan yields launches and skipped arrays with both vocabularies (camelCase in JSON, kebab in text).
#[test]
fn plan_exposes_both_skip_vocabularies_and_a_non_empty_launches() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create LOC-1 without template (will be skipped)
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("new")
        .arg("--service")
        .arg("demo")
        .arg("--summary")
        .arg("No template");

    let output = cmd.output().expect("Failed to run ticket new for LOC-1");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ticket new LOC-1 should succeed"
    );

    // Create LOC-2 with template (will be launched)
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("new")
        .arg("--service")
        .arg("demo")
        .arg("--summary")
        .arg("With template")
        .arg("--template")
        .arg("dev-aidp");

    let output = cmd.output().expect("Failed to run ticket new for LOC-2");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ticket new LOC-2 should succeed"
    );

    // Plan with json-compact format
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("plan")
        .arg("--format")
        .arg("json-compact");

    let output = cmd.output().expect("Failed to run ticket plan");
    assert_eq!(output.status.code(), Some(0), "ticket plan should succeed");

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("plan output should be valid JSON");

    // Check launches and skipped arrays exist
    let launches = parsed["launches"]
        .as_array()
        .expect("launches should be an array");
    let skipped = parsed["skipped"]
        .as_array()
        .expect("skipped should be an array");

    // At least one launch must be present (explicit acceptance criterion)
    assert!(
        !launches.is_empty(),
        "launches must have length >= 1 (contains LOC-2 with template)"
    );

    // Check that LOC-2 is in launches with template dev-aidp
    let loc2_launch = launches
        .iter()
        .find(|l| l.get("key").and_then(|k| k.as_str()) == Some("LOC-2"))
        .expect("LOC-2 should be in launches");

    assert_eq!(
        loc2_launch["template"].as_str().unwrap(),
        "dev-aidp",
        "LOC-2 should have template dev-aidp"
    );
    assert!(
        loc2_launch.get("demand").is_some(),
        "launch should have demand object"
    );
    assert!(
        loc2_launch["demand"].get("prompt").is_some(),
        "demand should have prompt field"
    );

    // Check that LOC-1 is in skipped with noTemplate (camelCase in JSON)
    let loc1_skip = skipped
        .iter()
        .find(|s| s.get(0).and_then(|k| k.as_str()) == Some("LOC-1"))
        .expect("LOC-1 should be in skipped");

    assert_eq!(
        loc1_skip[1].as_str().unwrap(),
        "noTemplate",
        "LOC-1 skip reason should be 'noTemplate' (camelCase in JSON)"
    );

    // FROZEN BY DESIGN: two vocabularies for one concept, camelCase in JSON, kebab in text

    // Plan with text format
    let mut cmd = aidp_with_fixture_skill(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("plan")
        .arg("--format")
        .arg("text");

    let output = cmd
        .output()
        .expect("Failed to run ticket plan --format text");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ticket plan --format text should succeed"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);

    // In text format, LOC-1 line should be "LOC-1" + TAB + "no-template" (kebab-case)
    let loc1_line = stdout_str
        .lines()
        .find(|line| line.starts_with("LOC-1"))
        .expect("LOC-1 should appear in text output");

    let parts: Vec<&str> = loc1_line.split('\t').collect();
    assert_eq!(
        parts.len(),
        2,
        "LOC-1 line should have exactly 2 tab-separated parts"
    );
    assert_eq!(parts[0], "LOC-1");
    assert_eq!(
        parts[1], "no-template",
        "skip reason should be 'no-template' (kebab-case in text)"
    );
}

/// Scenario 7: `aidp ticket reindex` regenerates the board and service indexes.
/// Check exit code, output format, and the sections present in generated files.
#[test]
fn reindex_regenerates_the_board_assertable_by_its_sections() {
    let ws = workspace();

    // Create the vault and demo service
    let demo_dir = ws.child("vault").join("Pipelines").join("demo");
    fs::create_dir_all(&demo_dir).expect("Failed to create demo service");

    // Create a few test tickets
    for i in 1..=3 {
        let mut cmd = aidp(&ws);
        cmd.arg("ticket")
            .arg("--vault")
            .arg(ws.child("vault"))
            .arg("new")
            .arg("--service")
            .arg("demo")
            .arg("--summary")
            .arg(format!("Test ticket {}", i));

        let msg = format!("Failed to create LOC-{}", i);
        let output = cmd.output().expect(&msg);
        assert_eq!(output.status.code(), Some(0));
    }

    // Run reindex
    let mut cmd = aidp(&ws);
    cmd.arg("ticket")
        .arg("--vault")
        .arg(ws.child("vault"))
        .arg("reindex");

    let output = cmd.output().expect("Failed to run ticket reindex");
    assert_eq!(
        output.status.code(),
        Some(0),
        "ticket reindex should exit 0"
    );

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("reindex output should be valid JSON");

    assert_eq!(
        parsed["archived"].as_i64().unwrap_or(-1),
        0,
        "archived count should be exactly 0"
    );

    // Verify _TODO.md exists and has expected sections IN ORDER
    let todo_board = ws.child("vault").join("TODO").join("_TODO.md");
    assert!(todo_board.exists(), "_TODO.md should exist");

    let board_content = fs::read_to_string(&todo_board).expect("Failed to read _TODO.md");

    // Expected section titles IN ORDER
    let expected_sections = vec![
        "# Tickets",
        "## Dispatcher",
        "## À faire",
        "## En cours",
        "## En revue",
        "## Bloqués",
        "## Terminés (7 j)",
        "## Par service",
        "## Légende",
    ];

    // Verify each section exists
    for section in &expected_sections {
        assert!(
            board_content.contains(section),
            "_TODO.md should contain section: {}",
            section
        );
    }

    // Verify the sections appear in the correct order (offsets increase)
    let mut last_offset = 0;
    for section in &expected_sections {
        let offset = board_content
            .find(section)
            .unwrap_or_else(|| panic!("section '{}' should exist in board_content", section));
        assert!(
            offset > last_offset,
            "sections should appear in order; '{}' at offset {} but previous was at {}",
            section,
            offset,
            last_offset
        );
        last_offset = offset;
    }

    // Verify _demo.md exists and contains wikilinks
    let demo_index = ws.child("vault").join("TODO").join("demo").join("_demo.md");
    assert!(demo_index.exists(), "_demo.md should exist");

    let demo_content = fs::read_to_string(&demo_index).expect("Failed to read _demo.md");

    // Should contain wikilinks to the created tickets (at least LOC-1)
    assert!(
        demo_content.contains("[[TODO/demo/LOC-1|LOC-1]]"),
        "_demo.md should contain wikilink to LOC-1"
    );

    // Assert nothing about the `updated:` line or the Age column (both non-deterministic)
}
