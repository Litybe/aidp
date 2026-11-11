use super::*;
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Weak;
use std::time::{Duration, Instant};

const FAKE_MODEL: &str = "fake-model";
const FAKE_DIM: usize = 4;
const FAKE_BYTES: u64 = 1024;
const LOAD_FAILURE: &str = "weights are corrupt";
const WAIT: Duration = Duration::from_secs(5);

struct FakeEmbedder;

impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        FAKE_MODEL
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> aidp_memory::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![1.0; FAKE_DIM]).collect())
    }
}

struct FakeLoader {
    installed: AtomicBool,
    fail: AtomicBool,
    gate: Mutex<Option<Receiver<()>>>,
    loaded: Mutex<Vec<Weak<dyn Embedder>>>,
}

impl FakeLoader {
    fn new(installed: bool) -> Arc<Self> {
        Arc::new(Self {
            installed: AtomicBool::new(installed),
            fail: AtomicBool::new(false),
            gate: Mutex::new(None),
            loaded: Mutex::new(Vec::new()),
        })
    }

    fn hold_loads(&self) -> Sender<()> {
        let (release, gate) = channel();
        *self.gate.lock().unwrap() = Some(gate);
        release
    }

    fn last_loaded(&self) -> Weak<dyn Embedder> {
        self.loaded.lock().unwrap().last().unwrap().clone()
    }
}

impl ModelLoader for FakeLoader {
    fn installed(&self, _: &Path) -> bool {
        self.installed.load(Ordering::SeqCst)
    }

    fn weights_bytes(&self, _: &Path) -> Option<u64> {
        Some(FAKE_BYTES)
    }

    fn load(&self, _: &Path) -> Result<Arc<dyn Embedder>, String> {
        if let Some(gate) = self.gate.lock().unwrap().take() {
            let _ = gate.recv_timeout(WAIT);
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(LOAD_FAILURE.to_string());
        }
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder);
        self.loaded.lock().unwrap().push(Arc::downgrade(&embedder));
        Ok(embedder)
    }
}

fn service(gateway: &TempWorkspace, loader: &Arc<FakeLoader>) -> (Arc<ModelService>, Arc<Bus>) {
    let bus = Arc::new(Bus::new());
    (
        ModelService::with_loader(gateway.path(), bus.clone(), loader.clone()),
        bus,
    )
}

