use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Walk a directory recursively, collecting every file's path relative to the base directory
/// alongside its bytes, so two snapshots can be compared byte for byte to prove a dry run
/// wrote nothing.
///
/// # Returns
/// A BTreeMap where keys are file paths relative to the base directory using `/` separators
/// (never `\` even on Windows), and values are the file contents as bytes.
pub fn snapshot_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if let Ok(bytes) = fs::read(&path) {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_tree_basic() {
        let ws = crate::TempWorkspace::new("snapshot_tree_basic");
        let base = ws.path();

        // Create test files
        fs::create_dir_all(base.join("a")).expect("create subdir a");
        fs::write(base.join("a/b.txt"), b"content of a/b.txt").expect("write a/b.txt");
        fs::write(base.join("c.txt"), b"content of c.txt").expect("write c.txt");

        // First snapshot
        let snap1 = snapshot_tree(base);
        assert_eq!(snap1.len(), 2);
        assert_eq!(snap1.get("a/b.txt"), Some(&b"content of a/b.txt".to_vec()));
        assert_eq!(snap1.get("c.txt"), Some(&b"content of c.txt".to_vec()));

        // Second snapshot of same content is identical
        let snap2 = snapshot_tree(base);
        assert_eq!(snap1, snap2);

        // Modify one file
        fs::write(base.join("a/b.txt"), b"modified content").expect("modify a/b.txt");
        let snap3 = snapshot_tree(base);

        // Third snapshot differs on exactly one key
        assert_eq!(snap3.len(), 2);
        assert_ne!(snap1, snap3);
        assert_eq!(snap3.get("c.txt"), snap1.get("c.txt"));
        assert_ne!(snap3.get("a/b.txt"), snap1.get("a/b.txt"));
    }

    #[test]
    fn test_snapshot_tree_uses_forward_slashes() {
        let ws = crate::TempWorkspace::new("snapshot_tree_slashes");
        let base = ws.path();

        // Create nested structure
        fs::create_dir_all(base.join("x").join("y").join("z")).expect("create nested dirs");
        fs::write(base.join("x/y/z/deep.txt"), b"deep").expect("write deep file");

        let snap = snapshot_tree(base);

        // Keys must use '/' separators, never '\'
        assert!(snap.contains_key("x/y/z/deep.txt"));
        assert!(!snap.iter().any(|(k, _)| k.contains('\\')));
    }
}
