#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use tempfile::TempDir;

thread_local! {
    static SCRATCH_DIRS: RefCell<Vec<TempDir>> = const { RefCell::new(Vec::new()) };
}

/// A fresh temporary directory owned by the current test thread and removed when the test ends.
pub fn scratch_dir() -> PathBuf {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();
    SCRATCH_DIRS.with(|dirs| dirs.borrow_mut().push(dir));
    path
}

pub const GROUPED: &str = r#"
version: 5
id: grouped
stages:
  - name: discover
    agent: analyzer
    outputs: [context.md]
  - name: code
    agent: executor
    needs: [discover]
    outputs: [changed-files.json]
"#;

/// A vault + a git-backed workspace, the shape v7 actually runs in.
pub fn vault_fixture() -> (PathBuf, PathBuf, aidp_core::vault::Vault) {
    let root = scratch_dir();
    let vault_root = root.join("AIDP");
    std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
    let ws = root.join("code").join("checkout");
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    std::fs::write(
        ws.join(".git").join("config"),
        "[remote \"origin\"]\n\turl = git@github.com:org/cheat-alerts-service.git\n",
    )
    .unwrap();
    let vault = aidp_core::vault::Vault {
        root: vault_root,
        source: aidp_core::vault::VaultSource::Flag,
        ambiguity: None,
    };
    (root, ws, vault)
}

pub const VAULT_PIPE: &str = r#"
version: 5
id: dev-csharp
stages:
  - name: explore
    agent: analyzer
    outputs: [context.md]
  - name: implement
    agent: executor
    needs: [explore]
    outputs: [changed-files.json]
"#;
