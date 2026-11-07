mod hook_wake_tests {
    use aidp_core::ExitStatus;

    #[test]
    fn wakes_with_the_reserved_status() {
        let outcome = super::super::HookOutcome::wake("wake up", true);
        assert_eq!(outcome.status, ExitStatus::HookWake);
    }

    #[test]
    fn wake_carries_the_directive_on_stderr_and_json_on_stdout() {
        let outcome = super::super::HookOutcome::wake("wake up", true);
        assert_eq!(outcome.stderr.as_deref(), Some("wake up"));
        let stdout = outcome.stdout.expect("wake writes a payload");
        assert!(stdout.contains("wake up"), "{stdout}");
    }

    #[test]
    fn a_noop_is_silent_and_successful() {
        let outcome = super::super::HookOutcome::noop();
        assert!(outcome.stdout.is_none() && outcome.stderr.is_none());
        assert_eq!(outcome.status, ExitStatus::Success);
    }
}

mod dev_target_strict_check_tests {
    use crate::dev_target;
    use aidp_testkit::TempWorkspace;

    #[test]
    fn a_missing_config_reports_a_failed_check() {
        let workspace = TempWorkspace::new("dev-target-strict-check");
        let ws = workspace.path().to_path_buf();

        let report = dev_target::check(&ws, false).unwrap();

        assert!(!report.ok);
    }
}
