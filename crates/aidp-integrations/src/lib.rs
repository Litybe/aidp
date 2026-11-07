//! CLI/API wrappers for Jira, GitHub, SonarQube, Confluence, Teams,
//! plus the CI autopilot (`ci`) that turns CI/PR state into pipeline dispatches.
//! Preferred over MCP (per v4 rule 6). Each module exposes typed
//! functions that return data; the CLI renders them.
//!
//! All commands honor `--projection` (lean by default) so I/O is token-frugal.

pub mod branch_key;
pub mod branch_publish;
pub mod diff_hunks;
pub mod flow_repo;
pub mod gh;
pub mod github_asset;
pub mod github_issues;
pub mod github_labels;
pub mod github_rate_limit;
pub mod integration_error;
pub mod mock;
pub mod models;
pub mod net;
pub mod notifications;
pub mod points;
pub mod pr;
pub mod pr_actions;
pub mod pr_comments;
pub mod pr_detail;
pub mod pr_writes;
pub mod review_post;
pub mod squad;

pub use aidp_core::models::enums::{ErrorKind, HasErrorKind};
pub use integration_error::IntegrationError;

pub use aidp_core::program::{resolve_program, resolve_program_in};

/// Marker substituted for every secret removed from emitted text.
pub const REDACTED: &str = "<redacted>";

/// Replace occurrences of a secret (e.g. a Mongo URI with credentials) with a
/// placeholder before emitting text to stdout/stderr.
pub(crate) fn redact(s: &str, secret: &str) -> String {
    if secret.is_empty() {
        return s.to_string();
    }
    s.replace(secret, REDACTED)
}

/// Keep only scheme, host and port of a URL; userinfo, path and query are
/// replaced by the marker. An unparsable input becomes the marker alone.
pub fn redact_url(raw: &str) -> String {
    let Ok(parsed) = url::Url::parse(raw) else {
        return REDACTED.to_string();
    };
    let Some(host) = parsed.host_str() else {
        return REDACTED.to_string();
    };
    let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{}://{host}{port}/{REDACTED}", parsed.scheme())
}

/// Remove `raw_url` and its userinfo password from free text such as an
/// error message that embeds the request URL.
pub fn redact_url_in(text: &str, raw_url: &str) -> String {
    let mut out = redact(text, raw_url);
    if let Ok(parsed) = url::Url::parse(raw_url) {
        if let Some(password) = parsed.password() {
            out = redact(&out, password);
        }
    }
    out
}

/// Tiny base64 encoder (avoid extra dep). Used by Jira/Confluence basic auth.
pub fn jira_b64(s: &str) -> String {
    const ALPH: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = s.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPH[((n >> 18) & 63) as usize] as char);
        out.push(ALPH[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPH[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPH[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod lib_tests;
