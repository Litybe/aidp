use super::*;

const TWICE: u8 = 2;

/// A context built with default flags reports no verbosity.
#[test]
fn default_globals_give_zero_verbosity() {
    let ctx = Ctx::for_test(PathBuf::new(), None);
    assert_eq!(ctx.verbosity(), 0);
}

/// The verbosity accessor returns the count stored in the global flags.
#[test]
fn verbosity_returns_the_count_from_globals() {
    let mut ctx = Ctx::for_test(PathBuf::new(), None);
    ctx.globals.verbose = TWICE;
    assert_eq!(ctx.verbosity(), TWICE);
}

const REPO_DIR: &str = "repo";
const NESTED_DIR: &str = "nested";
const SIBLING_DIR: &str = "sibling";
const CURRENT: &str = ".";
const PARENT: &str = "..";

fn repo_dir() -> PathBuf {
    std::env::temp_dir().join(REPO_DIR)
}

/// A relative path is resolved against the current directory of the context.
#[test]
fn absolute_joins_a_relative_path_to_the_current_directory() {
    let ctx = Ctx::for_test(repo_dir(), None);
    assert_eq!(
        ctx.absolute(Path::new(NESTED_DIR)),
        repo_dir().join(NESTED_DIR)
    );
}

/// A bare `.` resolves to the current directory itself, with no `.` component left.
#[test]
fn absolute_turns_the_current_directory_marker_into_the_current_directory() {
    let ctx = Ctx::for_test(repo_dir(), None);
    assert_eq!(ctx.absolute(Path::new(CURRENT)), repo_dir());
}

/// `..` components are folded away, as the daemon refuses a path that keeps one.
#[test]
fn absolute_folds_parent_components() {
    let ctx = Ctx::for_test(repo_dir().join(NESTED_DIR), None);
    let relative = Path::new(PARENT).join(SIBLING_DIR);
    assert_eq!(ctx.absolute(&relative), repo_dir().join(SIBLING_DIR));
}

/// An absolute path is kept as it is.
#[test]
fn absolute_keeps_an_absolute_path() {
    let ctx = Ctx::for_test(repo_dir(), None);
    let elsewhere = std::env::temp_dir().join(SIBLING_DIR);
    assert_eq!(ctx.absolute(&elsewhere), elsewhere);
}
