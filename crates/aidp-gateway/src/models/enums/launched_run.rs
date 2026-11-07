use crate::sessions::Launch;

/// What a launch produced: a native run, or a legacy session.
pub enum LaunchedRun {
    Native { run_id: String },
    Session(Launch),
}

impl LaunchedRun {
    /// The native run id, when the launch became a native run.
    pub fn run_id(&self) -> Option<&str> {
        match self {
            LaunchedRun::Native { run_id } => Some(run_id),
            LaunchedRun::Session(_) => None,
        }
    }

    /// The session id, when the launch became a session.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            LaunchedRun::Native { .. } => None,
            LaunchedRun::Session(launch) => Some(launch.id()),
        }
    }
}
