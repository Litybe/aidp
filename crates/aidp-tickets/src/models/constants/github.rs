//! Names and defaults of the GitHub issue source: config keys, blocked reasons, sync state.

/// Default prefix of a mirror key: `GH-<number>`.
pub const DEFAULT_KEY_PREFIX: &str = "GH";

/// Default prefix of the state labels (`aidp:ready`, `aidp:done`…).
pub const DEFAULT_LABEL_PREFIX: &str = "aidp:";

/// Default minutes between two daemon syncs; `0` turns the periodic sync off.
pub const DEFAULT_SYNC_MINUTES: u32 = 10;

/// Default cap on the open issues read per repository and sync.
pub const DEFAULT_LIMIT: u32 = 200;

/// Separator between the owner and the name of a repository.
pub const REPO_SEPARATOR: char = '/';

/// Separator between the prefix and the number of a mirror key.
pub const KEY_SEPARATOR: char = '-';

/// Separator between a status and a blocked reason in a `status_labels` key.
pub const STATUS_REASON_SEPARATOR: char = ':';

/// Blocked reason of an issue with no `CA-n` acceptance criterion.
pub const NEEDS_REFINE_REASON: &str = "needs-refine";

/// Blocked reason of an issue no routing rule matched.
pub const NEEDS_ROUTING_REASON: &str = "needs-routing";

/// Blocked reason of an issue labelled skip: never dispatched.
pub const SKIP_REASON: &str = "skip";

/// Blocked reason of a complete issue still waiting for its ready label.
pub const NOT_READY_REASON: &str = "not-ready";

/// Prefix of the blocked reason that hands the ticket to the state label it carried on GitHub.
pub const LABEL_REASON_PREFIX: &str = "label:";

/// Pattern of an acceptance criterion id in an issue body.
pub const ACCEPTANCE_CRITERION_PATTERN: &str = r"\bCA-\d+\b";

/// Pattern of a blocker sentence: the issue references that follow it on the line block the issue.
pub const BLOCKER_SENTENCE_PATTERN: &str =
    r"(?i)(d[ée]pend(?:s|ent)?\s+(?:de|du|des|on|of)|blocked\s+by|bloqu[ée]e?s?\s+par)";

/// Pattern of one issue reference: `#12` or `owner/name#12`.
pub const ISSUE_REFERENCE_PATTERN: &str = r"(?:([\w.-]+/[\w.-]+))?#(\d+)\b";

/// Pattern of a closing keyword for issue `{n}` in a pull request body.
pub const CLOSING_KEYWORD_PATTERN: &str = r"(?i)\b(close[sd]?|fix(e[sd])?|resolve[sd]?)\s+#{n}\b";

/// Closing line appended to a pull request body that does not close its issue yet.
pub const CLOSES_PREFIX: &str = "Closes #";

/// Expectation message for a regex literal valid by construction.
pub const VALID_REGEX: &str = "valid regex literal";

/// Label prefix of a priority label: `prio:high`.
pub const PRIORITY_LABEL_PREFIX: &str = "prio:";

/// Label that makes a mirror a `Bug`.
pub const BUG_LABEL: &str = "bug";

/// File, under `TODO/.state/`, the last GitHub sync report is written to.
pub const SYNC_STATE_FILE: &str = "github-sync.json";

/// Heading of the local notes section that every update preserves.
pub const NOTES_LOCALES_HEADING: &str = "## Notes locales";

/// Journal reason of a mirror whose issue was closed without being delivered.
pub const CLOSED_UNDONE_REASON: &str = "github: issue fermée sans être faite";

/// Journal reason of a mirror whose issue was closed while its PR waited.
pub const CLOSED_REASON: &str = "github: issue fermée";

/// Journal reason prefix of a transition the sync derived from a label change.
pub const LABEL_CHANGED_PREFIX: &str = "github: label";

/// Journal reason of a transition the sync derived from the issue gates (route, criteria, skip).
pub const GATE_REASON_PREFIX: &str = "github:";

/// Most dependency reads in flight at once.
pub const DEPENDENCY_THREADS: usize = 6;

/// Opening of the marker a bug ticket carries in its body: `<!-- aidp:bug parent=272 base=feat/GH-272 -->`.
pub const BUG_MARKER_PATTERN: &str = r"<!--\s*aidp:bug\s+([^>]*?)\s*-->";

/// Key of the parent issue number inside the bug marker.
pub const BUG_MARKER_PARENT_KEY: &str = "parent";

/// Key of the base branch inside the bug marker.
pub const BUG_MARKER_BASE_KEY: &str = "base";

/// Most bug tickets one parent issue may spawn; the next failed test blocks the parent.
pub const MAX_BUGS_PER_PARENT: usize = 3;

/// Most characters of the summary in the title of a bug issue.
pub const BUG_SUMMARY_MAX_CHARS: usize = 100;

/// Summary used when the failed test gave none.
pub const BUG_DEFAULT_SUMMARY: &str = "test en échec";

/// Label family that tells the routing which route a bug takes: `bug:code`, `bug:art`, `bug:map`.
pub const BUG_ROUTE_LABEL_PREFIX: &str = "bug:";
