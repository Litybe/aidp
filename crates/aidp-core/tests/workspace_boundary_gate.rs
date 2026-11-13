//! Keeps the workspace self-contained: tests compile the frozen skill under `fixtures/skill`
//! instead of reading the skill or the benchmark, which live in their own repositories. The
//! source-wide boundary rule lives in `repository_source_gates`.

#[test]
fn the_frozen_skill_is_where_the_tests_expect_it() {
    assert!(aidp_testkit::fixtures::templates_dir().is_dir());
    assert!(aidp_testkit::fixtures::mocks_dir().is_dir());
    assert!(aidp_testkit::fixtures::template_inputs().is_file());
}
