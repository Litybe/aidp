/// JSON key naming the ticket in command outputs.
pub const KEY_FIELD: &str = "key";

/// JSON key carrying the note's path in command outputs.
pub const PATH_FIELD: &str = "path";

/// JSON key carrying the archived count in `aidp ticket reindex`'s output.
pub const ARCHIVED_FIELD: &str = "archived";

/// JSON key, always present, listing every index file that could not be written.
pub const WARNINGS_FIELD: &str = "warnings";

/// JSON key carrying the previous status in a transition output.
pub const FROM_FIELD: &str = "from";

/// JSON key carrying the new status in a transition output.
pub const TO_FIELD: &str = "to";

/// JSON key carrying the ticket in `aidp ticket show`'s output.
pub const TICKET_FIELD: &str = "ticket";

/// JSON key carrying the ticket body in `aidp ticket show`'s output.
pub const BODY_FIELD: &str = "body";

/// JSON key carrying the routed count in `route --from-unrouted`'s output.
pub const ROUTED_FIELD: &str = "routed";

/// JSON key carrying the count of tickets left in place.
pub const LEFT_FIELD: &str = "left";

/// JSON key carrying the routed ticket keys.
pub const KEYS_FIELD: &str = "keys";

/// JSON key carrying the per-ticket error messages.
pub const ERRORS_FIELD: &str = "errors";

/// Reserved service name under which unroutable tickets are parked.
pub const UNROUTED_SERVICE: &str = "unrouted";

/// Reason recorded when `aidp ticket cancel` is given none.
pub const DEFAULT_CANCEL_REASON: &str = "cancelled-by-user";

/// Flag appended to a `list --format text` line for a ticket without template.
pub const NO_TEMPLATE_FLAG: &str = " [no-template]";

/// Placeholder printed when a journal event has no previous status.
pub const NO_PREVIOUS_STATUS: &str = "-";

/// Separator between the values of a comma-separated argument.
pub const CSV_SEPARATOR: char = ',';

/// Error raised when `route` is given neither `--from-unrouted` nor KEY and `--service`.
pub const ROUTE_ARGS_REQUIRED: &str =
    "`route` requires either --from-unrouted, or both a KEY and --service";

/// Error raised when `--from-unrouted` is combined with a KEY or `--service`.
pub const FROM_UNROUTED_CONFLICT: &str = "--from-unrouted cannot be combined with a KEY or --service: pass --from-unrouted alone, or a KEY and --service without it";

/// Stderr warning when no skill root is configured for `plan`.
pub const SKILL_ROOT_UNSET_WARNING: &str =
    "aidp ticket plan: AIDP_SKILL_ROOT is not set; using an empty template catalog";

/// Builds the error for a verb the gateway client in `aidp-cli` should have intercepted.
pub fn verb_intercepted(verb: &str) -> String {
    format!("`aidp ticket {verb}` is served by the gateway client in aidp-cli; reaching aidp_tickets::run means the interception in main.rs was removed")
}

/// Builds the stderr warning for an unreadable skill root.
pub fn skill_root_unreadable_warning(root: &str) -> String {
    format!("aidp ticket plan: AIDP_SKILL_ROOT ({root}) is not a readable directory;                  using an empty template catalog")
}

/// Status token `todo`.
pub const STATUS_TODO: &str = "todo";

/// Status token `queued`.
pub const STATUS_QUEUED: &str = "queued";

/// Status token `in-progress`.
pub const STATUS_IN_PROGRESS: &str = "in-progress";

/// Snake-case spelling of `in-progress`.
pub const STATUS_IN_PROGRESS_SNAKE: &str = "in_progress";

/// Flat spelling of `in-progress`.
pub const STATUS_IN_PROGRESS_FLAT: &str = "inprogress";

/// Status token `in-review`.
pub const STATUS_IN_REVIEW: &str = "in-review";

/// Snake-case spelling of `in-review`.
pub const STATUS_IN_REVIEW_SNAKE: &str = "in_review";

/// Flat spelling of `in-review`.
pub const STATUS_IN_REVIEW_FLAT: &str = "inreview";

/// Status token `done`.
pub const STATUS_DONE: &str = "done";

/// Status token `blocked`.
pub const STATUS_BLOCKED: &str = "blocked";

/// Status token `cancelled`.
pub const STATUS_CANCELLED: &str = "cancelled";

/// American spelling of `cancelled`.
pub const STATUS_CANCELED: &str = "canceled";

/// Status list quoted by the unknown-status error.
pub const STATUS_EXPECTED: &str = "todo/queued/in-progress/in-review/done/blocked/cancelled";

/// Source token `github`.
pub const SOURCE_GITHUB: &str = "github";

/// Source token `local`.
pub const SOURCE_LOCAL: &str = "local";

/// Source list quoted by the unknown-source error.
pub const SOURCE_EXPECTED: &str = "github/local";

