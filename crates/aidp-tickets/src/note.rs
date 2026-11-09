//! Ticket note format: parse/render with unknown-key preservation (vision
//! §5.1 of `docs/plans/ticket-autopilot/00-vision-et-architecture.md`).
//!
//! A note is a YAML frontmatter block (`---` … `---`) whose first key is
//! `aidp: ticket`, followed by a human Markdown body. [`parse`] and
//! [`render`] are pure string functions: no filesystem I/O, and neither calls
//! `Ticket::recompute_derived` — the store owns that.
//!
//! The frontmatter is parsed into a full [`serde_yaml::Mapping`] first, then
//! projected: keys [`crate::model::Ticket`] knows go into the `Ticket`,
//! everything else is kept, in its original order, in [`ParsedNote::extra`]
//! and rendered back verbatim. This is deliberate (see the module's task
//! notes): a hand-rolled scan of a few known keys would silently drop
//! anything it doesn't recognise, and preserving unknown keys is an
//! acceptance criterion of this lot.

use crate::model::Ticket;
use crate::safe_name;
use anyhow::Context;
use chrono::{DateTime, SecondsFormat, Utc};

/// The frontmatter keys [`crate::model::Ticket`] knows about, i.e. everything
/// that is projected out of the parsed mapping rather than kept in
/// [`ParsedNote::extra`]. Mirrors `Ticket`'s own field order. `aidp` is
/// handled separately (it is the note's format marker, not a `Ticket` field
/// and not an unknown key either).
const KNOWN_KEYS: &[&str] = &[
    "key",
    "source",
    "type",
    "summary",
    "status",
    "priority",
    "priority_rank",
    "service",
    "template",
    "track",
    "route",
    "flow",
    "labels",
    "components",
    "epic",
    "assignee",
    "milestone",
    "created",
    "updated",
    "github",
    "autopilot",
    "tags",
];

/// A ticket note, split into what [`crate::model::Ticket`] knows and what it
/// doesn't.
#[derive(Debug)]
pub struct ParsedNote {
    pub ticket: crate::model::Ticket,
    /// Everything after the closing `---`, exactly as it appeared — no
    /// reordering, trimming or reflowing.
    pub body: String,
    /// Frontmatter keys `Ticket` has no field for (e.g. a newer `aidp`'s
    /// field this build doesn't know, or a human's hand-added key), in their
    /// original order. [`render`] writes them back after the known fields.
    pub extra: serde_yaml::Mapping,
}

