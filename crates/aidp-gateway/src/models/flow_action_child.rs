use super::flow_action_process::FlowActionProcess;
use std::sync::{Arc, Mutex};

pub struct FlowActionChild {
    pub(crate) process: Arc<FlowActionProcess>,
    pub(crate) children: Arc<Mutex<Vec<Arc<FlowActionProcess>>>>,
}
