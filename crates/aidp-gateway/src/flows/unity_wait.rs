//! The Unity compile gate and the longer wait the doctor may grant one flow run.

use super::actions::{skipped_when_flagged, waiting_when_flagged};
use super::registry::{ActionContext, FlowAction, SubprocessAction};
use crate::models::constants::flow_actions::{
    FLAG_WAIT_MULTIPLIER, UNITY_REFRESH_TIMEOUT_SECONDS, VERB_FLOW, VERB_FLOW_UNITY_REFRESH,
};
use crate::sync::lock;
use aidp_flows::ActionOutcome;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

/// Multiplier of the compile gate wait, by flow key; a flow without an entry waits the default.
#[derive(Default)]
pub struct UnityWaitExtensions {
    multipliers: Mutex<HashMap<String, u32>>,
}

impl UnityWaitExtensions {
    /// Grants flow `key` a wait `multiplier` times longer for its next compile gates.
    pub fn extend(&self, key: &str, multiplier: u32) {
        lock(&self.multipliers).insert(key.to_string(), multiplier.max(1));
    }

    /// The multiplier of flow `key`, 1 when none was granted.
    pub fn multiplier(&self, key: &str) -> u32 {
        lock(&self.multipliers).get(key).copied().unwrap_or(1)
    }
}

/// `unity.refresh`: one compile gate of the open editor, with the wait the flow was granted.
pub struct UnityRefreshAction {
    extensions: std::sync::Arc<UnityWaitExtensions>,
}

impl UnityRefreshAction {
    pub fn new(extensions: std::sync::Arc<UnityWaitExtensions>) -> Self {
        UnityRefreshAction { extensions }
    }
}

impl FlowAction for UnityRefreshAction {
    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        let multiplier = self.extensions.multiplier(context.key);
        let mut args = vec![VERB_FLOW.to_string(), VERB_FLOW_UNITY_REFRESH.to_string()];
        if multiplier > 1 {
            args.push(FLAG_WAIT_MULTIPLIER.to_string());
            args.push(multiplier.to_string());
        }
        let timeout = Duration::from_secs(UNITY_REFRESH_TIMEOUT_SECONDS * u64::from(multiplier));
        let gate = SubprocessAction::new(args).timeout(timeout);
        waiting_when_flagged(skipped_when_flagged(gate.run(context, with)))
    }
}