/// Parse a ticket note's text into its [`Ticket`], body and unknown
/// frontmatter keys.
///
/// `text` is normalised first ([`normalise_body`]: a leading BOM stripped,
/// `\r\n`/lone `\r` folded to `\n`), so a note that passed through a Windows
/// editor or a PowerShell pipe still parses and still comes back as LF-only.
///
/// The frontmatter is the *first* `---` … `---` block only: a `---` line
/// appearing later — e.g. a YAML example fenced inside the body — is left
/// alone as ordinary body text, because parsing stops at the first closing
/// delimiter it finds.
///
/// Fails if the file doesn't open with `---`, if the frontmatter is never
/// closed, if it doesn't parse as a YAML mapping, if its `aidp` key is
/// anything other than the literal string `ticket` (including absent), or
/// if the projected `Ticket`'s `key` or `service` is not a single safe path
/// component ([`safe_name::check`]) — these are all reported as a clear
/// `anyhow` error, never a panic.
///
/// That last check is what closes the path-traversal hole a hand-edited
/// note can otherwise open: a frontmatter carrying `key: "../../escaped"` or
/// `service: "../.."` would, without it, parse successfully and later let
/// `TicketStore::save` write outside `<vault>/TODO/`. Every caller of this
/// function already treats a parse failure as "skip this one note, keep
/// going" ([`crate::store`]'s `collect_notes` reports it on stderr and moves
/// on; `list` still succeeds) — never let this particular failure become an
/// exception to that, e.g. by softening it into a warning: a poisoned note
/// must disappear from every listing, not merely log louder.
pub fn parse(text: &str) -> anyhow::Result<ParsedNote> {
    let normalised = normalise_body(text);

    let after_open = normalised.strip_prefix("---\n").ok_or_else(|| {
        anyhow::anyhow!(
            "not an aidp ticket note: expected the file to start with a `---` frontmatter delimiter"
        )
    })?;

    let (yaml_str, body) = split_frontmatter(after_open).ok_or_else(|| {
        anyhow::anyhow!(
            "not an aidp ticket note: the frontmatter is never closed by a `---` delimiter"
        )
    })?;

    let value: serde_yaml::Value =
        serde_yaml::from_str(yaml_str).context("failed to parse ticket frontmatter as YAML")?;
    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        other => anyhow::bail!(
            "not an aidp ticket note: frontmatter must be a YAML mapping, found {}",
            yaml_type_name(&other)
        ),
    };

    match mapping.get("aidp") {
        Some(serde_yaml::Value::String(s)) if s == "ticket" => {}
        Some(other) => anyhow::bail!(
            "not an aidp ticket note: `aidp:` is `{}`, expected `ticket`",
            yaml_scalar_display(other)
        ),
        None => {
            anyhow::bail!("not an aidp ticket note: missing the `aidp: ticket` frontmatter key")
        }
    }

    let mut known = serde_yaml::Mapping::new();
    let mut extra = serde_yaml::Mapping::new();
    for (k, v) in mapping {
        let is_aidp_marker = matches!(&k, serde_yaml::Value::String(s) if s == "aidp");
        if is_aidp_marker {
            continue;
        }
        let is_known =
            matches!(&k, serde_yaml::Value::String(s) if KNOWN_KEYS.contains(&s.as_str()));
        if is_known {
            known.insert(k, v);
        } else {
            extra.insert(k, v);
        }
    }

    let ticket: Ticket = serde_yaml::from_value(serde_yaml::Value::Mapping(known))
        .context("failed to deserialize ticket fields from frontmatter")?;

    safe_name::check("key", &ticket.key)?;
    safe_name::check("service", &ticket.service)?;

    Ok(ParsedNote {
        ticket,
        body: body.to_string(),
        extra,
    })
}

/// A fast-path hint at a note's `status`, without the full YAML-to-`Ticket`
/// deserialize [`parse`] does.
///
/// Reuses the same [`normalise_body`] + `strip_prefix("---\n")` +
/// [`split_frontmatter`] steps `parse` itself uses, then parses the
/// frontmatter block as a bare `serde_yaml::Value` and reads its `status`
/// key. This is a HINT only, never a source of truth or of errors: any
/// failure at any step (not frontmatter-shaped, no `status` key, unparseable
/// status) returns `None`, and every caller must fall back to the real
/// [`parse`] path on `None` — today's error-reporting-and-skip behaviour for
/// a genuinely broken note must stay unchanged. See `store::collect_notes`,
/// this function's one caller.
pub(crate) fn peek_status(text: &str) -> Option<crate::model::Status> {
    let normalised = normalise_body(text);
    let after_open = normalised.strip_prefix("---\n")?;
    let (yaml_str, _body) = split_frontmatter(after_open)?;
    let value: serde_yaml::Value = serde_yaml::from_str(yaml_str).ok()?;
    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        _ => return None,
    };
    let status_value = mapping.get("status")?;
    serde_yaml::from_value(status_value.clone()).ok()
}

