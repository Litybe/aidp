use super::*;

const SERVE_LEVEL: LevelFilter = LevelFilter::INFO;
const DEFAULT_LEVEL: LevelFilter = LevelFilter::WARN;
const NO_FLAG: u8 = 0;
const ONE_FLAG: u8 = 1;
const TWO_FLAGS: u8 = 2;
const MANY_FLAGS: u8 = 5;
const RUST_LOG_VALUE: &str = "error";
const AIDP_LOG_VALUE: &str = "aidp_cli=debug";

fn filter_text(filter: EnvFilter) -> String {
    filter.to_string()
}

fn expected(level: LevelFilter) -> String {
    filter_text(EnvFilter::default().add_directive(level.into()))
}

fn some(value: &str) -> Option<String> {
    Some(value.to_string())
}

/// The verbose flag wins over both environment variables.
#[test]
fn verbose_flag_overrides_environment() {
    let filter = build_filter(
        ONE_FLAG,
        some(RUST_LOG_VALUE),
        some(AIDP_LOG_VALUE),
        DEFAULT_LEVEL,
    );
    assert_eq!(filter_text(filter), expected(LevelFilter::INFO));
}

/// Each verbosity count maps to its level, saturating at trace.
#[test]
fn verbosity_levels_saturate_at_trace() {
    let level = |count| filter_text(build_filter(count, None, None, DEFAULT_LEVEL));
    assert_eq!(level(TWO_FLAGS), expected(LevelFilter::DEBUG));
    assert_eq!(level(MANY_FLAGS), expected(LevelFilter::TRACE));
}

/// Without the flag, `RUST_LOG` wins over `AIDP_LOG`.
#[test]
fn rust_log_overrides_aidp_log() {
    let filter = build_filter(
        NO_FLAG,
        some(RUST_LOG_VALUE),
        some(AIDP_LOG_VALUE),
        DEFAULT_LEVEL,
    );
    assert_eq!(
        filter_text(filter),
        filter_text(EnvFilter::new(RUST_LOG_VALUE))
    );
}

/// `AIDP_LOG` applies when `RUST_LOG` is unset or empty.
#[test]
fn aidp_log_applies_when_rust_log_is_empty() {
    let filter = build_filter(NO_FLAG, some(""), some(AIDP_LOG_VALUE), DEFAULT_LEVEL);
    assert_eq!(
        filter_text(filter),
        filter_text(EnvFilter::new(AIDP_LOG_VALUE))
    );
}

/// Empty values fall through to the default level.
#[test]
fn empty_values_fall_back_to_default() {
    let filter = build_filter(NO_FLAG, some(""), some(""), DEFAULT_LEVEL);
    assert_eq!(filter_text(filter), expected(DEFAULT_LEVEL));
}

/// The gateway serve default is info.
#[test]
fn serve_default_is_info() {
    let filter = build_filter(NO_FLAG, None, None, SERVE_LEVEL);
    assert_eq!(filter_text(filter), expected(LevelFilter::INFO));
}