fn wait_for(service: &ModelService, state: ModelState) {
    let deadline = Instant::now() + WAIT;
    while service.status().state != state {
        assert!(
            Instant::now() < deadline,
            "never reached {state:?}: {:?}",
            service.status()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn states(events: &Receiver<Value>) -> Vec<String> {
    events
        .try_iter()
        .filter(|event| event["type"] == EVENT_MEMORY_MODEL)
        .map(|event| event["data"]["state"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn start_without_the_model_files_is_refused_and_nothing_loads() {
    let gateway = TempWorkspace::new("model-missing");
    let loader = FakeLoader::new(false);
    let (service, _) = service(&gateway, &loader);

    assert!(!service.start());

    let status = service.status();
    assert_eq!(status.state, ModelState::Stopped);
    assert!(!status.installed);
    assert!(service.embedder().is_none());
}

#[test]
fn start_loads_in_the_background_and_publishes_each_state() {
    let gateway = TempWorkspace::new("model-start");
    let loader = FakeLoader::new(true);
    let (service, bus) = service(&gateway, &loader);
    let events = bus.subscribe();

    assert!(service.start());
    wait_for(&service, ModelState::Running);

    let status = service.status();
    assert_eq!(status.dim, Some(FAKE_DIM));
    assert_eq!(status.ram_bytes, Some(FAKE_BYTES));
    assert!(status.loaded_at.is_some());
    assert_eq!(status.error, None);
    assert_eq!(service.embedder().unwrap().model_id(), FAKE_MODEL);
    assert_eq!(states(&events), ["loading", "running"]);
}

#[test]
fn a_second_start_while_running_loads_nothing_more() {
    let gateway = TempWorkspace::new("model-twice");
    let loader = FakeLoader::new(true);
    let (service, _) = service(&gateway, &loader);
    service.start();
    wait_for(&service, ModelState::Running);

    assert!(service.start());

    assert_eq!(loader.loaded.lock().unwrap().len(), 1);
}

#[test]
fn stop_drops_the_model_so_its_memory_is_freed() {
    let gateway = TempWorkspace::new("model-stop");
    let loader = FakeLoader::new(true);
    let (service, bus) = service(&gateway, &loader);
    service.start();
    wait_for(&service, ModelState::Running);
    let weak = loader.last_loaded();
    assert!(weak.upgrade().is_some());
    let events = bus.subscribe();

    service.stop();

    assert!(weak.upgrade().is_none(), "the model is still alive");
    let status = service.status();
    assert_eq!(status.state, ModelState::Stopped);
    assert_eq!(status.ram_bytes, None);
    assert_eq!(status.loaded_at, None);
    assert!(service.embedder().is_none());
    assert_eq!(states(&events), ["stopped"]);
}

#[test]
fn a_failed_load_is_an_error_state_that_a_new_start_can_recover_from() {
    let gateway = TempWorkspace::new("model-error");
    let loader = FakeLoader::new(true);
    let (service, bus) = service(&gateway, &loader);
    let events = bus.subscribe();
    loader.fail.store(true, Ordering::SeqCst);

    service.start();
    wait_for(&service, ModelState::Error);

    assert_eq!(service.status().error.as_deref(), Some(LOAD_FAILURE));
    assert!(service.embedder().is_none());
    assert_eq!(states(&events), ["loading", "error"]);

    loader.fail.store(false, Ordering::SeqCst);
    service.start();
    wait_for(&service, ModelState::Running);
    assert_eq!(service.status().error, None);
}

#[test]
fn stopping_during_a_load_discards_the_model_it_was_loading() {
    let gateway = TempWorkspace::new("model-cancel");
    let loader = FakeLoader::new(true);
    let (service, _) = service(&gateway, &loader);
    let release = loader.hold_loads();

    service.start();
    assert_eq!(service.status().state, ModelState::Loading);
    service.stop();
    release.send(()).unwrap();

    let deadline = Instant::now() + WAIT;
    while loader.loaded.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the load never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(service.status().state, ModelState::Stopped);
    assert!(service.embedder().is_none());
    assert!(loader.last_loaded().upgrade().is_none());
}

#[test]
fn autostart_defaults_to_the_presence_of_the_model_and_is_persisted() {
    let gateway = TempWorkspace::new("model-autostart");
    let installed = FakeLoader::new(true);
    let (first, _) = service(&gateway, &installed);
    assert!(first.autostart());

    first.set_autostart(false).unwrap();

    let (second, _) = service(&gateway, &installed);
    assert!(!second.autostart());
    assert!(!second.status().autostart);
    second.start_if_autostart();
    assert_eq!(second.status().state, ModelState::Stopped);
    let saved = std::fs::read_to_string(gateway.path().join("memory/model.json")).unwrap();
    assert!(saved.contains("\"autostart\": false"), "{saved}");

    let absent = TempWorkspace::new("model-autostart-absent");
    let (third, _) = service(&absent, &FakeLoader::new(false));
    assert!(!third.autostart());
}

#[test]
fn start_if_autostart_loads_the_model_when_the_choice_is_on() {
    let gateway = TempWorkspace::new("model-boot");
    let loader = FakeLoader::new(true);
    let (service, _) = service(&gateway, &loader);
    service.set_autostart(true).unwrap();

    service.start_if_autostart();

    wait_for(&service, ModelState::Running);
}

#[test]
fn a_service_given_a_running_model_serves_it_at_once() {
    let gateway = TempWorkspace::new("model-given");
    let service =
        ModelService::running(gateway.path(), Arc::new(Bus::new()), Arc::new(FakeEmbedder));

    assert_eq!(service.status().state, ModelState::Running);
    assert_eq!(service.embedder().unwrap().dim(), FAKE_DIM);
}
