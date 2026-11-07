use std::time::Duration;

use serde_json::{Map, Value};

/// How one session behaves; the default writes every declared output and an accepted reply.
#[derive(Debug, Clone, Default)]
pub struct Behaviour {
    pub signals: Map<String, Value>,
    /// Files written under the workspace, relative to it.
    pub workspace_writes: Vec<(String, String)>,
    /// Text written to `reply.md` instead of a valid envelope.
    pub raw_reply: Option<String>,
    /// The session stays live this long after its file writes, before it writes its reply.
    pub hold: Option<Duration>,
    /// The session never ends and writes no reply.
    pub hang: bool,
}

impl Behaviour {
    pub fn ok() -> Self {
        Self::default()
    }

    pub fn hang() -> Self {
        Behaviour {
            hang: true,
            ..Self::default()
        }
    }

    pub fn raw(text: &str) -> Self {
        Behaviour {
            raw_reply: Some(text.to_string()),
            ..Self::default()
        }
    }

    pub fn signal(mut self, key: &str, value: Value) -> Self {
        self.signals.insert(key.to_string(), value);
        self
    }

    pub fn write(mut self, relative_path: &str, contents: &str) -> Self {
        self.workspace_writes
            .push((relative_path.to_string(), contents.to_string()));
        self
    }
}

impl Behaviour {
    /// Keeps the session live for `duration` once its files are written.
    #[allow(dead_code)]
    pub fn hold(mut self, duration: Duration) -> Self {
        self.hold = Some(duration);
        self
    }
}
