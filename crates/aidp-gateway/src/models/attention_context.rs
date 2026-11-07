use aidp_integrations::models::notifications_pr_config::NotificationsPrConfig;

/// Everything besides the fetched data that attention processing reads.
pub struct AttentionContext<'a> {
    pub config: &'a NotificationsPrConfig,
    pub bot_authors: &'a [String],
}
