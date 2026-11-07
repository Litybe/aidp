use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const CI_HOOK_INFIX: &str = "ci hook";
pub const ACTIVE_RUN_MARKER: &str = ".aidp/active-run";
pub const SCOPE_CHECK_MARKER: &str = "aidp scope check";
pub const RUN_SUMMARY_MARKER: &str = "aidp run summary";
const GUARD_MARKERS: &[&str] = &[ACTIVE_RUN_MARKER, SCOPE_CHECK_MARKER, RUN_SUMMARY_MARKER];

/// One hook command. Fields other than `type` and `command` are preserved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookCommand {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A matcher and the commands it runs for one hook event.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookMatcherEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(default)]
    pub hooks: Vec<HookCommand>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Claude Code settings: the hooks map, plus every other key kept untouched.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HookSettings {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hooks: BTreeMap<String, Vec<HookMatcherEntry>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Idempotent merge of `source` hooks into `target`. Returns the hooks newly added.
pub fn merge_hooks(target: &mut HookSettings, source: &HookSettings) -> usize {
    let mut added = 0usize;
    for (event, source_entries) in &source.hooks {
        let target_entries = target.hooks.entry(event.clone()).or_default();
        for source_entry in source_entries {
            if has_aidp_hook(target_entries, source_entry) {
                continue;
            }
            target_entries.push(source_entry.clone());
            added += 1;
        }
    }
    added
}

/// True when `existing` holds the same aidp hook as `candidate`: equal matcher plus a shared
/// command, CI hook kind or guard marker. A CI candidate never matches a guard hook.
pub fn has_aidp_hook(existing: &[HookMatcherEntry], candidate: &HookMatcherEntry) -> bool {
    let candidate_matcher = matcher_of(candidate);
    let candidate_commands = entry_commands(candidate);
    let candidate_ci_kinds = ci_hook_kinds(&candidate_commands);
    let candidate_has_marker = entry_has_marker(candidate);

    existing
        .iter()
        .filter(|entry| matcher_of(entry) == candidate_matcher)
        .any(|entry| {
            let commands = entry_commands(entry);
            if commands.iter().any(|c| candidate_commands.contains(c)) {
                return true;
            }
            let ci_kinds = ci_hook_kinds(&commands);
            if !candidate_ci_kinds.is_empty()
                && ci_kinds.iter().any(|k| candidate_ci_kinds.contains(k))
            {
                return true;
            }
            candidate_ci_kinds.is_empty() && candidate_has_marker && entry_has_marker(entry)
        })
}

fn matcher_of(entry: &HookMatcherEntry) -> &str {
    entry.matcher.as_deref().unwrap_or("")
}

/// Trimmed commands of an entry, skipping hooks without one.
pub fn entry_commands(entry: &HookMatcherEntry) -> Vec<String> {
    entry
        .hooks
        .iter()
        .filter_map(|hook| hook.command.as_deref())
        .map(|command| command.trim().to_string())
        .collect()
}

/// `aidp ci hook stop` gives `stop`. Empty for anything that is not a CI hook.
pub fn ci_hook_kinds(commands: &[String]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|command| {
            let index = command.find(CI_HOOK_INFIX)?;
            command[index + CI_HOOK_INFIX.len()..]
                .split_whitespace()
                .next()
                .map(str::to_string)
        })
        .collect()
}

/// True when any command of the entry carries a guard marker.
pub fn entry_has_marker(entry: &HookMatcherEntry) -> bool {
    entry
        .hooks
        .iter()
        .filter_map(|hook| hook.command.as_deref())
        .any(|command| GUARD_MARKERS.iter().any(|marker| command.contains(marker)))
}

#[cfg(test)]
#[path = "../tests/unit/hooks_tests.rs"]
mod tests;