/// Lowercase issue type name `story`.
pub const ISSUE_STORY: &str = "story";

/// Lowercase issue type name `task`.
pub const ISSUE_TASK: &str = "task";

/// Lowercase issue type name `bug`.
pub const ISSUE_BUG: &str = "bug";

/// Lowercase issue type name `spike`.
pub const ISSUE_SPIKE: &str = "spike";

/// Help text of `aidp ticket new`.
pub const HELP_NEW: &str = "Create a new local ticket. Body precedence: `--body-file` > `--body` > `--stdin` > the default template. Prints `{key, path, warnings}`";

/// Help text of `aidp ticket render`.
pub const HELP_RENDER: &str = "Print a ticket's body exactly as it will be passed as the `prompt` input (lot 9a) — no frontmatter, no added heading";

/// Help text of `aidp ticket edit`.
pub const HELP_EDIT: &str = "Edit a ticket's frontmatter (never the body). `--service` delegates to the same code path as `route`";

/// Help text of `aidp ticket status`.
pub const HELP_STATUS: &str = "Manual status transition, actor=human, journalled. Refused explicitly (non-zero exit) if illegal";

/// Help text of `aidp ticket route`.
pub const HELP_ROUTE: &str = "Move a ticket to another service (and, optionally, set its template), or — with `--from-unrouted` — re-route the whole `unrouted/` backlog in one non-interactive batch. Exactly one of \"a KEY and --service\" or `--from-unrouted` is required; see [`route_mode`] for the exact validation";

/// Help text of `aidp ticket reindex`.
pub const HELP_REINDEX: &str = "Regenerate `_TODO.md` and every `_<service>.md`, sweeping old done/cancelled tickets into `archive/` first";

/// Help text of `aidp ticket start`.
pub const HELP_START: &str = "Start a dispatcher-driven pipeline run for one ticket right now, bypassing the scheduler. Declared here only so `clap` renders its help and parses its arguments — this crate stays HTTP-free by decision (no `ureq`, no `aidp-gateway` dependency), so `aidp-cli`'s `main.rs` (which already depends on both crates) intercepts `aidp ticket start` and calls the gateway client itself before [`crate::run`] is ever invoked. Reaching the `Start` arm in `run_inner` means that interception in `main.rs` was removed";

/// Help text of `aidp ticket plan`.
pub const HELP_PLAN: &str = "What the dispatcher would launch right now, computed LOCALLY with no running daemon: opens the store directly, lists `todo` + `queued` tickets, builds a [`plan::TemplateCatalog`] from the injected skill root (an empty catalog, with a stderr warning, when it is absent or unreadable), and calls [`plan::plan`] with an EMPTY `running` slice and [`plan::Verdict::unconstrained`] — a CLI invocation outside the daemon cannot see the daemon's running sessions or its budget verdict, so this answers \"what would launch if nothing else were running and the budget were unconstrained\", not \"what will the daemon do on its next tick\". For the daemon-aware answer, including real occupancy and the real budget, use `aidp gateway dispatcher status` instead. Prints the resulting [`plan::DispatchPlan`]";

/// Help text of `aidp ticket dispatch`.
pub const HELP_DISPATCH: &str = "Ask the running daemon to run one dispatch tick now, instead of waiting for its next scheduled tick. Declared here only so `clap` renders its help and parses its arguments — like [`Op::Start`], this is served by `aidp-cli`'s `main.rs` (via the gateway client) and never reaches [`crate::run`]. Reaching the `Dispatch` arm in `run_inner` means that interception in `main.rs` was removed. For the local, no-daemon preview of what a tick would launch, use [`Op::Plan`] instead";

/// Help text of `aidp ticket budget`.
pub const HELP_BUDGET: &str = "Explain the token-budget verdict the daemon computed on its last tick: windows (utilization and reset instant), thresholds, reserve (learned or configured), sessions not yet reflected in the snapshot, and the resulting verdict. Declared here only so `clap` renders its help and parses its arguments — like [`Op::Start`] and [`Op::Dispatch`], this verb is served by `aidp-cli`'s `main.rs` through the gateway client and never reaches [`crate::run`], because this crate is deliberately HTTP-free";

/// Help text of the `--reason` filter of `aidp ticket list`.
pub const HELP_LIST_REASON: &str = "Keep only tickets whose `autopilot.blocked_reason` equals this value, e.g. `needs-routing`. Applied after the store's own filters, in this file — `ListFilter` itself gains no field for it";

/// Help text of the `--from-unrouted` flag of `aidp ticket route`.
pub const HELP_ROUTE_FROM_UNROUTED: &str = "Non-interactive batch: re-route every ticket parked under `service: unrouted` against the current routing rules, moving and unblocking every one that now matches. Mutually exclusive with KEY/`--service`";
