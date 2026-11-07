use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A temporary workspace directory that is automatically cleaned up.
///
/// `TempWorkspace` wraps `tempfile::TempDir` and creates directories with
/// readable tags and random suffixes to aid in debugging concurrent tests
/// and ensure uniqueness across processes.
pub struct TempWorkspace {
    inner: TempDir,
}

impl TempWorkspace {
    /// Creates a new temporary workspace with the given tag.
    ///
    /// The directory name will be in the format `aidp-{tag}-{random}`, ensuring
    /// uniqueness both within a single process and across concurrent processes.
    ///
    /// # Panics
    ///
    /// Panics if the temporary directory cannot be created.
    pub fn new(tag: &str) -> TempWorkspace {
        let prefix = format!("aidp-{tag}-");
        let inner = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir()
            .unwrap_or_else(|e| {
                panic!("Failed to create temp workspace with tag '{}': {}", tag, e)
            });
        TempWorkspace { inner }
    }

    /// Returns the path to the workspace directory.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Returns a path to a child of the workspace directory.
    pub fn child(&self, rel: &str) -> PathBuf {
        self.path().join(rel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    #[test]
    fn temp_workspace_is_removed_on_panic() {
        let path = {
            let workspace = TempWorkspace::new("test");
            let p = workspace.path().to_path_buf();

            // Verify the directory exists before panicking
            assert!(p.exists(), "Workspace should exist at {:?}", p);

            // Panic inside a catch_unwind to test Drop behavior
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                panic!("Intentional panic for testing")
            }));

            assert!(result.is_err(), "Expected panic to occur");

            p
        };

        // After TempWorkspace is dropped, the directory should be gone
        assert!(
            !path.exists(),
            "Workspace directory should be removed after drop, but found at {:?}",
            path
        );
    }

    #[test]
    fn two_workspaces_with_same_tag_differ() {
        let workspace1 = TempWorkspace::new("base");
        let workspace2 = TempWorkspace::new("base");

        let path1 = workspace1.path();
        let path2 = workspace2.path();

        // Paths should be different due to random suffixes
        assert_ne!(
            path1, path2,
            "Two workspaces with the same tag should have different paths"
        );

        // Both should exist simultaneously
        assert!(path1.exists(), "First workspace should exist");
        assert!(path2.exists(), "Second workspace should exist");
    }
}