/// Rebuild a ticket note from its parts.
///
/// The frontmatter is fully rewritten every time — this is never a
/// byte-preserving patch of some prior version. `aidp: ticket` is always the
/// first key. Every known [`Ticket`] field comes next, in the struct's own
/// declaration order (matching vision §5.1): reflected out of `ticket` via
/// `serde_yaml` rather than hand-listed here, so a field added to
/// [`crate::model`] later shows up for free, exactly like `Ticket`'s own doc
/// comments describe for the store layer. `created`, `updated`,
/// `autopilot.queued_at`, `autopilot.started_at` and `github.synced_at` are then
/// patched to the exact `SecondsFormat::Secs` RFC3339 form vision §5.1 uses
/// (`2026-09-10T08:12:00Z`) — chrono's default `Serialize` keeps sub-second
/// precision when a timestamp has any, which would otherwise leak through.
/// `extra`'s keys are appended last, in their original order.
///
/// `body` is emitted exactly as given, byte for byte: this function never
/// reorders, trims or reflows it. The returned string ends with exactly one
/// trailing `\n`.
pub fn render(ticket: &Ticket, body: &str, extra: &serde_yaml::Mapping) -> String {
    let mut ticket_mapping = match serde_yaml::to_value(ticket) {
        Ok(serde_yaml::Value::Mapping(m)) => m,
        _ => unreachable!("Ticket always serialises to a YAML mapping"),
    };

    let secs = |dt: DateTime<Utc>| dt.to_rfc3339_opts(SecondsFormat::Secs, true);
    ticket_mapping.insert(
        serde_yaml::Value::String("created".to_string()),
        serde_yaml::Value::String(secs(ticket.created)),
    );
    ticket_mapping.insert(
        serde_yaml::Value::String("updated".to_string()),
        serde_yaml::Value::String(secs(ticket.updated)),
    );
    if let Some(serde_yaml::Value::Mapping(autopilot)) = ticket_mapping.get_mut("autopilot") {
        if let Some(dt) = ticket.autopilot.queued_at {
            autopilot.insert(
                serde_yaml::Value::String("queued_at".to_string()),
                serde_yaml::Value::String(secs(dt)),
            );
        }
        if let Some(dt) = ticket.autopilot.started_at {
            autopilot.insert(
                serde_yaml::Value::String("started_at".to_string()),
                serde_yaml::Value::String(secs(dt)),
            );
        }
    }
    if let Some(serde_yaml::Value::Mapping(github_map)) = ticket_mapping.get_mut("github") {
        if let Some(dt) = ticket.github.as_ref().and_then(|g| g.synced_at) {
            github_map.insert(
                serde_yaml::Value::String("synced_at".to_string()),
                serde_yaml::Value::String(secs(dt)),
            );
        }
    }

    let mut frontmatter = serde_yaml::Mapping::new();
    frontmatter.insert(
        serde_yaml::Value::String("aidp".to_string()),
        serde_yaml::Value::String("ticket".to_string()),
    );
    for (k, v) in ticket_mapping {
        frontmatter.insert(k, v);
    }
    for (k, v) in extra {
        frontmatter.insert(k.clone(), v.clone());
    }

    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(frontmatter))
        .expect("a Mapping of Values always serialises to YAML");
    // Defensive: some serde_yaml configurations emit a leading `---` document
    // marker of their own. Strip it so we control the delimiter ourselves and
    // never end up with two.
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let yaml = yaml.trim_end_matches('\n');

    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(yaml);
    out.push('\n');
    out.push_str("---\n");
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Normalise line endings to `\n` and strip a leading UTF-8 BOM.
///
/// [`parse`] applies this to a whole note's text on ingestion. It is also
/// exposed for the CLI to run independently over `--body`, `--body-file` and
/// `--stdin` content before it ever reaches this crate — without it, a body
/// typed on Windows or piped from PowerShell carries `\r\n` all the way into
/// the note, and the "UTF-8 without BOM, LF endings" acceptance criterion is
/// false on the very first real use.
pub fn normalise_body(input: &str) -> String {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// The body `aidp ticket new` writes when no `--body`, `--body-file` or piped
/// stdin content is supplied — vision §5.1's template: an H1 title, then the
/// French level-2 sections `## 🎯 Objectif`, `## 🔧 Impacts techniques`,
/// `## ✅ Critères d'acceptation`, then `## Notes locales`.
///
/// Note for future readers: the `create-ticket` skill uses an **English**
/// `###`/`####`-heading variant of this same shape (Objective / Technical
/// impacts / Acceptance criteria, one level deeper). This lot deliberately
/// follows the vision document instead — it is the more recent decision, and
/// it is the shape `discover_prompt` actually consumes in this repo.
pub fn default_body(key: &str, summary: &str) -> String {
    format!(
        "# {key} - {summary}\n\n\
        ## 🎯 Objectif\n\n\
        ## 🔧 Impacts techniques\n\n\
        ## ✅ Critères d'acceptation\n\n\
        ## Notes locales\n"
    )
}

/// Split the text following the opening `---\n` into `(yaml, body)` at the
/// first closing `---` delimiter. `input` is assumed already
/// newline-normalised (only `\n`, no `\r`).
fn split_frontmatter(input: &str) -> Option<(&str, &str)> {
    if let Some(idx) = input.find("\n---\n") {
        return Some((&input[..idx], &input[idx + 5..]));
    }
    if let Some(yaml) = input.strip_suffix("\n---") {
        return Some((yaml, ""));
    }
    if input == "---" {
        return Some(("", ""));
    }
    None
}

fn yaml_type_name(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "a bool",
        serde_yaml::Value::Number(_) => "a number",
        serde_yaml::Value::String(_) => "a string",
        serde_yaml::Value::Sequence(_) => "a sequence",
        serde_yaml::Value::Mapping(_) => "a mapping",
        serde_yaml::Value::Tagged(_) => "a tagged value",
    }
}

