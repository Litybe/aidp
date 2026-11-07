use super::*;

const RUN_ID: &str = "20261005-095643-b224";
const STAGE_NAME: &str = "fix";
const RUN_DIR: &str = "run-dir";
const VAULT_ROOT: &str = "vault";

/// A non-autonomous session carries only the run id and the stage name.
#[test]
fn session_env_exports_run_and_stage() {
    let env = session_env(RUN_ID, STAGE_NAME, false);
    assert_eq!(env.get(ENV_RUN).map(String::as_str), Some(RUN_ID));
    assert_eq!(env.get(ENV_STAGE).map(String::as_str), Some(STAGE_NAME));
    assert_eq!(env.get(AUTONOMOUS_ENV), None);
    assert_eq!(env.len(), 2);
}

/// An autonomous run exports the autonomous switch to its stage sessions.
#[test]
fn session_env_exports_autonomous_switch_for_autonomous_run() {
    let env = session_env(RUN_ID, STAGE_NAME, true);
    assert_eq!(
        env.get(AUTONOMOUS_ENV).map(String::as_str),
        Some(AUTONOMOUS_ENABLED)
    );
    assert_eq!(env.get(ENV_RUN).map(String::as_str), Some(RUN_ID));
    assert_eq!(env.len(), 3);
}

const DEFAULT_MODE: &str = "default";

/// Claude reporting another mode than the preferred one that was asked is a refusal.
#[test]
fn a_reported_mode_other_than_the_preferred_one_is_a_refusal() {
    assert!(permission_refused(
        PermissionMode::STAGE_PREFERRED,
        Some(DEFAULT_MODE)
    ));
}

/// Claude reporting the preferred mode back is no refusal.
#[test]
fn the_preferred_mode_being_honoured_is_not_a_refusal() {
    assert!(!permission_refused(
        PermissionMode::STAGE_PREFERRED,
        Some(PermissionMode::STAGE_PREFERRED.as_str())
    ));
}

/// An init message without a permission mode leaves the session alone.
#[test]
fn a_silent_init_is_not_a_refusal() {
    assert!(!permission_refused(PermissionMode::STAGE_PREFERRED, None));
}

/// Only the preferred mode is replaced: the fallback is never asked twice.
#[test]
fn the_fallback_mode_is_never_replaced() {
    assert!(!permission_refused(
        PermissionMode::STAGE_FALLBACK,
        Some(DEFAULT_MODE)
    ));
}

/// The failure reason names both modes when claude ran another one than asked.
#[test]
fn the_note_names_both_modes_on_a_mismatch() {
    let note = permission_note(PermissionMode::STAGE_FALLBACK, Some(DEFAULT_MODE));
    assert!(note.contains("`default`"), "{note}");
    assert!(
        note.contains(&format!("`{}`", PermissionMode::STAGE_FALLBACK.as_str())),
        "{note}"
    );
}

/// No note when claude honoured the mode or did not report one.
#[test]
fn the_note_is_empty_without_a_mismatch() {
    let asked = PermissionMode::STAGE_FALLBACK;
    assert_eq!(permission_note(asked, Some(asked.as_str())), "");
    assert_eq!(permission_note(asked, None), "");
}

#[test]
fn explicit_permission_modes_survive_a_previous_fallback() {
    let control = RunControl::default();
    control.fall_back_permission();
    for mode in PermissionMode::ALL {
        let requested = requested_permission_mode(Some(mode.as_str())).unwrap();
        assert_eq!(requested, Some(mode));
        assert_eq!(control.permission_mode(requested), mode);
    }
    assert_eq!(
        control.permission_mode(None),
        PermissionMode::STAGE_FALLBACK
    );
}

#[test]
fn every_explicit_permission_mismatch_is_refused() {
    for asked in PermissionMode::ALL {
        for observed in PermissionMode::ALL {
            assert_eq!(
                explicit_permission_refused(asked, Some(observed.as_str())),
                asked != observed
            );
        }
    }
}

/// A headless run names its stage mode even after a fallback: `dontAsk` unless asked otherwise.
#[test]
fn headless_runs_never_reach_the_permission_fallback() {
    let control = RunControl::default();
    control.fall_back_permission();
    for asked in [None, Some(DEFAULT_MODE)] {
        let requested = requested_stage_mode(asked, true).unwrap();
        assert_eq!(requested, Some(PermissionMode::DontAsk));
        assert_eq!(control.permission_mode(requested), PermissionMode::DontAsk);
    }
    let plan = PermissionMode::Plan;
    assert_eq!(
        requested_stage_mode(Some(plan.as_str()), true).unwrap(),
        Some(plan)
    );
    assert_eq!(requested_stage_mode(None, false).unwrap(), None);
    assert_eq!(
        requested_stage_mode(Some(DEFAULT_MODE), false).unwrap(),
        Some(PermissionMode::Default)
    );
}

/// A headless stage may use the vault root beside its run directory; another stage may not.
#[test]
fn headless_stages_add_the_vault_root_beside_the_run_directory() {
    let run_dir = Path::new(RUN_DIR);
    let vault_root = Path::new(VAULT_ROOT);
    assert_eq!(
        stage_add_dirs(run_dir, Some(vault_root), true),
        vec![run_dir.to_path_buf(), vault_root.to_path_buf()]
    );
    assert_eq!(
        stage_add_dirs(run_dir, Some(vault_root), false),
        vec![run_dir.to_path_buf()]
    );
    assert_eq!(
        stage_add_dirs(run_dir, None, true),
        vec![run_dir.to_path_buf()]
    );
}

/// A job with its own workspace overrides the run workspace; without one the run's is kept.
#[test]
fn session_job_workspace_override_reaches_the_start_request() {
    let run = Path::new("ticket");
    let isolated = Path::new("run/iso/T1");
    assert_eq!(session_workspace(Some(isolated), run), isolated);
    assert_eq!(session_workspace(None, run), run);
}
