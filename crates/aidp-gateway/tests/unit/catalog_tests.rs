use super::*;

const TEMPLATE_ID: &str = "native-gated";
const FLOW_ID: &str = "test-flow";
const OTHER_ID: &str = "other";
const BAD_PATH: &str = "../escape";
const TEMPLATE_YAML: &str = include_str!("../fixtures/native_runs/gated.yaml");
const FLOW_YAML: &str = include_str!("../fixtures/flows/test-flow.yaml");
const CYCLIC_YAML: &str = include_str!("../fixtures/catalog/cycle.yaml");
const EMPTY_OBJECT: &str = "{}";
const EXPECTED_STAGE_COUNT: usize = 3;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_BAD_REQUEST: u16 = 400;
const STATUS_CONFLICT: u16 = 409;
const TEST_COLLECTION_LIMIT: usize = 3;

fn catalog() -> (tempfile::TempDir, PipelineCatalog) {
    let root = tempfile::tempdir().unwrap();
    let catalog = PipelineCatalog::new(root.path().to_path_buf());
    (root, catalog)
}

fn write_body(id: &str, content: &str) -> Value {
    serde_json::to_value(CatalogWriteRequest {
        id: Some(id.to_string()),
        content: content.to_string(),
    })
    .unwrap()
}

#[test]
fn catalog_create_read_update_and_list_share_validation() {
    let (_root, catalog) = catalog();
    assert!(catalog.templates().unwrap().is_empty());
    catalog
        .save_template(TEMPLATE_ID, TEMPLATE_YAML, true)
        .unwrap();
    let listed = catalog.templates().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].stage_count, EXPECTED_STAGE_COUNT);
    assert_eq!(catalog.template(TEMPLATE_ID).unwrap().raw, TEMPLATE_YAML);
    let modified = format!("{TEMPLATE_YAML}\n");
    catalog
        .save_template(TEMPLATE_ID, &modified, false)
        .unwrap();
    assert_eq!(catalog.template(TEMPLATE_ID).unwrap().raw, modified);
    assert!(catalog
        .save_template(TEMPLATE_ID, EMPTY_OBJECT, false)
        .is_err());
    assert_eq!(catalog.template(TEMPLATE_ID).unwrap().raw, modified);
}

#[test]
fn catalog_rejects_unsafe_names_and_mismatched_document_ids() {
    let (_root, catalog) = catalog();
    assert!(catalog
        .save_template(BAD_PATH, TEMPLATE_YAML, true)
        .is_err());
    assert!(catalog
        .save_template(OTHER_ID, TEMPLATE_YAML, true)
        .is_err());
    assert!(catalog.templates().unwrap().is_empty());
}

#[test]
fn catalog_reads_flow_definitions_without_launching() {
    let (_root, catalog) = catalog();
    let directory = catalog.directory(true);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{FLOW_ID}.{YAML_EXTENSION}")),
        FLOW_YAML,
    )
    .unwrap();
    assert_eq!(catalog.flows().unwrap()[0].id, FLOW_ID);
    assert_eq!(catalog.flow(FLOW_ID).unwrap().raw, FLOW_YAML);
}

#[test]
fn catalog_validation_rejects_schema_errors_cycles_and_oversized_yaml() {
    assert!(validate_template(TEMPLATE_YAML).valid);
    assert!(!validate_template(EMPTY_OBJECT).valid);
    assert!(!validate_template(CYCLIC_YAML).valid);
    assert!(!validate_template(&" ".repeat(MAX_PIPELINE_BYTES as usize + 1)).valid);
}

#[test]
fn catalogue_collection_limits_stop_at_exact_budget_and_file_count() {
    let (root, catalog) = catalog();
    let directory = catalog.directory(false);
    fs::create_dir_all(&directory).unwrap();
    for index in 0..=MAX_COLLECTION_FILES {
        fs::write(
            directory.join(format!("{TEMPLATE_ID}-{index}.{YAML_EXTENSION}")),
            EMPTY_OBJECT,
        )
        .unwrap();
    }
    assert_eq!(
        catalog.templates().unwrap_err().to_string(),
        COLLECTION_TOO_LARGE
    );
    let path = format!("{API_PREFIX}/{TEMPLATES}");
    assert_eq!(
        handle(
            Some(root.path().to_path_buf()),
            &Method::Get,
            &path,
            &Value::Null
        )
        .unwrap()
        .status,
        STATUS_BAD_REQUEST
    );
    let load = |_: &str| {
        Ok(CatalogDocument {
            parsed: true,
            raw: EMPTY_OBJECT.to_string(),
        })
    };
    assert_eq!(
        collect_bounded(vec![TEMPLATE_ID.into()], TEST_COLLECTION_LIMIT, load).unwrap(),
        vec![true]
    );
    assert_eq!(
        collect_bounded(
            vec![TEMPLATE_ID.into(), OTHER_ID.into()],
            TEST_COLLECTION_LIMIT,
            load
        )
        .unwrap_err()
        .to_string(),
        COLLECTION_TOO_LARGE
    );
}

#[test]
fn catalog_routes_return_exact_not_found_conflict_and_bad_request_statuses() {
    let (root, _catalog) = catalog();
    let templates_path = format!("{API_PREFIX}/{TEMPLATES}");
    let template_path = format!("{templates_path}/{TEMPLATE_ID}");
    let root = || Some(root.path().to_path_buf());
    assert_eq!(
        handle(root(), &Method::Get, &template_path, &Value::Null)
            .unwrap()
            .status,
        STATUS_NOT_FOUND
    );
    let body = write_body(TEMPLATE_ID, TEMPLATE_YAML);
    assert_eq!(
        handle(root(), &Method::Post, &templates_path, &body)
            .unwrap()
            .status,
        HTTP_CREATED
    );
    assert_eq!(
        handle(root(), &Method::Post, &templates_path, &body)
            .unwrap()
            .status,
        STATUS_CONFLICT
    );
    let invalid = write_body(TEMPLATE_ID, EMPTY_OBJECT);
    assert_eq!(
        handle(root(), &Method::Put, &template_path, &invalid)
            .unwrap()
            .status,
        STATUS_BAD_REQUEST
    );
    assert!(handle(root(), &Method::Get, TEMPLATES, &Value::Null).is_none());
}
