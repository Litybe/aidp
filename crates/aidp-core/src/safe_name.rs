//! Shape validation for the two fields that become (part of) a filesystem
//! path under `<vault>/TODO/` — a ticket's `key` and `service`. Pure and
//! I/O-free. Moved from `aidp-tickets` (LOC-27) so `aidp-core` can validate
//! the same shapes; `aidp-tickets::safe_name` re-exports it unchanged.
//!
//! This module judges two things, not one. First, *shape* — is `value` safe
//! to use as a single path component without escaping whatever base
//! directory it gets joined onto. Second, whether `value` is safe to land,
//! unescaped, in the two places `index.rs` renders a ticket's `key` and
//! `service` raw: the `[[TODO/<service>/<key>|<key>]]` wikilink built by
//! `index.rs::ticket_link` and the Markdown table row built by
//! `index.rs::dequeue_table` — plus whether it is safe as half of the
//! `<key>.md` filename this becomes on disk, on every platform, since a
//! vault gets synced between machines.
//!
//! It still knows nothing about which names are *reserved* by this crate's
//! own conventions (`unrouted`, `archive`, `.state`) or which services are
//! *known* to the vault — that policy stays in `store.rs::check_service`,
//! which layers its own rules on top of [`check`]. `unrouted` is a real,
//! legitimate service name and must pass this check; so must any Jira key
//! from a project this crate has never heard of. No charset allowlist, no
//! `PREFIX-N` shape requirement — only the specific characters and names
//! that would break something downstream are refused.

use std::path::{Component, Path};

/// Refuse a `key` or `service` value that is not safe to use as a single
/// path component. `kind` is `"key"` or `"service"` and is used only to name
/// the field in the error message.
///
/// Refuses, in order:
/// - an empty or whitespace-only value;
/// - a value starting with `_` or `.` (this also covers the special
///   components `.` and `..`);
/// - a value containing `/`, `\` or `:` (`:` matters even off Windows-style
///   separators: `Path::join("C:x.md")` on Windows carries a drive prefix
///   and *replaces* the base path instead of extending it, so it is a
///   traversal of its own on this repo's primary platform);
/// - a value containing any character below `0x20` (a raw control
///   character, e.g. from a hand-edited frontmatter);
/// - a value containing `[`, `]`, `|`, `#` or a backtick — the characters
///   that would let a hand-edited `key` or `service` break out of the
///   `[[...]]` wikilink `index.rs::ticket_link` builds, or corrupt the
///   Markdown table row `index.rs::dequeue_table` builds, since both render
///   these values unescaped;
/// - a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`
///   through `COM9`, `LPT1` through `LPT9`), comparing case-insensitively
///   against the stem of `value` — the part before its first `.` — because
///   a `key` becomes `<key>.md` on disk and the reservation applies with an
///   extension too; checked on every platform, not only Windows, so a vault
///   synced between machines accepts or refuses the same names everywhere,
///   and so the rejection is this function's named message instead of an
///   opaque OS error surfacing later out of `fs::create_dir_all`.
///
/// On top of those explicit, readable rules, this also asserts the real
/// invariant: `Path::new(value).components()` must yield exactly one
/// component, a [`Component::Normal`] whose string is exactly `value`. Keep
/// both — the explicit rules give a readable error message, the component
/// assertion is what actually guarantees containment against whatever this
/// list of explicit rules failed to anticipate.
pub fn check(kind: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{kind} `{value}` must not be empty or whitespace-only");
    }
    if value.starts_with('_') || value.starts_with('.') {
        anyhow::bail!("{kind} `{value}` must not start with `_` or `.`");
    }
    if value.contains('/') || value.contains('\\') || value.contains(':') {
        anyhow::bail!("{kind} `{value}` must not contain `/`, `\\` or `:`");
    }
    if value.chars().any(|c| c < '\u{20}') {
        anyhow::bail!("{kind} `{value}` must not contain control characters");
    }
    if value
        .chars()
        .any(|c| matches!(c, '[' | ']' | '|' | '#' | '`'))
    {
        anyhow::bail!("{kind} `{value}` must not contain `[`, `]`, `|`, `#` or a backtick");
    }
    const RESERVED_DEVICE_NAMES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = value.split('.').next().unwrap_or(value);
    if RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        anyhow::bail!("{kind} `{value}` must not be a Windows reserved device name");
    }

    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if part.to_str() == Some(value) => Ok(()),
        _ => anyhow::bail!("{kind} `{value}` must be a single plain path component"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_legitimate_keys_and_services() {
        for value in ["PIXEL-5677", "LOC-1", "aidp-viewer", "unrouted", "a.b-c_d"] {
            assert!(
                check("key", value).is_ok(),
                "`{value}` should be accepted as a key"
            );
            assert!(
                check("service", value).is_ok(),
                "`{value}` should be accepted as a service"
            );
        }
    }

    #[test]
    fn refuses_traversal_and_control_shapes() {
        let refused = [
            "",
            " ",
            ".",
            "..",
            "../..",
            "../../../etc/passwd",
            "a/b",
            "a\\b",
            "_hidden",
            ".state",
            "C:evil",
            "a\nb",
        ];
        for value in refused {
            assert!(
                check("key", value).is_err(),
                "`{value:?}` should be refused as a key"
            );
            assert!(
                check("service", value).is_err(),
                "`{value:?}` should be refused as a service"
            );
        }
    }

    #[test]
    fn error_message_names_the_field_kind_and_value() {
        let err = check("key", "../evil").unwrap_err().to_string();
        assert!(
            err.contains("key"),
            "error should name the field kind: {err}"
        );
        assert!(
            err.contains("../evil"),
            "error should name the offending value: {err}"
        );
    }

    #[test]
    fn refuses_wikilink_and_markdown_breakout_characters() {
        let refused = ["Z]] **bold** [[W", "a[b", "a]b", "a|b", "a#b", "a`b"];
        for value in refused {
            assert!(
                check("key", value).is_err(),
                "`{value}` should be refused as a key"
            );
            assert!(
                check("service", value).is_err(),
                "`{value}` should be refused as a service"
            );
        }
    }

    #[test]
    fn refuses_windows_reserved_device_names() {
        let refused = [
            "CON", "con", "CoN", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "LPT9", "CON.md",
            "nul.txt",
        ];
        for value in refused {
            assert!(
                check("key", value).is_err(),
                "`{value}` should be refused as a key"
            );
            assert!(
                check("service", value).is_err(),
                "`{value}` should be refused as a service"
            );
        }

        let accepted = ["CONSOLE", "COM0", "COM10", "LPT0"];
        for value in accepted {
            assert!(
                check("key", value).is_ok(),
                "`{value}` should be accepted as a key"
            );
            assert!(
                check("service", value).is_ok(),
                "`{value}` should be accepted as a service"
            );
        }
    }
}
