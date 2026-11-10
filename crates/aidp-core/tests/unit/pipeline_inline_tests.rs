use super::*;

const DEFAULT_PIPELINE_KIND: &str = "pipeline";

#[test]
fn load_refuses_oversized_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("big.yaml");
    std::fs::write(&f, vec![b'#'; (super::MAX_PIPELINE_BYTES + 1) as usize]).unwrap();
    let err = format!("{:#}", super::load(&f).unwrap_err());
    assert!(err.contains("1048576"), "{err}");
}

#[test]
fn load_accepts_small_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("ok.yaml");
    std::fs::write(
        &f,
        "version: 5
id: tiny
stages:
  - name: a
agent: analyzer
",
    )
    .unwrap();
    let p = super::load(&f).unwrap();
    assert_eq!(p.id, "tiny");
}

#[test]
fn it_discover_skips_oversized_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.yaml");
    let mut big_yaml = String::from("version: 5\nid: too-big\nstages: []\n# ");
    big_yaml.push_str(&"#".repeat((super::MAX_PIPELINE_BYTES + 1) as usize));
    std::fs::write(&big, big_yaml).unwrap();
    let small = dir.path().join("small.yaml");
    std::fs::write(
        &small,
        "version: 5
id: just-right
stages: []
",
    )
    .unwrap();
    let found = super::discover(&[dir.path()]).unwrap();
    assert!(
        found.iter().all(|(id, _)| id != "too-big"),
        "oversized candidate should be skipped: {found:?}"
    );
    assert!(
        found.iter().any(|(id, _)| id == "just-right"),
        "small candidate should still be discovered: {found:?}"
    );
}

