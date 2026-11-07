use super::*;

const GRAPH: &str = r#"{
 "nodes": [
  {"id":"a","label":"PlayerController","source_file":"Assets/P.cs","source_location":"L10"},
  {"id":"b","label":"Jump","source_file":"Assets/J.cs","source_location":"L3"},
  {"id":"c","label":"Ext"},
  {"id":"d","label":"Mover","source_file":"Assets\\M.cs","source_location":"L1"}
 ],
 "links": [
  {"source":"a","target":"b","relation":"calls"},
  {"source":"a","target":"c","relation":"references"},
  {"source":"d","target":"a","relation":"inherits"},
  {"source":"b","target":"d","relation":"contains"}
 ]
}"#;

#[test]
fn neighbours_follow_calls_inherits_references_both_ways() {
    let graph = CodeGraph::parse(GRAPH).unwrap();
    let a = graph.symbols_named("playercontroller")[0];
    let labels: Vec<String> = graph
        .neighbours_of(a)
        .into_iter()
        .map(|n| graph.symbol(n).unwrap().label.clone())
        .collect();
    assert_eq!(labels, ["Jump", "Mover"]);
}

#[test]
fn symbols_in_filters_by_file_and_line_and_normalizes_slashes() {
    let graph = CodeGraph::parse(GRAPH).unwrap();
    assert_eq!(graph.symbols_in("Assets/P.cs", 1, 20).len(), 1);
    assert!(graph.symbols_in("Assets/P.cs", 11, 20).is_empty());
    assert_eq!(graph.symbols_in("Assets/M.cs", 1, 1).len(), 1);
}

#[test]
fn invalid_json_is_none() {
    assert!(CodeGraph::parse("nope").is_none());
}
