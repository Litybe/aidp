use super::*;
use clap::error::ErrorKind;

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    let mut argv = vec!["aidp"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv)
}

#[test]
fn overlap_accepts_the_closed_unit_interval() {
    assert_eq!(parse_overlap("0"), Ok(0.0));
    assert_eq!(parse_overlap("0.5"), Ok(0.5));
    assert_eq!(parse_overlap("1"), Ok(1.0));
}

#[test]
fn overlap_outside_the_unit_interval_names_the_range() {
    for raw in ["42", "-0.1", "1.01"] {
        let message = parse_overlap(raw).expect_err("out of range");
        assert!(message.contains("0..=1"), "{message}");
    }
}

#[test]
fn overlap_that_is_not_a_number_is_refused() {
    let message = parse_overlap("half").expect_err("not a number");
    assert!(message.contains("0..=1"), "{message}");
}

#[test]
fn window_of_zero_is_refused_and_one_is_accepted() {
    assert_eq!(parse_window("1"), Ok(1));
    assert!(parse_window("0").is_err());
    assert!(parse_window("-3").is_err());
}

#[test]
fn git_outcomes_rejects_a_bad_overlap_at_parse_time() {
    let error = parse(&["git", "outcomes", "--overlap", "42"])
        .err()
        .expect("overlap 42 is refused");
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
    assert!(error.to_string().contains("0..=1"));
}

#[test]
fn git_outcomes_rejects_a_zero_window() {
    let error = parse(&["git", "outcomes", "--window", "0"])
        .err()
        .expect("window 0 is refused");
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
}

#[test]
fn key_value_splits_on_the_first_equals_sign() {
    assert_eq!(
        parse_key_value("stack=csharp"),
        Ok(("stack".to_string(), "csharp".to_string()))
    );
    assert_eq!(
        parse_key_value("query=a=b"),
        Ok(("query".to_string(), "a=b".to_string()))
    );
    assert_eq!(
        parse_key_value("empty="),
        Ok(("empty".to_string(), String::new()))
    );
}

#[test]
fn key_value_refuses_an_empty_key_or_a_missing_separator() {
    assert!(parse_key_value("=value").is_err());
    assert!(parse_key_value("   =value").is_err());
    assert!(parse_key_value("novalue").is_err());
}

#[test]
fn run_start_collects_typed_input_pairs() {
    let cli = parse(&[
        "run",
        "start",
        "--pipeline",
        "dev",
        "--input",
        "a=1",
        "--input",
        "b=2",
    ])
    .expect("valid");
    let Cmd::Run {
        op: RunOp::Start { input, .. },
    } = cli.cmd
    else {
        panic!("expected run start");
    };
    assert_eq!(
        input,
        vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string())
        ]
    );
}

#[test]
fn run_start_refuses_an_input_with_an_empty_key() {
    let error = parse(&["run", "start", "--pipeline", "dev", "--input", "=x"])
        .err()
        .expect("empty key is refused");
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
}

#[test]
fn validate_all_conflicts_with_inputs() {
    let error = parse(&["validate", "--all", "--inputs", "{}"])
        .err()
        .expect("--all with --inputs is refused");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn validate_needs_a_pipeline_or_all() {
    let error = parse(&["validate"]).err().expect("a target is required");
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    assert!(parse(&["validate", "dev"]).is_ok());
    assert!(parse(&["validate", "--all"]).is_ok());
}

#[test]
fn validate_all_conflicts_with_a_named_pipeline() {
    let error = parse(&["validate", "dev", "--all"])
        .err()
        .expect("a pipeline and --all are exclusive");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn global_flags_are_accepted_before_the_verb() {
    let cli = parse(&[
        "--workspace-root",
        "/w",
        "--vault",
        "/v",
        "--format",
        "ndjson",
        "-vv",
        "doctor",
    ])
    .expect("valid");
    assert_eq!(cli.workspace_root, Some(PathBuf::from("/w")));
    assert_eq!(cli.vault, Some(PathBuf::from("/v")));
    assert_eq!(cli.format, Some(FormatArg::Ndjson));
    assert_eq!(cli.verbose, 2);
}

#[test]
fn global_flags_are_accepted_after_the_verb() {
    let cli = parse(&[
        "run",
        "state",
        "--run",
        "r1",
        "--workspace-root",
        "/w",
        "--vault",
        "/v",
        "--format",
        "json-pretty",
        "-v",
    ])
    .expect("valid");
    assert_eq!(cli.workspace_root, Some(PathBuf::from("/w")));
    assert_eq!(cli.vault, Some(PathBuf::from("/v")));
    assert_eq!(cli.format, Some(FormatArg::JsonPretty));
    assert_eq!(cli.verbose, 1);
}

#[test]
fn no_vault_conflicts_with_vault() {
    let error = parse(&["--no-vault", "--vault", "/v", "doctor"])
        .err()
        .expect("exclusive");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn format_and_verbosity_default_to_unset() {
    let cli = parse(&["doctor"]).expect("valid");
    assert_eq!(cli.format, None);
    assert_eq!(cli.verbose, 0);
    assert!(!cli.no_vault);
}

#[test]
fn knowledge_stale_no_longer_accepts_since() {
    let error = parse(&["knowledge", "stale", "--since", "2026-01-01"])
        .err()
        .expect("--since is gone");
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn command_declaration_is_consistent() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
}

#[test]
fn version_is_not_the_stale_workspace_version() {
    use clap::CommandFactory;
    let version = Cli::command().get_version().map(str::to_string);
    assert_eq!(version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_ne!(version.as_deref(), Some("4.0.0"));
}

/// Proves each `-v` repetition is counted, so `-vv` yields 2.
#[test]
fn repeated_short_verbose_flag_is_counted() {
    let cli = parse(&["-vv", "vault", "status"]).expect("parses");
    assert_eq!(cli.verbose, 2);
}
