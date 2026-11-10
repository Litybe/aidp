use super::*;
use crate::retriever::Bm25Retriever;
use crate::test_support::{indexed_store, PROJECT};

const CASES: &str = include_str!("../fixtures/eval/cases.yaml");
const K: usize = 3;
const MISSED_QUERY: &str = "tresor introuvable inexistant";

#[test]
fn the_report_counts_answers_within_k_and_lists_misses() {
    let dir = tempfile::tempdir().unwrap();
    let (store, project) = indexed_store(dir.path());
    let retriever = Bm25Retriever::new(&store, Some(project.root));
    let cases = parse_cases(CASES).unwrap();
    let manager = MemoryManager::new(&store);
    let report = evaluate(&manager, &retriever, PROJECT, &cases, K).unwrap();
    assert_eq!((report.cases, report.k), (4, K));
    assert_eq!(report.recall, 0.75);
    assert_eq!(report.misses.len(), 1);
    assert_eq!(report.misses[0].query, MISSED_QUERY);
    assert!(report.mrr > 0.5 && report.mrr <= 0.75);
}

#[test]
fn unknown_fields_in_cases_are_refused() {
    assert!(matches!(
        parse_cases("- query: x\n  expect: []\n  extra: 1\n"),
        Err(MemoryError::Invalid(_))
    ));
}
