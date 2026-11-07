//! Vault artifact leak detection.
//!
//! When a vault is configured, all run artifacts must live under `<vault>/Pipelines/` or
//! `<vault>/Knowledge/`, never in the repository tree. This module detects when a sub-agent
//! mistakenly writes a vault-shaped relative path like `Pipelines/<repo>/<feature>/<run>/<stage>/<file>`,
//! which then gets resolved against the repo's working directory instead of the vault.

use serde::{Deserialize, Serialize};

/// A vault artifact that was mistakenly written to the repository instead of the vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultLeak {
    /// The relative path that was written to the repo.
    pub path: String,
    /// What kind of artifact this is.
    pub kind: LeakKind,
    /// Human-readable explanation of the vault rule that was broken.
    pub reason: String,
}

/// Classification of a vault leak by artifact type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LeakKind {
    /// A run-stage output (Pipelines/...)
    RunArtifact,
    /// A knowledge base note (Knowledge/...)
    KnowledgeNote,
    /// Legacy pipeline artifact (.aidp/pipelines/... or .aidp/knowledge/...)
    LegacyRunArtifact,
}

/// Detects vault artifacts that were mistakenly written into the repository.
///
/// When a vault is configured, this function classifies paths whose leading component is
/// `Pipelines`, `Knowledge`, or `.aidp/pipelines` as artifacts that should have gone to the vault.
/// If no vault is configured (legacy mode), returns empty — legacy mode writes into the repo on purpose.
///
/// Returns a vector of detected leaks, classified by type.
pub fn detect(paths: &[String], vault_configured: bool) -> Vec<VaultLeak> {
    if !vault_configured {
        return Vec::new();
    }

    paths
        .iter()
        .filter_map(|path| classify_path(path))
        .collect()
}

/// Classifies a single path as a vault leak if it matches the pattern.
fn classify_path(path: &str) -> Option<VaultLeak> {
    // Normalize separators and split into components
    let normalized = path.replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').collect();

    if components.is_empty() {
        return None;
    }

    let first_component = components[0];

    // Check for leading component matches (case-insensitive)
    if first_component.eq_ignore_ascii_case("Pipelines") {
        return Some(VaultLeak {
            path: path.to_string(),
            kind: LeakKind::RunArtifact,
            reason: "run artefacts belong under <vault>/Pipelines/, never in the repo tree"
                .to_string(),
        });
    }

    if first_component.eq_ignore_ascii_case("Knowledge") {
        return Some(VaultLeak {
            path: path.to_string(),
            kind: LeakKind::KnowledgeNote,
            reason: "knowledge notes belong under <vault>/Knowledge/, never in the repo tree"
                .to_string(),
        });
    }

    // Check for .aidp/pipelines or .aidp/knowledge prefix
    if (first_component == ".aidp") && components.len() > 1 {
        let second = components[1];
        if second.eq_ignore_ascii_case("pipelines") || second.eq_ignore_ascii_case("knowledge") {
            return Some(VaultLeak {
                path: path.to_string(),
                kind: LeakKind::LegacyRunArtifact,
                reason: "legacy artefacts belong under <vault>/.aidp/, never in the repo tree when a vault is configured".to_string(),
            });
        }
    }

    None
}

