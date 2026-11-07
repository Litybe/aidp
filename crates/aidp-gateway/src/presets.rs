//! Launch presets: saved session-start configurations replayed by the CLI or the viewer.
//! File plumbing mirrors `projects.rs`; upsert semantics mirror `automations.rs::upsert`.

use crate::bus::Bus;
use crate::ids::new_id;
use crate::launcher::RunLauncher;
use crate::models::enums::engine::Engine;
use crate::models::launched_run::LaunchedRun;
use crate::sessions::{Origin, StartRequest};
use crate::store::{read_json, write_json};
use crate::sync::lock;
use crate::time::now_iso;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PRESETS_FILE: &str = "presets.json";
const ORIGIN_KIND_PRESET: &str = "preset";
const EVENT_PRESET_SAVED: &str = "preset.saved";
const NOT_FOUND_MESSAGE: &str = "preset not found";
const KEY_ID: &str = "id";
const KEY_NAME: &str = "name";
const KEY_PROJECT_ID: &str = "projectId";
const KEY_TEMPLATE_ID: &str = "templateId";
const KEY_TARGET: &str = "target";
const KEY_INPUTS: &str = "inputs";
const KEY_PERMISSION_MODE: &str = "permissionMode";
const KEY_MODEL: &str = "model";
const KEY_EFFORT: &str = "effort";
const KEY_ENGINE: &str = "engine";
const KEY_AUTONOMOUS: &str = "autonomous";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub project_id: String,
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Absent means native; "v7" or "workflow" keeps the legacy session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    /// Auto-approves every gate of the native run.
    #[serde(default)]
    pub autonomous: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct Presets {
    file: PathBuf,
    list: Mutex<Vec<Preset>>,
    bus: Arc<Bus>,
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

impl Presets {
    pub fn load(dir: &Path, bus: Arc<Bus>) -> Self {
        let file = dir.join(PRESETS_FILE);
        let list: Vec<Preset> = read_json(&file, Vec::new());
        Presets {
            file,
            list: Mutex::new(list),
            bus,
        }
    }

    fn save(&self, list: &[Preset]) {
        if let Err(e) = write_json(&self.file, &list) {
            tracing::warn!(error = %e, "cannot save presets");
        }
    }

    pub fn list(&self) -> Vec<Preset> {
        lock(&self.list).clone()
    }

    pub fn get(&self, id: &str) -> Option<Preset> {
        lock(&self.list).iter().find(|p| p.id == id).cloned()
    }

    /// Create or update from a JSON document (a non-empty `id` ⇒ update).
    pub fn upsert(&self, input: Value) -> anyhow::Result<Preset> {
        let id = input[KEY_ID]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let existing = id.as_deref().and_then(|i| self.get(i));
        let name = opt_str(&input, KEY_NAME).ok_or_else(|| anyhow!("{KEY_NAME} is required"))?;
        let project_id = opt_str(&input, KEY_PROJECT_ID)
            .ok_or_else(|| anyhow!("{KEY_PROJECT_ID} is required"))?;
        let template_id = opt_str(&input, KEY_TEMPLATE_ID)
            .ok_or_else(|| anyhow!("{KEY_TEMPLATE_ID} is required"))?;
        let target = opt_str(&input, KEY_TARGET);
        let inputs: Option<BTreeMap<String, String>> = input
            .get(KEY_INPUTS)
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let permission_mode = opt_str(&input, KEY_PERMISSION_MODE);
        let model = opt_str(&input, KEY_MODEL);
        let effort = opt_str(&input, KEY_EFFORT);
        let engine = opt_str(&input, KEY_ENGINE)
            .map(|name| Engine::parse(&name).ok_or_else(|| anyhow!("{KEY_ENGINE} is invalid")))
            .transpose()?;
        let autonomous = input
            .get(KEY_AUTONOMOUS)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let now = now_iso();
        let p = Preset {
            id: existing
                .as_ref()
                .map(|e| e.id.clone())
                .or(id)
                .unwrap_or_else(new_id),
            name,
            project_id,
            template_id,
            target,
            inputs,
            permission_mode,
            model,
            effort,
            engine,
            autonomous,
            created_at: existing
                .as_ref()
                .map(|e| e.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        {
            let mut l = lock(&self.list);
            if let Some(pos) = l.iter().position(|x| x.id == p.id) {
                l[pos] = p.clone();
            } else {
                l.push(p.clone());
            }
            self.save(&l);
        }
        self.bus.emit(
            EVENT_PRESET_SAVED,
            json!({ "presetId": p.id, "name": p.name }),
        );
        Ok(p)
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut l = lock(&self.list);
        let before = l.len();
        l.retain(|p| p.id != id);
        let removed = l.len() != before;
        if removed {
            self.save(&l);
        }
        removed
    }

    /// The launch request of a preset; its engine decides native or legacy.
    pub fn start_request(preset: &Preset) -> StartRequest {
        StartRequest {
            project_id: preset.project_id.clone(),
            engine: preset.engine.map(|engine| engine.as_str().to_string()),
            template_id: Some(preset.template_id.clone()),
            target: preset.target.clone(),
            inputs: preset.inputs.clone(),
            permission_mode: preset.permission_mode.clone(),
            model: preset.model.clone(),
            effort: preset.effort.clone(),
            origin: Some(Origin {
                kind: ORIGIN_KIND_PRESET.to_string(),
                r#ref: Some(preset.id.clone()),
                label: Some(preset.name.clone()),
            }),
            ..Default::default()
        }
    }

    /// Starts a preset through the launcher: native unless its engine names the legacy path.
    pub fn start(&self, launcher: &RunLauncher, id: &str) -> anyhow::Result<LaunchedRun> {
        let preset = self.get(id).ok_or_else(|| {
            crate::session_error::SessionError::NotFound(NOT_FOUND_MESSAGE.to_string())
        })?;
        launcher.launch(Self::start_request(&preset), preset.autonomous)
    }
}

#[cfg(test)]
#[path = "../tests/unit/presets_tests.rs"]
mod tests;
