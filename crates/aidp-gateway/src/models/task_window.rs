/// Run window of one task: its start tick and, once released, its end tick.
pub struct TaskWindow {
    pub id: String,
    pub started: u64,
    pub ended: Option<u64>,
}
