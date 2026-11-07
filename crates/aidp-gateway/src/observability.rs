//! Logging building blocks of the gateway: a query-safe request path and the file layer.
//! The gateway never installs a subscriber; the binary composes the layers it wants.

use std::path::Path;
use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Span field carrying the pipeline run id.
pub const FIELD_RUN_ID: &str = "run_id";
/// Span field carrying the gateway session id.
pub const FIELD_SESSION_ID: &str = "session_id";
/// Span field carrying the ticket key.
pub const FIELD_TICKET_KEY: &str = "ticket_key";
const QUERY_SEPARATOR: char = '?';
const PARAM_SEPARATOR: char = '&';
const VALUE_SEPARATOR: char = '=';
const LOGGABLE_QUERY_PARAMS: &[&str] = &["limit", "since", "status"];
const LOG_DIR_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "gateway.log";

/// Returns the request path for logs: the query string is dropped, except the
/// parameters listed in [`LOGGABLE_QUERY_PARAMS`]. Any other parameter, `token` included, is never kept.
pub fn loggable_path(url: &str) -> String {
    let Some((path, query)) = url.split_once(QUERY_SEPARATOR) else {
        return url.to_string();
    };
    let kept: Vec<&str> = query
        .split(PARAM_SEPARATOR)
        .filter(|pair| is_loggable_param(pair))
        .collect();
    if kept.is_empty() {
        return path.to_string();
    }
    format!(
        "{path}{QUERY_SEPARATOR}{}",
        kept.join(&PARAM_SEPARATOR.to_string())
    )
}

fn is_loggable_param(pair: &str) -> bool {
    let name = pair.split(VALUE_SEPARATOR).next().unwrap_or_default();
    LOGGABLE_QUERY_PARAMS.contains(&name)
}

/// Builds the JSON layer writing `<state_dir>/logs/gateway.log` with daily rotation.
/// Keep the returned guard alive: dropping it flushes and stops the writer thread.
pub fn file_layer<S>(state_dir: &Path) -> anyhow::Result<(impl Layer<S>, WorkerGuard)>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    let log_dir = state_dir.join(LOG_DIR_NAME);
    std::fs::create_dir_all(&log_dir)?;
    let appender = tracing_appender::rolling::daily(log_dir, LOG_FILE_NAME);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let layer = fmt::layer().json().with_ansi(false).with_writer(writer);
    Ok((layer, guard))
}

#[cfg(test)]
#[path = "../tests/unit/observability_tests.rs"]
mod observability_tests;
