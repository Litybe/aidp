use super::*;
use serde_json::{json, Value};

fn proposal(overrides: Value) -> Value {
    let mut base = json!({
        "action": "add", "domain": "unity", "scope": "global",
        "slug": "coyote-time", "title": "Coyote time", "body": "Texte."
    });
    for (key, value) in overrides.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn parse(proposals: Vec<Value>) -> Result<KnowledgeProposalFile> {
    parse_knowledge_proposals(json!({ "proposals": proposals }).to_string().as_bytes())
}

#[test]
fn a_minimal_proposal_gets_its_defaults() {
    let file = parse(vec![proposal(json!({}))]).unwrap();
    let only = &file.proposals[0];
    assert_eq!(only.importance, 0.5);
    assert!(only.tags.is_empty() && only.agents.is_empty());
    assert!(only.target.is_none() && only.evidence.is_none());
}

#[test]
fn more_than_five_proposals_refuse_the_file() {
    let many = (0..6).map(|_| proposal(json!({}))).collect();
    assert!(parse(many).unwrap_err().to_string().contains("at most 5"));
    assert!(parse((0..5).map(|_| proposal(json!({}))).collect()).is_ok());
}

#[test]
fn limits_are_enforced() {
    let cases = [
        json!({ "slug": "Not Kebab" }),
        json!({ "slug": "-lead" }),
        json!({ "title": "  " }),
        json!({ "title": "two\nlines" }),
        json!({ "body": "x".repeat(8001) }),
        json!({ "body": " " }),
        json!({ "importance": 1.5 }),
        json!({ "importance": -0.1 }),
        json!({ "tags": ["bad,tag"] }),
        json!({ "agents": ["[x]"] }),
        json!({ "evidence": "x".repeat(1001) }),
        json!({ "action": "update" }),
        json!({ "action": "supersede", "target": "" }),
    ];
    for case in cases {
        assert!(parse(vec![proposal(case.clone())]).is_err(), "{case}");
    }
    assert!(parse(vec![proposal(json!({ "body": "x".repeat(8000) }))]).is_ok());
}

#[test]
fn unknown_fields_and_values_refuse_the_file() {
    assert!(parse(vec![proposal(json!({ "extra": 1 }))]).is_err());
    assert!(parse(vec![proposal(json!({ "domain": "cooking" }))]).is_err());
    assert!(parse(vec![proposal(json!({ "scope": "galaxy" }))]).is_err());
    assert!(parse_knowledge_proposals(b"not json").is_err());
    assert!(parse_knowledge_proposals(br#"{"proposals":[],"x":1}"#).is_err());
}

#[test]
fn plugin_prefixed_agents_and_updates_with_a_target_are_accepted() {
    let file = parse(vec![proposal(json!({
        "action": "update", "target": "_global/unity/a.md", "agents": ["aidp:executor"], "tags": ["a b"]
    }))])
    .unwrap();
    assert_eq!(file.proposals[0].agents, ["aidp:executor"]);
}

#[test]
fn the_schema_file_accepts_what_the_parser_accepts() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../../schema/knowledge-proposals.schema.json"
    ))
    .unwrap();
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .expect("schema compiles");
    let good = json!({ "proposals": [proposal(json!({ "tags": ["a"], "importance": 0.4 }))] });
    assert!(compiled.is_valid(&good));
    let bad = json!({ "proposals": [proposal(json!({ "slug": "Bad Slug" }))] });
    assert!(!compiled.is_valid(&bad));
    let too_many = json!({ "proposals": (0..6).map(|_| proposal(json!({}))).collect::<Vec<_>>() });
    assert!(!compiled.is_valid(&too_many));
}
