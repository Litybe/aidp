use serde::{Deserialize, Serialize};

/// How the dispatcher launches a queued ticket: a pipeline session or a flow
/// run. Read from `dispatcher.launch`; absent means `session`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DispatcherLaunch {
    #[default]
    Session,
    Flow,
}
