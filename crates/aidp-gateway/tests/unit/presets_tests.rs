use super::*;
use crate::models::constants::engine::{ENGINE_V7_ALIAS, ENGINE_WORKFLOW};
use aidp_testkit::TempWorkspace;

fn temp_dir(tag: &str) -> TempWorkspace {
    TempWorkspace::new(&format!("presets-test-{tag}"))
}

#[test]
fn upsert_validates_required_fields() {
    let _ws = temp_dir("validate");
    let dir = _ws.path();
    let presets = Presets::load(dir, Arc::new(Bus::new()));

    let err = presets.upsert(json!({})).unwrap_err().to_string();
    assert_eq!(err, "name is required");

    let err = presets
        .upsert(json!({ "name": "n" }))
        .unwrap_err()
        .to_string();
    assert_eq!(err, "projectId is required");

    let err = presets
        .upsert(json!({ "name": "n", "projectId": "p" }))
        .unwrap_err()
        .to_string();
    assert_eq!(err, "templateId is required");
}

#[test]
fn upsert_creates_and_persists() {
    let _ws = temp_dir("create");
    let dir = _ws.path();
    let presets = Presets::load(dir, Arc::new(Bus::new()));

    let p = presets
        .upsert(json!({ "name": "My preset", "projectId": "proj-1", "templateId": "dev" }))
        .unwrap();
    assert!(!p.id.is_empty());
    assert_eq!(p.name, "My preset");
    assert_eq!(p.created_at, p.updated_at);

    let reloaded = Presets::load(dir, Arc::new(Bus::new()));
    assert_eq!(reloaded.list().len(), 1);
    assert_eq!(
        reloaded.get(&p.id).map(|x| x.name),
        Some("My preset".to_string())
    );
}

#[test]
fn upsert_with_existing_id_updates_in_place() {
    let _ws = temp_dir("update");
    let dir = _ws.path();
    let presets = Presets::load(dir, Arc::new(Bus::new()));

    let created = presets
        .upsert(json!({ "name": "n1", "projectId": "p1", "templateId": "t1" }))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let updated = presets
        .upsert(json!({ "id": created.id, "name": "n2", "projectId": "p1", "templateId": "t1" }))
        .unwrap();

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.name, "n2");
    assert_eq!(updated.created_at, created.created_at);
    assert_ne!(updated.updated_at, created.updated_at);
    assert_eq!(presets.list().len(), 1);
}

#[test]
fn remove_returns_true_then_false() {
    let _ws = temp_dir("remove");
    let dir = _ws.path();
    let presets = Presets::load(dir, Arc::new(Bus::new()));

    let p = presets
        .upsert(json!({ "name": "n", "projectId": "p", "templateId": "t" }))
        .unwrap();
    assert!(presets.remove(&p.id));
    assert!(!presets.remove(&p.id));
}

#[test]
fn serializes_with_camel_case_keys() {
    let _ws = temp_dir("camel");
    let dir = _ws.path();
    let presets = Presets::load(dir, Arc::new(Bus::new()));

    let p = presets
        .upsert(json!({ "name": "n", "projectId": "p", "templateId": "t", "permissionMode": "acceptEdits" }))
        .unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert!(v.get("projectId").is_some());
    assert!(v.get("templateId").is_some());
    assert!(v.get("permissionMode").is_some());
    assert!(v.get("createdAt").is_some());
    assert!(v.get("project_id").is_none());
}

fn preset_with_engine(engine: Option<Engine>) -> Preset {
    Preset {
        id: "preset-1".to_string(),
        name: "name".to_string(),
        project_id: "project-1".to_string(),
        template_id: "dev".to_string(),
        target: None,
        inputs: None,
        permission_mode: None,
        model: None,
        effort: None,
        engine,
        autonomous: false,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn preset_start_request_defaults_to_native() {
    let request = Presets::start_request(&preset_with_engine(None));
    assert_eq!(request.engine, None);
    assert_eq!(request.template_id.as_deref(), Some("dev"));
}

#[test]
fn preset_start_request_keeps_workflow_engine_fallback() {
    let request = Presets::start_request(&preset_with_engine(Some(Engine::Workflow)));
    assert_eq!(request.engine.as_deref(), Some(ENGINE_WORKFLOW));
}

#[test]
fn upsert_persists_engine_and_autonomous() {
    let ws = temp_dir("engine");
    let presets = Presets::load(ws.path(), Arc::new(Bus::new()));
    let preset = presets
        .upsert(json!({
            "name": "n", "projectId": "p", "templateId": "t",
            "engine": ENGINE_V7_ALIAS, "autonomous": true
        }))
        .unwrap();
    assert_eq!(preset.engine, Some(Engine::Workflow));
    assert!(preset.autonomous);
}
