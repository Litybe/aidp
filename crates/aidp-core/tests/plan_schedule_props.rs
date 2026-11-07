//! Property-based tests for plan::schedule invariants.
//!
//! These tests verify three core properties:
//! 1. Topological order: each task's wave is strictly greater than the waves of its dependencies.
//! 2. Intra-wave write-set disjointness: any two tasks in the same wave have disjoint write-sets.
//! 3. Purity: calling schedule twice on the same plan yields identical results.

use aidp_core::plan::{self, Plan, Task};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

// ============================================================================
// Shared DAG generator: 1-8 tasks with acyclic needs
// ============================================================================

/// A file alphabet for write-sets (4 distinct paths).
const FILE_ALPHABET: &[&str] = &["src/foo.rs", "tests/bar.rs", "docs/baz.md", "config.yaml"];

/// Strategy to generate a task id: T1, T2, ..., Tn.
fn task_id_strategy(index: usize) -> String {
    format!("T{}", index + 1)
}

/// Strategy to generate task titles.
fn task_title_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Za-z0-9 _-]{5,30}")
        .expect("regex is valid")
        .prop_map(|s| format!("Task: {}", s))
}

/// Strategy to generate non-empty write-sets from the file alphabet.
fn write_set_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(0usize..FILE_ALPHABET.len(), 1..=4).prop_map(|indices| {
        // Remove duplicates and create non-empty set of file paths
        let set: BTreeSet<String> = indices
            .into_iter()
            .map(|i| FILE_ALPHABET[i].to_string())
            .collect();
        set.into_iter().collect()
    })
}

/// Strategy to generate a valid plan with 1-8 tasks.
fn plan_strategy() -> impl Strategy<Value = Plan> {
    // Simple approach: generate num_tasks, then for each task generate:
    // - title
    // - write-set
    // - dependency bitmap (which of the earlier tasks to depend on)
    (
        1usize..=8,
        prop::collection::vec(task_title_strategy(), 8..=8),
        prop::collection::vec(write_set_strategy(), 8..=8),
        prop::collection::vec(0u16..256, 8..=8), // dependency bitmap for each task
    )
        .prop_map(|(num_tasks, titles, write_sets, dep_bitmaps)| {
            let mut tasks = Vec::new();
            for i in 0..num_tasks {
                // Generate needs for task i that only point to earlier tasks
                let needs: Vec<String> = if i == 0 {
                    Vec::new()
                } else {
                    // Use the bitmap to decide which earlier tasks to depend on
                    let bitmap = dep_bitmaps[i];
                    (0..i)
                        .filter(|j| (bitmap >> j) & 1 == 1)
                        .map(task_id_strategy)
                        .collect()
                };

                tasks.push(Task {
                    id: task_id_strategy(i),
                    title: titles[i % titles.len()].clone(),
                    kind: "code".to_string(),
                    prompt: format!("Task {} prompt", i + 1),
                    instructions: Vec::new(),
                    needs,
                    files: write_sets[i % write_sets.len()].clone(),
                    reads: Vec::new(),
                    verification: String::new(),
                    acceptance: Vec::new(),
                    jira_keys: Vec::new(),
                    risk: None,
                    outputs: Vec::new(),
                    notes: String::new(),
                });
            }
            Plan {
                schema: Some("aidp-plan.v1".to_string()),
                summary: "Test plan".to_string(),
                jira_keys: Vec::new(),
                shared_files: Vec::new(),
                risks: Vec::new(),
                open_questions: Vec::new(),
                tasks,
            }
        })
}

// ============================================================================
// Property 1: Topological order
// ============================================================================

