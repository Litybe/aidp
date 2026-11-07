use crate::models::task_window::TaskWindow;
use aidp_core::plan::TaskStatus;
use std::collections::{HashMap, HashSet};

/// Running ids, settled outcomes, and per-task run windows on one shared clock.
pub struct PlanSlotState {
    pub running: HashSet<String>,
    pub settled: HashMap<String, TaskStatus>,
    pub windows: Vec<TaskWindow>,
    pub clock: u64,
}
