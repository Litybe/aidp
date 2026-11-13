//! Fixture git repositories built once per machine and copied per test, so a test pays a directory copy and one `git` call instead of a dozen.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const GIT_PROGRAM: &str = "git";
const CACHE_DIR: &str = "aidp-git-fixture";
const TARGET_DIR: &str = "target";
const WORKSPACE_ROOT_FROM_CRATE: &str = "../..";
const BUILD_PREFIX: &str = "build-";
const WORKSPACE_DIR: &str = "workspace";
const ORIGIN_DIR: &str = "origin.git";
const ORIGIN_REMOTE: &str = "origin";
const INITIAL_MESSAGE: &str = "initial";
const FORMAT_VERSION: &str = "git-fixture-v1";
const AUTOCRLF_KEY: &str = "core.autocrlf";
const AUTOCRLF_OFF: &str = "false";
const IDENTITY_EMAIL_KEY: &str = "user.email";
const IDENTITY_NAME_KEY: &str = "user.name";

/// What a fixture repository holds: its committed files, the empty directories beside them and the identity of its commit.
#[derive(Debug, Clone, Copy, Hash)]
pub struct GitFixture<'a> {
    pub files: &'a [(&'a str, &'a str)],
    pub empty_directories: &'a [&'a str],
    pub branch: Option<&'a str>,
    pub user_name: &'a str,
    pub user_email: &'a str,
    pub autocrlf_off: bool,
}

impl GitFixture<'_> {
    /// Copies the repository into `workspace`, an existing directory.
    pub fn copy_into(&self, workspace: &Path) {
        copy_directory(&self.template(false).join(WORKSPACE_DIR), workspace)
            .expect("the fixture repository is copied");
    }

    /// Copies the repository into `workspace` and its pushed bare remote into `origin`, then points `origin` at the copy.
    pub fn copy_with_origin_into(&self, workspace: &Path, origin: &Path) {
        let template = self.template(true);
        copy_directory(&template.join(WORKSPACE_DIR), workspace)
            .expect("the fixture repository is copied");
        copy_directory(&template.join(ORIGIN_DIR), origin).expect("the fixture remote is copied");
        git(
            workspace,
            &[
                "remote",
                "set-url",
                ORIGIN_REMOTE,
                origin.to_str().expect("the origin path is text"),
            ],
        );
    }

    fn template(&self, with_origin: bool) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        FORMAT_VERSION.hash(&mut hasher);
        self.hash(&mut hasher);
        with_origin.hash(&mut hasher);
        let cache_root = cache_root();
        fs::create_dir_all(&cache_root).expect("the fixture cache directory is created");
        let final_path = cache_root.join(format!("{:016x}", hasher.finish()));
        if final_path.join(WORKSPACE_DIR).is_dir() {
            return final_path;
        }
        let building = tempfile::Builder::new()
            .prefix(BUILD_PREFIX)
            .tempdir_in(&cache_root)
            .expect("the fixture build directory is created");
        self.build(building.path(), with_origin);
        let built = building.keep();
        if fs::rename(&built, &final_path).is_err() {
            let _ = fs::remove_dir_all(&built);
        }
        final_path
    }

    fn build(&self, root: &Path, with_origin: bool) {
        let workspace = root.join(WORKSPACE_DIR);
        fs::create_dir_all(&workspace).expect("the fixture workspace is created");
        for directory in self.empty_directories {
            fs::create_dir_all(workspace.join(directory)).expect("a fixture directory is created");
        }
        for (relative, contents) in self.files {
            let file = workspace.join(relative);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).expect("a fixture parent is created");
            }
            fs::write(&file, contents).expect("a fixture file is written");
        }
        match self.branch {
            Some(branch) => git(&workspace, &["init", "-q", "-b", branch]),
            None => git(&workspace, &["init", "-q"]),
        }
        git(&workspace, &["config", IDENTITY_EMAIL_KEY, self.user_email]);
        git(&workspace, &["config", IDENTITY_NAME_KEY, self.user_name]);
        if self.autocrlf_off {
            git(&workspace, &["config", AUTOCRLF_KEY, AUTOCRLF_OFF]);
        }
        git(&workspace, &["add", "."]);
        git(&workspace, &["commit", "-q", "-m", INITIAL_MESSAGE]);
        if with_origin {
            let branch = self
                .branch
                .expect("a fixture with a remote names its branch");
            let origin = root.join(ORIGIN_DIR);
            fs::create_dir_all(&origin).expect("the fixture remote is created");
            git(&origin, &["init", "-q", "--bare", "-b", branch]);
            git(
                &workspace,
                &[
                    "remote",
                    "add",
                    ORIGIN_REMOTE,
                    origin.to_str().expect("the origin path is text"),
                ],
            );
            git(&workspace, &["push", "-q", "-u", ORIGIN_REMOTE, branch]);
            git(&workspace, &["remote", "set-head", ORIGIN_REMOTE, branch]);
        }
    }
}

fn cache_root() -> PathBuf {
    let target = match option_env!("CARGO_TARGET_DIR") {
        Some(directory) => PathBuf::from(directory),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(WORKSPACE_ROOT_FROM_CRATE)
            .join(TARGET_DIR),
    };
    target.join(CACHE_DIR)
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new(GIT_PROGRAM)
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
