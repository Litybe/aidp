//! Resolve an executable on `PATH` without ever invoking a shell. Lives in
//! `aidp-core` so crates that cannot depend on `aidp-integrations` share one
//! implementation; `aidp-integrations` re-exports it unchanged.

/// Resolve `name` against `PATH` x `PATHEXT` (Windows) / `PATH` (elsewhere) and
/// return the first matching executable's absolute path. No shell is ever
/// invoked to do this resolution - a `cmd /C` fallback would hand Windows'
/// metacharacter interpretation (`&`, `|`, `^`, `<`, `>`) to argv content that
/// Rust otherwise escapes safely under MSVCRT rules.
pub fn resolve_program(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };
    resolve_program_in(&path_var, &exts, name)
}

/// Pure, environment-free core of [`resolve_program`], kept separate so tests
/// can drive it without mutating `PATH`/`PATHEXT`. Empty `PATH` entries are
/// skipped: joined onto `name` they would yield a relative path resolved in
/// the current directory, so a resolution can never return a relative path.
pub fn resolve_program_in(
    path_var: &std::ffi::OsStr,
    exts: &[String],
    name: &str,
) -> Option<std::path::PathBuf> {
    let has_ext = std::path::Path::new(name).extension().is_some();
    for dir in std::env::split_paths(path_var).filter(|d| !d.as_os_str().is_empty()) {
        if cfg!(windows) {
            if has_ext {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
            for ext in exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        } else {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_exts() -> Vec<String> {
        if cfg!(windows) {
            vec![".EXE".to_string()]
        } else {
            Vec::new()
        }
    }

    #[test]
    fn resolve_program_in_skips_empty_path_entries() {
        let cwd_standin = tempfile::tempdir().unwrap();
        let bin = if cfg!(windows) { "gh.exe" } else { "gh" };
        std::fs::write(cwd_standin.path().join(bin), b"").unwrap();
        let real = cwd_standin.path().join("empty-dir");
        std::fs::create_dir(&real).unwrap();
        let real = real.to_string_lossy().into_owned();
        #[cfg(windows)]
        let sep = ";";
        #[cfg(not(windows))]
        let sep = ":";
        let cases = [
            format!("{sep}{real}"),
            format!("{real}{sep}{sep}{real}"),
            format!("{real}{sep}"),
            format!("{sep}{sep}"),
        ];
        let exts = test_exts();
        for case in &cases {
            let got = resolve_program_in(std::ffi::OsStr::new(case), &exts, "gh");
            if let Some(p) = got {
                assert!(p.is_absolute(), "relative path {p:?} for PATH {case:?}");
                assert_ne!(p, std::path::PathBuf::from(bin));
            }
            // `cargo test` runs from the crate dir, which holds `Cargo.toml`: an
            // unfiltered empty entry would resolve it as the relative `Cargo.toml`.
            let cwd_hit =
                resolve_program_in(std::ffi::OsStr::new(case), &[".toml".to_string()], "Cargo");
            assert_eq!(cwd_hit, None, "cwd file resolved for PATH {case:?}");
        }
    }

    #[test]
    fn resolve_program_in_finds_file_in_listed_dir() {
        let dir = tempfile::tempdir().unwrap();
        let bin = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(dir.path().join(bin), b"").unwrap();
        let got = resolve_program_in(dir.path().as_os_str(), &test_exts(), "tool")
            .expect("tool should resolve");
        assert!(got.is_absolute());
        let file = got.file_name().unwrap().to_string_lossy().to_lowercase();
        assert_eq!(file, bin);
    }

    #[test]
    fn resolve_program_finds_cargo() {
        let p = resolve_program("cargo").expect("cargo should be resolvable on PATH");
        assert!(p.is_absolute());
        assert!(p.is_file());
    }

    #[test]
    fn resolve_program_returns_none_for_missing_binary() {
        assert!(resolve_program("zzz-inexistant-binary-aidp").is_none());
    }
}
