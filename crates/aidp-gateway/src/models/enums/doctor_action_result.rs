/// How an allowed action of the doctor ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorActionResult {
    Done(String),
    Failed(String),
    /// Its precondition does not hold: nothing was changed.
    Refused(String),
}
