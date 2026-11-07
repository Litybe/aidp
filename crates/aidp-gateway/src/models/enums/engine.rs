use crate::models::constants::engine::{
    ENGINE_NATIVE, ENGINE_PROMPT, ENGINE_V7_ALIAS, ENGINE_WORKFLOW,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Engine a launch runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    V7,
    Workflow,
    Native,
    Prompt,
}

impl Engine {
    /// The engine name carried by a start request.
    pub const fn as_str(self) -> &'static str {
        match self {
            Engine::V7 => ENGINE_V7_ALIAS,
            Engine::Workflow => ENGINE_WORKFLOW,
            Engine::Native => ENGINE_NATIVE,
            Engine::Prompt => ENGINE_PROMPT,
        }
    }

    /// The engine a request names; the legacy `v7` spelling means `Workflow`.
    pub fn parse(name: &str) -> Option<Engine> {
        match name {
            ENGINE_NATIVE => Some(Engine::Native),
            ENGINE_WORKFLOW | ENGINE_V7_ALIAS => Some(Engine::Workflow),
            ENGINE_PROMPT => Some(Engine::Prompt),
            _ => None,
        }
    }
}

impl Serialize for Engine {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Engine {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Engine::parse(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown engine: {name}")))
    }
}