fn yaml_scalar_display(v: &serde_yaml::Value) -> String {
    serde_yaml::to_string(v)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Autopilot, IssueType, Priority, Source, Status};
    use crate::models::blocker_link::BlockerLink;
    use crate::models::enums::issue_state::IssueState;
    use crate::models::github_meta::GithubMeta;
    use crate::models::milestone::Milestone;
    use chrono::TimeZone;

    #[test]
    fn round_trip_full_note() {
        let created = Utc.with_ymd_and_hms(2026, 9, 10, 8, 12, 0).unwrap();
        let updated = Utc.with_ymd_and_hms(2026, 9, 10, 9, 40, 0).unwrap();

        let ticket = Ticket {
            key: "GH-5677".to_string(),
            source: Source::Github,
            r#type: IssueType::Story,
            summary: "Ajouter le motif de blocage dans l'alerte".to_string(),
            status: Status::Todo,
            priority: Priority::High,
            priority_rank: 2,
            service: "PixelHood".to_string(),
            template: Some("dev-unity".to_string()),
            track: Some(crate::models::track::Track::Standard),
            route: Some("code".to_string()),
            flow: Some("game-code".to_string()),
            labels: vec!["dev".to_string(), "aidp:ready".to_string()],
            components: Vec::new(),
            epic: None,
            assignee: Some("Alexis Vuk".to_string()),
            milestone: Some(Milestone {
                title: "RM-1".to_string(),
                number: Some(3),
                due_on: Some(created),
            }),
            created,
            updated,
            github: Some(GithubMeta {
                repo: "Litybe/PixelHood".to_string(),
                number: 5677,
                url: "https://github.com/Litybe/PixelHood/issues/5677".to_string(),
                state: IssueState::Open,
                synced_at: Some(updated),
                blocked_by: vec![BlockerLink {
                    key: "GH-12".to_string(),
                    state: IssueState::Closed,
                }],
                base_branch: None,
                parent: None,
            }),
            autopilot: Autopilot {
                attempts: 0,
                queued_at: None,
                started_at: None,
                session_id: None,
                run_id: None,
                branch: None,
                pr: None,
                blocked_reason: None,
                manual_priority: None,
                adopted: false,
                ..Autopilot::default()
            },
            tags: vec![
                "aidp/ticket".to_string(),
                "aidp/ticket/todo".to_string(),
                "service/PixelHood".to_string(),
                "source/github".to_string(),
            ],
        };

        let body = "# GH-5677 - Ajouter le motif de blocage dans l'alerte\n\n\
             ## 🎯 Objectif\n\
             Ajouter le motif.\n\n\
             ## 🔧 Impacts techniques\n\
             RAS.\n\n\
             ## ✅ Critères d'acceptation\n\
             - Le motif est visible.\n\n\
             ## Notes locales\n\
             (préservé à chaque synchronisation)\n"
            .to_string();

        let text = render(&ticket, &body, &serde_yaml::Mapping::new());

        assert!(text.starts_with("---\naidp: ticket\n"));
        assert!(text.contains("created: 2026-09-10T08:12:00Z"));
        assert!(text.contains("updated: 2026-09-10T09:40:00Z"));
        assert!(text.contains("synced_at: 2026-09-10T09:40:00Z"));

        let parsed = parse(&text).expect("a fully populated note round trips");
        assert_eq!(parsed.ticket, ticket);
        assert_eq!(parsed.body, body);
        assert!(parsed.extra.is_empty());
    }

    #[test]
    fn round_trip_minimal_note() {
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-8\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Minimal ticket\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "# LOC-8 - Minimal ticket\n",
        );

        let parsed = parse(note).expect("a note with only the mandatory keys parses");
        let t = &parsed.ticket;
        assert_eq!(t.key, "LOC-8");
        assert_eq!(t.source, Source::Local);
        assert_eq!(t.r#type, IssueType::Task);
        assert_eq!(t.summary, "Minimal ticket");
        assert_eq!(t.status, Status::Todo);
        assert_eq!(t.priority, Priority::Medium);
        assert_eq!(t.service, "aidp-viewer");

        // Every absent optional/derived field takes its serde default.
        assert_eq!(t.priority_rank, 0);
        assert_eq!(t.template, None);
        assert_eq!(t.track, None);
        assert!(t.labels.is_empty());
        assert!(t.components.is_empty());
        assert_eq!(t.epic, None);
        assert_eq!(t.assignee, None);
        assert_eq!(t.github, None);
        assert_eq!(t.route, None);
        assert_eq!(t.flow, None);
        assert_eq!(t.milestone, None);
        assert_eq!(t.autopilot, Autopilot::default());
        assert!(t.tags.is_empty());
        assert!(parsed.extra.is_empty());

        // The now fully-explicit rendered frontmatter still round-trips.
        let rendered = render(&parsed.ticket, &parsed.body, &parsed.extra);
        let reparsed = parse(&rendered).expect("re-parses");
        assert_eq!(reparsed.ticket, parsed.ticket);
        assert_eq!(reparsed.body, parsed.body);
    }

    #[test]
    fn unknown_frontmatter_keys_preserved() {
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-6\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Unknown keys ticket\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "custom_key: 42\n",
            "custom_nested:\n",
            "  a: 1\n",
            "  b:\n",
            "    - 2\n",
            "    - 3\n",
            "---\n",
            "Body.\n",
        );

        let parsed = parse(note).expect("unknown frontmatter keys must not fail parsing");

        let custom_key = parsed
            .extra
            .get("custom_key")
            .expect("custom_key is preserved in extra");
        assert_eq!(custom_key, &serde_yaml::Value::from(42));

        let nested = parsed
            .extra
            .get("custom_nested")
            .expect("custom_nested is preserved in extra")
            .as_mapping()
            .expect("custom_nested is still a mapping");
        assert_eq!(nested.get("a"), Some(&serde_yaml::Value::from(1)));
        let b = nested
            .get("b")
            .expect("nested list preserved")
            .as_sequence()
            .expect("still a sequence");
        assert_eq!(b.len(), 2);

        // A save (render) must still carry the unknown keys afterwards.
        let rendered = render(&parsed.ticket, &parsed.body, &parsed.extra);
        let reparsed = parse(&rendered).expect("re-parses");
        assert!(reparsed.extra.contains_key("custom_key"));
        assert!(reparsed.extra.contains_key("custom_nested"));
        assert_eq!(
            reparsed.extra.get("custom_key"),
            parsed.extra.get("custom_key")
        );
    }

    #[test]
    fn parse_accepts_an_unquoted_numeric_priority() {
        // Before `Priority`'s `Deserialize` accepted an unquoted YAML integer,
        // this exact frontmatter failed with "failed to deserialize ticket
        // fields from frontmatter: invalid type: integer `1`, expected a
        // string", and `store::collect_notes` silently dropped the whole
        // ticket from every listing with a single stderr line. This pins the
        // fix at the layer a hand-edited note actually goes through.
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-11\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Unquoted priority ticket\"\n",
            "status: todo\n",
            "priority: 1\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "Body.\n",
        );

        let parsed = parse(note).expect("an unquoted numeric priority must parse");
        assert_eq!(parsed.ticket.priority, Priority::Highest);
    }

    #[test]
    fn notes_locales_preserved_byte_exact() {
        let body = "# LOC-7 - Whitespace ticket\n\n\
             ## Objectif\n\
             Do the thing.\n\n\
             ## Notes locales\n\
             - kept a trailing space   \n\
             \tindented with a tab\n\
             \n\
             \n\
             End of notes.\n";
        let note = format!(
            "---\naidp: ticket\nkey: LOC-7\nsource: local\ntype: Task\n\
             summary: \"Whitespace ticket\"\nstatus: todo\npriority: Medium\n\
             service: aidp-viewer\ncreated: 2026-09-10T08:12:00Z\n\
             updated: 2026-09-10T08:12:00Z\n---\n{body}"
        );

        let parsed = parse(&note).expect("parses");
        assert_eq!(
            parsed.body, body,
            "the body must survive parse byte-for-byte"
        );

        let rendered = render(&parsed.ticket, &parsed.body, &parsed.extra);
        let reparsed = parse(&rendered).expect("re-parses");
        assert_eq!(
            reparsed.body, body,
            "the body must survive a render round-trip byte-for-byte"
        );
    }

    #[test]
    fn crlf_body_normalised_to_lf() {
        let note = "---\r\naidp: ticket\r\nkey: LOC-1\r\nsource: local\r\ntype: Task\r\n\
             summary: \"CRLF ticket\"\r\nstatus: todo\r\npriority: Medium\r\n\
             service: aidp-viewer\r\ncreated: 2026-09-10T08:12:00Z\r\n\
             updated: 2026-09-10T08:12:00Z\r\n---\r\n\
             # LOC-1 - CRLF ticket\r\n\r\nSome body line.\r\n\r\n## Notes locales\r\n";

        let parsed = parse(note).expect("a CRLF note parses");
        assert!(
            !parsed.body.contains('\r'),
            "body must not contain CR after ingestion: {:?}",
            parsed.body
        );
        assert_eq!(
            parsed.body,
            "# LOC-1 - CRLF ticket\n\nSome body line.\n\n## Notes locales\n"
        );

        // normalise_body also folds a lone CR (old Mac-style endings), not just CRLF.
        assert_eq!(normalise_body("a\rb\r\nc"), "a\nb\nc");
    }

    #[test]
    fn bom_stripped_on_ingest_and_never_written() {
        let note = "\u{feff}---\naidp: ticket\nkey: LOC-2\nsource: local\ntype: Task\n\
             summary: \"BOM ticket\"\nstatus: todo\npriority: Low\n\
             service: aidp-viewer\ncreated: 2026-09-10T08:12:00Z\n\
             updated: 2026-09-10T08:12:00Z\n---\nBody.\n"
            .to_string();

        let parsed = parse(&note).expect("a leading BOM must be stripped, not fail parsing");
        assert_eq!(parsed.ticket.key, "LOC-2");

        let rendered = render(&parsed.ticket, &parsed.body, &parsed.extra);
        assert!(
            !rendered.starts_with('\u{feff}'),
            "render must never emit a BOM"
        );
        assert!(
            !rendered.contains('\u{feff}'),
            "render must never emit a BOM anywhere"
        );
    }

    #[test]
    fn rejects_note_that_is_not_aidp_ticket() {
        let wrong_marker = "---\naidp: something-else\nkey: LOC-3\n---\nBody.\n";
        let err = parse(wrong_marker).unwrap_err();
        assert!(
            err.to_string().contains("ticket"),
            "error should mention the expected `aidp: ticket` marker: {err}"
        );

        let missing_marker = "---\nkey: LOC-4\nsource: local\n---\nBody.\n";
        let err = parse(missing_marker).unwrap_err();
        assert!(
            err.to_string().contains("aidp"),
            "error should mention the missing `aidp` key: {err}"
        );
    }

    #[test]
    fn parse_refuses_a_traversing_key_in_frontmatter() {
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: \"../../escaped\"\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Poisoned key\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "Body.\n",
        );

        let err = parse(note).unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "error should name the offending field: {err}"
        );
    }

    #[test]
    fn parse_refuses_a_key_carrying_wikilink_breakout_characters() {
        // The review's payload would otherwise be interpolated unescaped by
        // `index.rs::ticket_link` into the generated `_TODO.md` and break out
        // of the `[[...]]` wikilink. Quoted in the YAML so the frontmatter
        // itself stays valid — the point is that `safe_name::check` refuses
        // it, not that YAML chokes on it.
        let key_note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: \"Z]] **bold** [[W\"\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Poisoned key\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "Body.\n",
        );

        let err = parse(key_note).unwrap_err();
        assert!(
            err.to_string().contains("key"),
            "error should name the offending field: {err}"
        );

        let service_note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-12\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Poisoned service\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: \"Z]] **bold** [[W\"\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "Body.\n",
        );

        let err = parse(service_note).unwrap_err();
        assert!(
            err.to_string().contains("service"),
            "error should name the offending field: {err}"
        );
    }

    #[test]
    fn parse_refuses_a_traversing_service_in_frontmatter() {
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-9\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Poisoned service\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: \"../..\"\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "Body.\n",
        );

        let err = parse(note).unwrap_err();
        assert!(
            err.to_string().contains("service"),
            "error should name the offending field: {err}"
        );
    }

    #[test]
    fn body_with_a_yaml_fence_is_not_mistaken_for_frontmatter() {
        let note = concat!(
            "---\n",
            "aidp: ticket\n",
            "key: LOC-5\n",
            "source: local\n",
            "type: Task\n",
            "summary: \"Fence ticket\"\n",
            "status: todo\n",
            "priority: Medium\n",
            "service: aidp-viewer\n",
            "created: 2026-09-10T08:12:00Z\n",
            "updated: 2026-09-10T08:12:00Z\n",
            "---\n",
            "# LOC-5 - Fence ticket\n",
            "\n",
            "Example frontmatter for another tool:\n",
            "\n",
            "```yaml\n",
            "---\n",
            "not: frontmatter\n",
            "---\n",
            "```\n",
            "\n",
            "## Notes locales\n",
        );

        let parsed = parse(note).expect("the real frontmatter is the first --- ... --- block only");
        assert_eq!(parsed.ticket.key, "LOC-5");
        assert!(
            parsed
                .body
                .contains("```yaml\n---\nnot: frontmatter\n---\n```"),
            "the body's own --- fence must survive untouched: {:?}",
            parsed.body
        );
    }

    fn minimal_note(status: &str) -> String {
        format!(
            concat!(
                "---\n",
                "aidp: ticket\n",
                "key: LOC-8\n",
                "source: local\n",
                "type: Task\n",
                "summary: \"Minimal ticket\"\n",
                "status: {}\n",
                "priority: Medium\n",
                "service: aidp-viewer\n",
                "created: 2026-09-10T08:12:00Z\n",
                "updated: 2026-09-10T08:12:00Z\n",
                "---\n",
                "# LOC-8 - Minimal ticket\n",
            ),
            status
        )
    }

    #[test]
    fn peek_status_matches_full_parse() {
        for status in ["todo", "queued", "in-progress", "done", "blocked"] {
            let note = minimal_note(status);
            let peeked = peek_status(&note)
                .unwrap_or_else(|| panic!("peek_status must recognise status `{status}`"));
            let parsed = parse(&note)
                .unwrap_or_else(|e| panic!("full parse of `{status}` must succeed: {e}"));
            assert_eq!(
                peeked, parsed.ticket.status,
                "peek_status must agree with the full parse for `{status}`"
            );
        }
    }

    #[test]
    fn peek_status_is_none_for_garbage() {
        assert_eq!(peek_status(""), None);
        assert_eq!(peek_status("not frontmatter at all"), None);
        assert_eq!(peek_status("---\nno closing delimiter\n"), None);
        assert_eq!(
            peek_status("---\naidp: ticket\nno_status_key: true\n---\nbody\n"),
            None,
            "a frontmatter with no status key must return None, not panic"
        );
        assert_eq!(
            peek_status("---\naidp: ticket\nstatus: not-a-real-status\n---\nbody\n"),
            None,
            "an unparseable status value must return None, not panic"
        );
    }
}
