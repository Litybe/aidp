/// Resident threads of the flow engine, named for the thread guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowWorker {
    Action,
    Timer,
    Bus,
}

impl FlowWorker {
    /// The loop name the thread guard reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            FlowWorker::Action => "flows.action",
            FlowWorker::Timer => "flows.timer",
            FlowWorker::Bus => "flows.bus",
        }
    }
}
