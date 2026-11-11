//! The one embedding model of the machine: owned by the daemon, loaded and unloaded on request.
//!
//! Every other process embeds through `POST /api/embed`; dropping the model frees its memory.

use crate::bus::Bus;
use crate::models::constants::model_service::{
    EVENT_MEMORY_MODEL, MODEL_LOAD_PANIC, MODEL_LOAD_THREAD, MODEL_THREAD_FAILED,
};
use crate::models::model_settings_file::ModelSettingsFile;
use crate::sync::lock;
use aidp_memory::models::constants::embedding::DEFAULT_MODEL_NAME;
use aidp_memory::models::constants::paths::{MEMORY_DIR, MODEL_SETTINGS_FILE};
use aidp_memory::models::enums::model_state::ModelState;
use aidp_memory::models::model_chunks::ModelChunks;
use aidp_memory::models::model_status_body::ModelStatusBody;
use aidp_memory::{
    model_dir, model_weights_bytes, resolve_model, Embedder, MemoryError, Model2VecEmbedder,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Finds and loads the model; replaced by a fake in tests.
pub trait ModelLoader: Send + Sync {
    /// Whether the model files are on disk.
    fn installed(&self, gateway_dir: &Path) -> bool;
    /// Size of the weights file, the RAM estimate.
    fn weights_bytes(&self, gateway_dir: &Path) -> Option<u64>;
    /// Reads the model into memory; the message of a failure is shown to the operator.
    fn load(&self, gateway_dir: &Path) -> Result<Arc<dyn Embedder>, String>;
}

/// Loads the Model2Vec folder under the gateway directory.
pub struct DiskModelLoader;

impl ModelLoader for DiskModelLoader {
    fn installed(&self, gateway_dir: &Path) -> bool {
        resolve_model(gateway_dir).is_some()
    }

    fn weights_bytes(&self, gateway_dir: &Path) -> Option<u64> {
        model_weights_bytes(gateway_dir)
    }

    fn load(&self, gateway_dir: &Path) -> Result<Arc<dyn Embedder>, String> {
        let dir = resolve_model(gateway_dir).ok_or_else(|| {
            MemoryError::NotFound(model_dir(gateway_dir).display().to_string()).to_string()
        })?;
        Model2VecEmbedder::load(&dir)
            .map(|model| Arc::new(model) as Arc<dyn Embedder>)
            .map_err(|error| error.to_string())
    }
}

/// Reads how many chunks hold a vector of the model.
pub type CoverageSource = Arc<dyn Fn() -> ModelChunks + Send + Sync>;

struct Inner {
    state: ModelState,
    embedder: Option<Arc<dyn Embedder>>,
    loaded_at: Option<String>,
    error: Option<String>,
    dim: Option<usize>,
    generation: u64,
}

/// State machine `stopped -> loading -> running | error`, back to `stopped` on stop.
pub struct ModelService {
    gateway_dir: PathBuf,
    bus: Arc<Bus>,
    loader: Arc<dyn ModelLoader>,
    coverage: Mutex<Option<CoverageSource>>,
    inner: Mutex<Inner>,
}

impl ModelService {
    /// A stopped service over the model folder of `gateway_dir`.
    pub fn new(gateway_dir: &Path, bus: Arc<Bus>) -> Arc<Self> {
        Self::with_loader(gateway_dir, bus, Arc::new(DiskModelLoader))
    }

    /// A stopped service with its own loader.
    pub fn with_loader(
        gateway_dir: &Path,
        bus: Arc<Bus>,
        loader: Arc<dyn ModelLoader>,
    ) -> Arc<Self> {
        Arc::new(Self {
            gateway_dir: gateway_dir.to_path_buf(),
            bus,
            loader,
            coverage: Mutex::new(None),
            inner: Mutex::new(Inner {
                state: ModelState::Stopped,
                embedder: None,
                loaded_at: None,
                error: None,
                dim: None,
                generation: 0,
            }),
        })
    }

    /// A service already running with the given embedder, for tests that need no load.
    pub fn running(gateway_dir: &Path, bus: Arc<Bus>, embedder: Arc<dyn Embedder>) -> Arc<Self> {
        let service = Self::new(gateway_dir, bus);
        {
            let mut inner = lock(&service.inner);
            inner.state = ModelState::Running;
            inner.dim = Some(embedder.dim());
            inner.loaded_at = Some(chrono::Utc::now().to_rfc3339());
            inner.embedder = Some(embedder);
        }
        service
    }

    /// Where the chunk counts of the status come from.
    pub fn set_coverage(&self, source: CoverageSource) {
        *lock(&self.coverage) = Some(source);
    }

    /// The model when it is running.
    pub fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        let inner = lock(&self.inner);
        (inner.state == ModelState::Running)
            .then(|| inner.embedder.clone())
            .flatten()
    }

    /// Starts loading on a background thread; `false` when the model is not installed.
    pub fn start(self: &Arc<Self>) -> bool {
        let generation = {
            let mut inner = lock(&self.inner);
            if matches!(inner.state, ModelState::Running | ModelState::Loading) {
                return true;
            }
            if !self.loader.installed(&self.gateway_dir) {
                return false;
            }
            inner.state = ModelState::Loading;
            inner.error = None;
            inner.generation += 1;
            inner.generation
        };
        self.publish();
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name(MODEL_LOAD_THREAD.to_string())
            .spawn(move || {
                let outcome = crate::threads::guarded(MODEL_LOAD_THREAD, || {
                    service.loader.load(&service.gateway_dir)
                })
                .unwrap_or_else(|| Err(MODEL_LOAD_PANIC.to_string()));
                service.finish_load(generation, outcome);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "embedding model load thread not started");
            self.finish_load(generation, Err(MODEL_THREAD_FAILED.to_string()));
        }
        true
    }

    fn finish_load(&self, generation: u64, outcome: Result<Arc<dyn Embedder>, String>) {
        {
            let mut inner = lock(&self.inner);
            if inner.generation != generation || inner.state != ModelState::Loading {
                return;
            }
            match outcome {
                Ok(embedder) => {
                    inner.dim = Some(embedder.dim());
                    inner.embedder = Some(embedder);
                    inner.state = ModelState::Running;
                    inner.loaded_at = Some(chrono::Utc::now().to_rfc3339());
                }
                Err(message) => {
                    tracing::warn!(error = %message, "embedding model not loaded");
                    inner.state = ModelState::Error;
                    inner.error = Some(message);
                }
            }
        }
        self.publish();
    }

    /// Unloads the model, cancelling a load in progress; the memory is freed once in-flight users finish.
    pub fn stop(&self) {
        let released = {
            let mut inner = lock(&self.inner);
            inner.generation += 1;
            inner.state = ModelState::Stopped;
            inner.loaded_at = None;
            inner.error = None;
            inner.embedder.take()
        };
        drop(released);
        self.publish();
    }

    /// Whether the model loads when the daemon starts: the saved choice, else true when installed.
    pub fn autostart(&self) -> bool {
        read_settings(&self.gateway_dir)
            .map(|settings| settings.autostart)
            .unwrap_or_else(|| self.loader.installed(&self.gateway_dir))
    }

    /// Saves the autostart choice in `memory/model.json` and publishes the status.
    pub fn set_autostart(&self, autostart: bool) -> std::io::Result<()> {
        let dir = self.gateway_dir.join(MEMORY_DIR);
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(&ModelSettingsFile { autostart })?;
        std::fs::write(dir.join(MODEL_SETTINGS_FILE), text)?;
        self.publish();
        Ok(())
    }

    /// Daemon start: loads the model when autostart is on and the model is installed.
    pub fn start_if_autostart(self: &Arc<Self>) {
        if self.autostart() {
            self.start();
        }
    }

    /// The status body of `GET /api/memory/model`.
    pub fn status(&self) -> ModelStatusBody {
        let (state, loaded_at, error, dim) = {
            let inner = lock(&self.inner);
            (
                inner.state,
                inner.loaded_at.clone(),
                inner.error.clone(),
                inner.dim,
            )
        };
        let source = lock(&self.coverage).clone();
        let chunks = source.map(|read| read()).unwrap_or_default();
        let ram_bytes = matches!(state, ModelState::Loading | ModelState::Running)
            .then(|| self.loader.weights_bytes(&self.gateway_dir))
            .flatten();
        ModelStatusBody {
            model: DEFAULT_MODEL_NAME.to_string(),
            installed: self.loader.installed(&self.gateway_dir),
            state,
            autostart: self.autostart(),
            dim,
            ram_bytes,
            loaded_at,
            error,
            chunks,
            path: model_dir(&self.gateway_dir).display().to_string(),
        }
    }

    fn publish(&self) {
        match serde_json::to_value(self.status()) {
            Ok(payload) => {
                self.bus.emit(EVENT_MEMORY_MODEL, payload);
            }
            Err(error) => tracing::warn!(%error, "model status event not serialised"),
        }
    }
}

fn read_settings(gateway_dir: &Path) -> Option<ModelSettingsFile> {
    let text =
        std::fs::read_to_string(gateway_dir.join(MEMORY_DIR).join(MODEL_SETTINGS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
#[path = "../tests/unit/model_service_tests.rs"]
mod tests;
