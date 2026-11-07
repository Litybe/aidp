use aidp_cli::execute_embedded;
use aidp_testkit::{EnvGuard, TempWorkspace};

const HOME_ENV: &str = "HOME";
const USERPROFILE_ENV: &str = "USERPROFILE";
const SKILL_ENV: &str = "AIDP_SKILL_ROOT";
const VAULT_ENV: &str = "AIDP_VAULT";
const VAULT_DISABLED: &str = "off";
const WORKSPACE: &str = "embedded-command";
const VALIDATE: &str = "validate";
const ALL: &str = "--all";
const WORKSPACE_ROOT: &str = "--workspace-root";
const FORMAT: &str = "--format";
const COMPACT: &str = "json-compact";
const RESULT_OK: &str = "ok";
const RESULT_FAILED: &str = "failed";
const EXPECTED_SUCCESS: i32 = 0;

#[test]
fn embedded_validate_uses_frozen_skill_and_returns_json_without_printing() {
    let workspace = TempWorkspace::new(WORKSPACE);
    let skill = aidp_testkit::fixtures::skill_root();
    let _environment = EnvGuard::with(&[
        (HOME_ENV, Some(workspace.path().to_str().unwrap())),
        (USERPROFILE_ENV, Some(workspace.path().to_str().unwrap())),
        (SKILL_ENV, Some(skill.to_str().unwrap())),
        (VAULT_ENV, Some(VAULT_DISABLED)),
    ]);
    let result = execute_embedded(vec![
        VALIDATE.to_string(),
        ALL.to_string(),
        FORMAT.to_string(),
        COMPACT.to_string(),
        WORKSPACE_ROOT.to_string(),
        workspace.path().to_string_lossy().into_owned(),
    ]);
    assert_eq!(result.exit_code, EXPECTED_SUCCESS, "{}", result.stderr);
    assert!(result.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value[RESULT_OK], true);
    assert_eq!(value[RESULT_FAILED], 0);
}