/// Normalize a file path the same way plan.rs does.
fn normalize_path(p: &str) -> String {
    let t = p.trim().replace('\\', "/");
    let mut t = t.as_str();
    while let Some(rest) = t.strip_prefix("./") {
        t = rest;
    }
    t.trim_matches('/').to_ascii_lowercase()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_schedule_topological_order(plan in plan_strategy()) {
        // Schedule the plan with max_parallel=4 and no kind filter.
        let schedule = plan::schedule(&plan, 4, &[])
            .expect("schedule must succeed on a valid plan");

        // Build a map from task id to its wave.
        let mut task_to_wave = BTreeMap::new();
        for (wave_idx, wave) in schedule.waves.iter().enumerate() {
            for task_id in wave {
                task_to_wave.insert(task_id.clone(), wave_idx);
            }
        }

        // For each scheduled task, verify topological order.
        for scheduled_task in &schedule.tasks {
            let task_id = &scheduled_task.task.id;
            let task_wave = scheduled_task.wave;

            // Check that all deps have strictly lower waves.
            for dep_id in &scheduled_task.deps {
                let dep_wave = task_to_wave
                    .get(dep_id)
                    .copied()
                    .unwrap_or_else(|| panic!("dep {dep_id} must be in the schedule"));
                prop_assert!(
                    task_wave > dep_wave,
                    "task {} (wave {}) must have wave > wave of dep {} (wave {})",
                    task_id,
                    task_wave,
                    dep_id,
                    dep_wave
                );
            }

            // Check that all mutex_after have strictly lower waves.
            for mutex_id in &scheduled_task.mutex_after {
                let mutex_wave = task_to_wave
                    .get(mutex_id)
                    .copied()
                    .unwrap_or_else(|| panic!("mutex_after {mutex_id} must be in the schedule"));
                prop_assert!(
                    task_wave > mutex_wave,
                    "task {} (wave {}) must have wave > wave of mutex_after {} (wave {})",
                    task_id,
                    task_wave,
                    mutex_id,
                    mutex_wave
                );
            }
        }
    }

    // ========================================================================
    // Property 2: Intra-wave write-set disjointness
    // ========================================================================

    #[test]
    fn prop_schedule_intra_wave_disjointness(plan in plan_strategy()) {
        let schedule = plan::schedule(&plan, 4, &[])
            .expect("schedule must succeed on a valid plan");

        // Group tasks by wave.
        let mut tasks_by_wave: BTreeMap<usize, Vec<&aidp_core::plan::ScheduledTask>> =
            BTreeMap::new();
        for task in &schedule.tasks {
            tasks_by_wave.entry(task.wave).or_default().push(task);
        }

        // For each wave, check that write-sets are disjoint.
        for (wave_idx, wave_tasks) in tasks_by_wave {
            for i in 0..wave_tasks.len() {
                for j in (i + 1)..wave_tasks.len() {
                    let task_i = wave_tasks[i];
                    let task_j = wave_tasks[j];

                    // Normalize write-sets.
                    let files_i: BTreeSet<String> =
                        task_i.task.files.iter().map(|f| normalize_path(f)).collect();
                    let files_j: BTreeSet<String> =
                        task_j.task.files.iter().map(|f| normalize_path(f)).collect();

                    // Check disjointness.
                    let intersection: Vec<_> =
                        files_i.intersection(&files_j).cloned().collect();
                    prop_assert!(
                        intersection.is_empty(),
                        "tasks {} and {} in wave {} have overlapping write-sets: {:?}",
                        task_i.task.id,
                        task_j.task.id,
                        wave_idx,
                        intersection
                    );
                }
            }
        }
    }

    // ========================================================================
    // Property 3: Purity
    // ========================================================================

    #[test]
    fn prop_schedule_purity(plan in plan_strategy()) {
        // Call schedule twice on the same plan.
        let schedule1 = plan::schedule(&plan, 4, &[])
            .expect("first schedule must succeed");
        let schedule2 = plan::schedule(&plan, 4, &[])
            .expect("second schedule must succeed");

        // Verify identical waves.
        prop_assert_eq!(
            schedule1.waves, schedule2.waves,
            "schedule waves must be identical across two calls"
        );

        // Verify identical serializations.
        prop_assert_eq!(
            schedule1.serializations, schedule2.serializations,
            "schedule serializations must be identical across two calls"
        );

        // Verify identical task order and dependencies.
        prop_assert_eq!(
            schedule1.tasks.len(),
            schedule2.tasks.len(),
            "number of scheduled tasks must be identical"
        );

        for (i, (task1, task2)) in schedule1.tasks.iter().zip(schedule2.tasks.iter()).enumerate() {
            prop_assert_eq!(
                &task1.task.id, &task2.task.id,
                "task {} id must be identical",
                i
            );
            prop_assert_eq!(
                &task1.deps, &task2.deps,
                "task {} deps must be identical",
                &task1.task.id
            );
            prop_assert_eq!(
                &task1.mutex_after, &task2.mutex_after,
                "task {} mutex_after must be identical",
                &task1.task.id
            );
            prop_assert_eq!(
                task1.wave, task2.wave,
                "task {} wave must be identical",
                &task1.task.id
            );
        }
    }
}