/// Generates a human-readable summary of vault leaks.
pub fn describe(leaks: &[VaultLeak]) -> String {
    let count = leaks.len();
    let examples: Vec<String> = leaks.iter().take(3).map(|l| l.path.clone()).collect();

    if count == 1 {
        format!("1 artefact: {}", examples[0])
    } else if count <= 3 {
        format!("{} artefacts: {}", count, examples.join(", "))
    } else {
        format!(
            "{} artefacts: {} (and {} more)",
            count,
            examples.join(", "),
            count - 3
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_empty_when_vault_not_configured() {
        let paths = vec![
            "Pipelines/ai-dev-pipeline-v7/loc-46/20260922-120724-7dd8/implement/file.md"
                .to_string(),
            "Knowledge/ai-dev-pipeline-v7/architecture.md".to_string(),
        ];

        let leaks = detect(&paths, false);
        assert!(leaks.is_empty());
    }

    #[test]
    fn test_detect_pipelines_path() {
        let paths = vec![
            "Pipelines/ai-dev-pipeline-v7/LOC-12/20260918-091530-03b0/implement/T3.md".to_string(),
        ];
        let leaks = detect(&paths, true);

        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].kind, LeakKind::RunArtifact);
        assert_eq!(
            leaks[0].path,
            "Pipelines/ai-dev-pipeline-v7/LOC-12/20260918-091530-03b0/implement/T3.md"
        );
    }

    #[test]
    fn test_detect_backslash_pipelines_path() {
        let paths = vec!["Pipelines\\ai-dev-pipeline-v7\\implement\\file.md".to_string()];
        let leaks = detect(&paths, true);

        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].kind, LeakKind::RunArtifact);
    }

    #[test]
    fn test_detect_knowledge_path() {
        let paths = vec!["Knowledge/ai-dev-pipeline-v7/architecture.md".to_string()];
        let leaks = detect(&paths, true);

        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].kind, LeakKind::KnowledgeNote);
    }

    #[test]
    fn test_detect_legacy_pipelines_path() {
        let paths = vec![".aidp/pipelines/x/y.json".to_string()];
        let leaks = detect(&paths, true);

        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].kind, LeakKind::LegacyRunArtifact);
    }

    #[test]
    fn test_detect_legacy_knowledge_path() {
        let paths = vec![".aidp/knowledge/doc.md".to_string()];
        let leaks = detect(&paths, true);

        assert_eq!(leaks.len(), 1);
        assert_eq!(leaks[0].kind, LeakKind::LegacyRunArtifact);
    }

    #[test]
    fn test_no_false_positive_for_source_file() {
        let paths = vec!["src/Pipelines/Runner.cs".to_string()];
        let leaks = detect(&paths, true);

        assert!(
            leaks.is_empty(),
            "src/Pipelines/... is a source file, not a leak"
        );
    }

    #[test]
    fn test_no_false_positive_for_rust_module() {
        let paths = vec!["rust/crates/aidp-core/src/pipeline.rs".to_string()];
        let leaks = detect(&paths, true);

        assert!(leaks.is_empty(), "pipeline.rs module is not a leak");
    }

    #[test]
    fn test_no_false_positive_for_absolute_vault_path() {
        let paths =
            vec!["C:/CODE/AIDP/Pipelines/ai-dev-pipeline-v7/loc-46/implement/file.md".to_string()];
        let leaks = detect(&paths, true);

        assert!(leaks.is_empty(), "absolute vault paths are not repo leaks");
    }

    #[test]
    fn test_describe_single_leak() {
        let leaks = vec![VaultLeak {
            path: "Pipelines/test.md".to_string(),
            kind: LeakKind::RunArtifact,
            reason: "test".to_string(),
        }];

        let summary = describe(&leaks);
        assert!(summary.contains("1 artefact"));
        assert!(summary.contains("Pipelines/test.md"));
    }

    #[test]
    fn test_describe_multiple_leaks() {
        let leaks = vec![
            VaultLeak {
                path: "Pipelines/a.md".to_string(),
                kind: LeakKind::RunArtifact,
                reason: "test".to_string(),
            },
            VaultLeak {
                path: "Pipelines/b.md".to_string(),
                kind: LeakKind::RunArtifact,
                reason: "test".to_string(),
            },
            VaultLeak {
                path: "Knowledge/c.md".to_string(),
                kind: LeakKind::KnowledgeNote,
                reason: "test".to_string(),
            },
        ];

        let summary = describe(&leaks);
        assert!(summary.contains("3 artefacts"));
    }
}
