//! Names, limits and field table of the settings routes.

use crate::models::enums::FieldKind;
use crate::models::setting_field::SettingField;

/// First path segment of the routes.
pub const SETTINGS_SEGMENT: &str = "settings";
/// Directory of the dispatcher files inside the vault.
pub const TODO_DIR: &str = "TODO";
/// Dispatcher configuration file.
pub const CONFIG_FILE: &str = "config.yaml";
/// Routing file.
pub const ROUTING_FILE: &str = "routing.yaml";
/// Event published after a settings file changed.
pub const EVENT_SETTINGS_CHANGED: &str = "settings.changed";
/// Key of the changed section in the event.
pub const EVENT_SECTION_KEY: &str = "section";
/// Extension suffix of the file written before the rename.
pub const TEMP_SUFFIX: &str = "settings-tmp";

pub const KEY_MODELS: &str = "models";
pub const KEY_DEFAULTS: &str = "defaults";
pub const KEY_RULES: &str = "rules";
pub const KEY_REPOS: &str = "repos";
pub const KEY_MATCH: &str = "match";
pub const KEY_SERVICE: &str = "service";
pub const KEY_ROUTE: &str = "route";
pub const KEY_TRACK: &str = "track";
pub const KEY_TEMPLATE: &str = "template";
pub const KEY_FLOW: &str = "flow";

/// Sections of `routing.yaml` a `PUT` may carry.
pub const ROUTING_SECTIONS: [&str; 3] = [KEY_DEFAULTS, KEY_RULES, KEY_REPOS];
/// Keys of a `defaults` object.
pub const DEFAULTS_KEYS: [&str; 1] = [KEY_TRACK];
/// Keys of a `repos` entry.
pub const REPO_KEYS: [&str; 4] = [KEY_TEMPLATE, KEY_TRACK, KEY_FLOW, KEY_ROUTE];
/// Criteria of a rule that hold one text.
pub const CRITERIA_TEXT_KEYS: [&str; 6] = [
    "component",
    "label",
    "epic",
    "key_prefix",
    "summary_regex",
    "issuetype",
];
/// Criteria of a rule that hold a list of texts.
pub const CRITERIA_LIST_KEYS: [&str; 2] = ["labels", "keywords"];
/// Key of the regular expression criterion.
pub const CRITERIA_REGEX_KEY: &str = "summary_regex";

/// Stage ids a model policy usually names.
pub const MODEL_STAGES: [&str; 16] = [
    "prepare",
    "plan",
    "implement",
    "repro",
    "review",
    "verdict",
    "fix",
    "test",
    "report",
    "brief",
    "generate",
    "animate",
    "import",
    "graph",
    "rooms",
    "write",
];
/// Model tiers a policy may use.
pub const MODEL_TIERS: [&str; 4] = ["haiku", "sonnet", "opus", "fable"];

/// Most dispatcher slots a settings update accepts.
pub const MAX_SLOTS: u64 = 32;
/// Longest window, in minutes, after a reset that still counts as "just after".
pub const MAX_HEAVY_WINDOW_MINUTES: u64 = 300;
/// Highest percentage of `stop_at_7d_percent`.
pub const MAX_PERCENT: f64 = 100.0;

/// Accepted `budget.unknown_policy` words.
pub const UNKNOWN_POLICIES: &[&str] = &["conservative", "allow", "block"];

/// Every leaf `PUT /api/settings/budget` may set.
pub const BUDGET_FIELDS: &[SettingField] = &[
    SettingField {
        path: &["budget", "max_utilization_5h"],
        kind: FieldKind::Fraction,
    },
    SettingField {
        path: &["budget", "max_utilization_7d"],
        kind: FieldKind::Fraction,
    },
    SettingField {
        path: &["budget", "reserve_per_run_5h"],
        kind: FieldKind::UnitInterval,
    },
    SettingField {
        path: &["budget", "reserve_per_run_7d"],
        kind: FieldKind::UnitInterval,
    },
    SettingField {
        path: &["budget", "unknown_policy"],
        kind: FieldKind::Choice(UNKNOWN_POLICIES),
    },
    SettingField {
        path: &["budget", "learn"],
        kind: FieldKind::Bool,
    },
    SettingField {
        path: &["budget", "per_ticket", "code"],
        kind: FieldKind::Tokens,
    },
    SettingField {
        path: &["budget", "per_ticket", "art"],
        kind: FieldKind::Tokens,
    },
    SettingField {
        path: &["budget", "per_ticket", "map"],
        kind: FieldKind::Tokens,
    },
    SettingField {
        path: &["budget", "per_ticket", "doc"],
        kind: FieldKind::Tokens,
    },
    SettingField {
        path: &["budget", "per_ticket", "default"],
        kind: FieldKind::Tokens,
    },
    SettingField {
        path: &["budget", "pixellab", "min_reserve"],
        kind: FieldKind::NonNegative,
    },
    SettingField {
        path: &["budget", "pixellab", "per_ticket"],
        kind: FieldKind::Count(0, u32::MAX as u64),
    },
    SettingField {
        path: &["dispatcher", "slots"],
        kind: FieldKind::Count(1, MAX_SLOTS),
    },
    SettingField {
        path: &["dispatcher", "test_slots"],
        kind: FieldKind::Count(1, MAX_SLOTS),
    },
    SettingField {
        path: &["dispatcher", "bug_slots"],
        kind: FieldKind::Count(1, MAX_SLOTS),
    },
    SettingField {
        path: &["dispatcher", "stop_at_7d_percent"],
        kind: FieldKind::OptionalPercent,
    },
    SettingField {
        path: &["dispatcher", "night", "enabled"],
        kind: FieldKind::Bool,
    },
    SettingField {
        path: &["dispatcher", "night", "start"],
        kind: FieldKind::ClockTime,
    },
    SettingField {
        path: &["dispatcher", "night", "end"],
        kind: FieldKind::ClockTime,
    },
    SettingField {
        path: &["dispatcher", "night", "heavy_after_reset"],
        kind: FieldKind::Bool,
    },
    SettingField {
        path: &["dispatcher", "night", "heavy_window_minutes"],
        kind: FieldKind::Count(1, MAX_HEAVY_WINDOW_MINUTES),
    },
];

/// Reply key: the change needs a daemon restart to apply.
pub const KEY_RELOAD_REQUIRED: &str = "reloadRequired";
/// Reply key: the file was rewritten.
pub const KEY_CHANGED: &str = "changed";
/// Tracks a routing entry may use.
pub const TRACK_NAMES: &str = "quick, standard, deep, bug";
