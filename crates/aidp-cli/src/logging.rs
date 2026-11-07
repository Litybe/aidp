//! The only subscriber of the workspace: every log goes to stderr, never to stdout.

use std::io::IsTerminal;
use std::path::Path;
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

const RUST_LOG_VAR: &str = "RUST_LOG";
const AIDP_LOG_VAR: &str = "AIDP_LOG";
/// Verbosity requested with `-v`, `-vv` and `-vvv`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verbosity {
    Quiet,
    Info,
    Debug,
    Trace,
}

impl From<u8> for Verbosity {
    fn from(count: u8) -> Self {
        match count {
            0 => Self::Quiet,
            1 => Self::Info,
            2 => Self::Debug,
            _ => Self::Trace,
        }
    }
}

impl Verbosity {
    /// The level forced by the flag; `None` without it.
    fn level(self) -> Option<LevelFilter> {
        match self {
            Self::Quiet => None,
            Self::Info => Some(LevelFilter::INFO),
            Self::Debug => Some(LevelFilter::DEBUG),
            Self::Trace => Some(LevelFilter::TRACE),
        }
    }
}

/// The first non-empty value among `RUST_LOG` then `AIDP_LOG`.
fn env_directive(rust_log: Option<String>, aidp_log: Option<String>) -> Option<String> {
    [rust_log, aidp_log]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
}

/// Filter from `-v`, else `RUST_LOG`, else `AIDP_LOG`, else the default level.
fn build_filter(
    verbose: u8,
    rust_log: Option<String>,
    aidp_log: Option<String>,
    default_level: LevelFilter,
) -> EnvFilter {
    if let Some(level) = Verbosity::from(verbose).level() {
        return EnvFilter::default().add_directive(level.into());
    }
    match env_directive(rust_log, aidp_log) {
        Some(directive) => EnvFilter::new(directive),
        None => EnvFilter::default().add_directive(default_level.into()),
    }
}

/// JSON when stderr is piped, human format with ANSI only on a terminal.
fn stderr_layer() -> Box<dyn Layer<Registry> + Send + Sync> {
    let is_terminal = std::io::stderr().is_terminal();
    let layer = fmt::layer().with_writer(std::io::stderr);
    if is_terminal {
        layer.with_ansi(true).boxed()
    } else {
        layer.json().with_ansi(false).boxed()
    }
}

/// Installs the subscriber. With a directory, the gateway file layer is added and
/// its guard returned; keep it alive until the process ends.
pub fn init_logging(verbose: u8, file_layer_dir: Option<&Path>) -> Option<WorkerGuard> {
    let default_level = if file_layer_dir.is_some() {
        LevelFilter::INFO
    } else {
        LevelFilter::WARN
    };
    let file = file_layer_dir.and_then(|dir| aidp_gateway::observability::file_layer(dir).ok());
    let (file_layer, guard) = match file {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    };
    let _ = tracing_subscriber::registry()
        .with(stderr_layer())
        .with(file_layer)
        .with(build_filter(
            verbose,
            std::env::var(RUST_LOG_VAR).ok(),
            std::env::var(AIDP_LOG_VAR).ok(),
            default_level,
        ))
        .try_init();
    guard
}

#[cfg(test)]
#[path = "../tests/unit/logging_tests.rs"]
mod tests;
