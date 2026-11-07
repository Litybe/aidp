//! Loading of the optional `notifications.pr` key of `~/.aidp/config.json`.

use crate::models::constants::notifications::{NOTIFICATIONS_KEY, PR_KEY};
use crate::models::constants::squad::{CONFIG_FILE_DIR, CONFIG_FILE_NAME};
use crate::models::notifications_pr_config::NotificationsPrConfig;
use std::path::Path;

/// The PR notification toggles of the current user; all on when absent or unreadable.
pub fn load_notifications_pr() -> NotificationsPrConfig {
    aidp_core::vault::home_dir()
        .map(|home| load_notifications_pr_from(&home.join(CONFIG_FILE_DIR).join(CONFIG_FILE_NAME)))
        .unwrap_or_default()
}

/// The PR notification toggles stored in `path`; all on when the file or key is missing or malformed.
pub fn load_notifications_pr_from(path: &Path) -> NotificationsPrConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|root| root.get(NOTIFICATIONS_KEY)?.get(PR_KEY).cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../tests/unit/notifications_tests.rs"]
mod notifications_tests;
