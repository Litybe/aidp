/// The live state of the run or session behind a running pipeline step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorLiveRun {
    Running,
    /// Ended at that RFC 3339 time, when known.
    Ended(Option<String>),
    /// No run, session or process answers for it.
    Gone,
}