/// Archived templates keep their original id, so discovery must skip them
/// or `uses:` and `--pipeline <id>` resolution becomes order-dependent.
#[test]
fn it_discovery_skips_the_legacy_archive() {
    let root = std::env::temp_dir().join(format!("aidp-archive-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("legacy")).unwrap();
    let yaml =
        |id: &str| format!("version: 5\nid: {id}\nstages:\n  - {{ name: s, agent: analyzer }}\n");
    std::fs::write(root.join("dev.yaml"), yaml("dev")).unwrap();
    std::fs::write(root.join("legacy").join("dev.yaml"), yaml("dev")).unwrap();
    std::fs::write(root.join("legacy").join("gone.yaml"), yaml("gone")).unwrap();

    let found = discover(&[root.as_path()]).unwrap();
    assert_eq!(
        found.len(),
        1,
        "archived pipelines must not be discovered: {found:?}"
    );
    assert_eq!(found[0].0, "dev");
    assert_eq!(found[0].1, root.join("dev.yaml"), "the live template wins");

    let inside = discover(&[root.join("legacy").as_path()]).unwrap();
    assert_eq!(
        inside.len(),
        2,
        "an explicit archive root still lists them: {inside:?}"
    );
    assert!(is_archived(&root.join("legacy").join("dev.yaml")));
    assert!(!is_archived(&root.join("dev.yaml")));
    let _ = std::fs::remove_dir_all(&root);
}

/// A removed `rolling_context_window` setting must land in `Settings::extra`
/// via `#[serde(flatten)]`, never regain a typed field.
#[test]
fn removed_rolling_context_window_lands_in_extra_not_a_typed_field() {
    let yaml = "rolling_context_window: 2\n";
    let settings: Settings = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(
        settings.extra.get("rolling_context_window"),
        Some(&serde_yaml::Value::Number(2.into())),
        "unknown key must be captured by the `extra` catch-all"
    );
    assert_eq!(settings.max_correction_iterations, 2);
    assert_eq!(settings.max_stage_retries, 0);
    assert!(settings.max_total_stages.is_none());
    assert!(settings.max_stages_per_pass.is_none());
    assert!(settings.knowledge_graph.is_none());
    assert!(!settings.emit_effort);
}

/// C5: `effort:` must validate against the enum; an out-of-enum value is
/// rejected by the schema (not silently accepted and passed through).
#[test]
fn effort_out_of_enum_rejected_by_schema() {
    let root = std::env::temp_dir().join(format!("aidp-effort-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let bad = root.join("bad-effort.yaml");
    std::fs::write(
        &bad,
        "version: 5\nid: bad-effort\nstages:\n  - { name: s, agent: executor, effort: nope }\n",
    )
    .unwrap();
    let err = load(&bad).expect_err("an out-of-enum `effort:` must fail schema validation");
    assert!(
        format!("{err}").contains("effort") || format!("{err:#}").contains("nope"),
        "error should reference the offending `effort` field: {err}"
    );

    let ok = root.join("ok-effort.yaml");
    std::fs::write(
        &ok,
        "version: 5\nid: ok-effort\nstages:\n  - { name: s, agent: executor, effort: high }\n",
    )
    .unwrap();
    load(&ok).expect("a valid enum `effort:` value must pass schema validation");

    let none = root.join("no-effort.yaml");
    std::fs::write(
        &none,
        "version: 5\nid: no-effort\nstages:\n  - { name: s, agent: executor }\n",
    )
    .unwrap();
    load(&none).expect("omitted `effort:` must still validate");

    let _ = std::fs::remove_dir_all(&root);
}

/// `kind:` defaults to `pipeline`; an explicit `fragment` survives
/// validation and a serialize/reload round trip.
#[test]
fn kind_defaults_to_pipeline_and_fragment_round_trips() {
    let root = std::env::temp_dir().join(format!("aidp-kind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let no_kind = root.join("no-kind.yaml");
    std::fs::write(
        &no_kind,
        "version: 5\nid: no-kind\nstages:\n  - { name: s, agent: executor }\n",
    )
    .unwrap();
    let p = load(&no_kind).expect("no `kind:` must still validate");
    assert_eq!(p.kind, "pipeline");

    let frag = root.join("frag.yaml");
    std::fs::write(
        &frag,
        "version: 5\nid: frag\nkind: fragment\nstages:\n  - { name: s, agent: executor }\n",
    )
    .unwrap();
    let p2 = load(&frag).expect("`kind: fragment` must validate");
    assert_eq!(p2.kind, "fragment");

    let reserialized = root.join("frag-reserialized.yaml");
    std::fs::write(&reserialized, serde_yaml::to_string(&p2).unwrap()).unwrap();
    let p3 = load(&reserialized).expect("re-serialized fragment must still re-validate");
    assert_eq!(p3.kind, "fragment", "kind must survive the round trip");

    let _ = std::fs::remove_dir_all(&root);
}

/// `SkillsBy::resolve` is `base` then the mapped or default list,
/// de-duplicated keeping the first occurrence and the order.
#[test]
fn skills_by_resolves_and_dedupes() {
    let mut map = BTreeMap::new();
    map.insert(
        "rust".to_string(),
        vec!["rust-conventions".to_string(), "shared-skill".to_string()],
    );
    let by = SkillsBy {
        input: "language".into(),
        map,
        default: vec!["generic-skill".to_string()],
    };
    let base = vec!["shared-skill".to_string(), "base-only".to_string()];

    let resolved = by.resolve(&base, Some("rust"));
    assert_eq!(
        resolved,
        vec![
            "shared-skill".to_string(),
            "base-only".to_string(),
            "rust-conventions".to_string(),
        ]
    );

    let resolved_unknown = by.resolve(&base, Some("cobol"));
    assert_eq!(
        resolved_unknown,
        vec![
            "shared-skill".to_string(),
            "base-only".to_string(),
            "generic-skill".to_string(),
        ]
    );

    let resolved_none = by.resolve(&base, None);
    assert_eq!(resolved_none, resolved_unknown);
}

/// `description` and `policy` round-trip through serde, and an absent
/// `skills_by` is omitted on re-serialization.
#[test]
fn new_stage_keys_survive_a_serde_round_trip() {
    let yaml =
        "name: s\nagent: executor\ndescription: does the thing\npolicy: [\"never touch prod\"]\n";
    let s: Stage = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(s.description, "does the thing");
    assert_eq!(s.policy, vec!["never touch prod".to_string()]);
    assert!(s.skills_by.is_none());

    let out = serde_yaml::to_string(&s).unwrap();
    let s2: Stage = serde_yaml::from_str(&out).unwrap();
    assert_eq!(s2.description, "does the thing");
    assert_eq!(s2.policy, vec!["never touch prod".to_string()]);
    assert!(s2.skills_by.is_none());

    assert!(
        !out.contains("skills_by"),
        "skip_serializing_if must omit an absent skills_by: {out}"
    );
}

mod validate_inputs {
    use super::*;

    fn pipe_with_inputs(inputs: BTreeMap<String, InputSpec>) -> Pipeline {
        Pipeline {
            version: 5,
            id: "test-pipe".into(),
            name: String::new(),
            description: String::new(),
            kind: DEFAULT_PIPELINE_KIND.into(),
            policy: vec![],
            inputs,
            settings: Settings::default(),
            stages: vec![Stage {
                name: "s".into(),
                description: String::new(),
                policy: vec![],
                r#type: None,
                uses: None,
                composite_entry: None,
                composite_when: vec![],
                agent: Some("executor".into()),
                model: None,
                effort: None,
                provider: None,
                skills: vec![],
                skills_by: None,
                needs: vec![],
                inputs: BTreeMap::new(),
                outputs: vec![],
                with: BTreeMap::new(),
                when: None,
                gate: None,
                scope: None,
                isolation: None,
                tools: None,
                fan_out: None,
                merge_findings: None,
                on_harness_gap: None,
                plan_tasks: None,
                max_iterations: None,
                on_exhausted: None,
                knowledge: None,
                body: vec![],
            }],
        }
    }

    fn spec(
        required: bool,
        kind: &str,
        values: Vec<&str>,
        default: Option<serde_yaml::Value>,
    ) -> InputSpec {
        InputSpec {
            required,
            kind: kind.into(),
            values: values.into_iter().map(String::from).collect(),
            default,
            description: "a test input".into(),
        }
    }

    #[test]
    fn missing_required_input_fails_naming_it() {
        let mut inputs = BTreeMap::new();
        inputs.insert("topic".to_string(), spec(true, "string", vec![], None));
        let pipe = pipe_with_inputs(inputs);
        let err = super::super::validate_inputs(&pipe, &BTreeMap::new())
            .expect_err("required input with no default must fail");
        assert!(err.to_string().contains("topic"), "got: {err}");
        assert!(err.to_string().contains("required input"), "got: {err}");
    }

    #[test]
    fn default_is_applied_when_absent() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "track".to_string(),
            spec(
                true,
                "string",
                vec![],
                Some(serde_yaml::Value::String("standard".into())),
            ),
        );
        let pipe = pipe_with_inputs(inputs);
        let out = super::super::validate_inputs(&pipe, &BTreeMap::new())
            .expect("default satisfies required");
        assert_eq!(out.get("track").and_then(|v| v.as_str()), Some("standard"));
    }

    #[test]
    fn value_outside_enum_fails_listing_allowed() {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "track".to_string(),
            spec(false, "string", vec!["standard", "quick"], None),
        );
        let pipe = pipe_with_inputs(inputs);
        let mut provided = BTreeMap::new();
        provided.insert("track".to_string(), serde_json::json!("bogus"));
        let err = super::super::validate_inputs(&pipe, &provided)
            .expect_err("value outside `values:` must fail");
        assert!(
            err.to_string().contains("standard") && err.to_string().contains("quick"),
            "got: {err}"
        );
    }

    #[test]
    fn undeclared_input_warns_and_succeeds() {
        let pipe = pipe_with_inputs(BTreeMap::new());
        let mut provided = BTreeMap::new();
        provided.insert("mystery".to_string(), serde_json::json!("x"));
        let out = super::super::validate_inputs(&pipe, &provided)
            .expect("undeclared input must only warn");
        assert_eq!(out.get("mystery").and_then(|v| v.as_str()), Some("x"));
    }

    #[test]
    fn jira_key_is_coerced_into_jira_keys_list() {
        let mut inputs = BTreeMap::new();
        inputs.insert("jira_keys".to_string(), spec(true, "list", vec![], None));
        let pipe = pipe_with_inputs(inputs);
        let mut provided = BTreeMap::new();
        provided.insert("jira_key".to_string(), serde_json::json!("PIXEL-1"));
        let out = super::super::validate_inputs(&pipe, &provided)
            .expect("alias coercion must satisfy required");
        assert_eq!(out.get("jira_keys"), Some(&serde_json::json!(["PIXEL-1"])));
    }

    #[test]
    fn target_is_coerced_into_jira_keys_list_as_last_resort() {
        let mut inputs = BTreeMap::new();
        inputs.insert("jira_keys".to_string(), spec(true, "list", vec![], None));
        let pipe = pipe_with_inputs(inputs);
        let mut provided = BTreeMap::new();
        provided.insert("target".to_string(), serde_json::json!("PIXEL-1"));
        let out = super::super::validate_inputs(&pipe, &provided)
            .expect("target coercion must satisfy required");
        assert_eq!(out.get("jira_keys"), Some(&serde_json::json!(["PIXEL-1"])));
    }

    /// A dual-demand pipeline defaults `jira_keys` to `[]`; an empty list must
    /// still be promoted from a singular alias.
    fn dual_demand_pipe() -> Pipeline {
        let mut inputs = BTreeMap::new();
        inputs.insert(
            "jira_keys".to_string(),
            spec(
                false,
                "list",
                vec![],
                Some(serde_yaml::Value::Sequence(vec![])),
            ),
        );
        inputs.insert(
            "prompt".to_string(),
            spec(
                false,
                "string",
                vec![],
                Some(serde_yaml::Value::String(String::new())),
            ),
        );
        pipe_with_inputs(inputs)
    }

    #[test]
    fn target_is_coerced_even_when_an_empty_list_default_already_filled_the_input() {
        let mut provided = BTreeMap::new();
        provided.insert("target".to_string(), serde_json::json!("PIXEL-1"));
        let out = super::super::validate_inputs(&dual_demand_pipe(), &provided)
            .expect("validation must pass");
        assert_eq!(
            out.get("jira_keys"),
            Some(&serde_json::json!(["PIXEL-1"])),
            "the declared `default: []` must not shadow the `--target` shorthand: defaults \
             are applied before coercion, so coercing only an ABSENT key silently disabled \
             this for every dual-demand template"
        );
    }

    #[test]
    fn target_is_not_coerced_into_jira_keys_on_a_prompt_driven_run() {
        let mut provided = BTreeMap::new();
        provided.insert("target".to_string(), serde_json::json!("LOC-1"));
        provided.insert("prompt".to_string(), serde_json::json!("figer le contrat"));
        let out = super::super::validate_inputs(&dual_demand_pipe(), &provided)
            .expect("validation must pass");
        assert_eq!(
            out.get("jira_keys"),
            Some(&serde_json::json!([])),
            "a prompt-driven run must keep `jira_keys` empty, or the run carries two demands \
             and hunts a local key in Jira"
        );
    }

    #[test]
    fn the_explicit_jira_key_alias_is_coerced_even_on_a_prompt_driven_run() {
        let mut provided = BTreeMap::new();
        provided.insert("jira_key".to_string(), serde_json::json!("PIXEL-9"));
        provided.insert("prompt".to_string(), serde_json::json!("x"));
        let out = super::super::validate_inputs(&dual_demand_pipe(), &provided)
            .expect("validation must pass");
        assert_eq!(
            out.get("jira_keys"),
            Some(&serde_json::json!(["PIXEL-9"])),
            "`jira_key` names Jira explicitly — unlike the generic `target`, passing it is an \
             unambiguous request, so it is honoured whatever else the run carries"
        );
    }

    #[test]
    fn a_non_empty_jira_keys_is_never_overwritten_by_an_alias() {
        let mut provided = BTreeMap::new();
        provided.insert("jira_keys".to_string(), serde_json::json!(["A", "B"]));
        provided.insert("target".to_string(), serde_json::json!("PIXEL-1"));
        let out = super::super::validate_inputs(&dual_demand_pipe(), &provided)
            .expect("validation must pass");
        assert_eq!(
            out.get("jira_keys"),
            Some(&serde_json::json!(["A", "B"])),
            "coercion is a filet for a missing value, never an override of an explicit one"
        );
    }
}
