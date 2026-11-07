use aidp_core::clock::{Clock, SystemClock};
use aidp_core::git::{Git, GitCli};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_core::Format;
use std::path::{Component, Path, PathBuf};

/// Environment variable naming the skill root that holds the pipeline templates.
pub const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";

const DEV_TEMPLATE_RELATIVE: &str = "templates/dev.yaml";
/// Directory of the pipeline templates under the skill root.
pub const TEMPLATES_DIR: &str = "templates";
/// Directory of the agent personas under the skill root.
pub const AGENTS_DIR: &str = "agents";
/// Directory of the JSON schemas under the skill root.
/// Per-user Claude Code configuration directory under the home directory.
pub const CLAUDE_DIR: &str = ".claude";
const PROJECT_FLOW_DIR: &str = ".aidp/pipeline/flow";
const SKILL_MANIFEST: &str = "SKILL.md";
const SKILL_ROOT_ANCESTOR_DEPTHS: std::ops::Range<usize> = 2..7;

/// The flags declared once on the top-level command, valid before or after any verb.
#[derive(Clone, Default)]
pub struct GlobalFlags {
    /// `--workspace-root`: the workspace to act on instead of the current directory.
    pub workspace_root: Option<PathBuf>,
    /// `--vault`: the Obsidian vault to use instead of the resolved one.
    pub vault: Option<PathBuf>,
    /// `--no-vault`: ignore any vault and use the in-repo layout.
    pub no_vault: bool,
    /// `--format`: the output format, `None` leaving each command its own default.
    pub format: Option<Format>,
    /// `--format footer`: the cost footer line instead of the report.
    pub footer: bool,
    /// `-v`: how many times the verbosity flag was given.
    pub verbose: u8,
}

/// Everything a command reads from its surroundings, built once in `main`.
/// The vault is resolved lazily per command from the global flags.
pub struct Ctx {
    pub globals: GlobalFlags,
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub skill_root: Option<PathBuf>,
    pub explicit_skill_root: Option<PathBuf>,
    pub clock: Box<dyn Clock>,
    pub git: Box<dyn Git>,
    pub runner: Box<dyn ProcessRunner>,
}

impl Ctx {
    /// Builds the context from the real environment. The only reader of the current directory.
    pub fn new(globals: GlobalFlags) -> anyhow::Result<Self> {
        let explicit_skill_root = std::env::var_os(SKILL_ROOT_ENV).map(PathBuf::from);
        let skill_root = explicit_skill_root.clone().or_else(probe_skill_root);
        Ok(Self {
            globals,
            cwd: std::env::current_dir()?,
            home: aidp_core::vault::home_dir(),
            skill_root,
            explicit_skill_root,
            clock: Box::new(SystemClock),
            git: Box::new(GitCli),
            runner: Box::new(OsProcessRunner),
        })
    }

    /// The `--workspace-root` flag when given, the current directory otherwise.
    pub fn cwd_or(&self, workspace_root: Option<PathBuf>) -> PathBuf {
        workspace_root.unwrap_or_else(|| self.cwd.clone())
    }

    /// The workspace every command acts on: `--workspace-root`, else the current directory.
    pub fn workspace(&self) -> PathBuf {
        self.cwd_or(self.globals.workspace_root.clone())
    }

    /// `path` made absolute against the current directory, with `.` and `..` folded away.
    /// For a path handed to the daemon, whose own current directory is not this one.
    pub fn absolute(&self, path: &Path) -> PathBuf {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let mut folded = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    folded.pop();
                }
                other => folded.push(other.as_os_str()),
            }
        }
        folded
    }

    /// The `--workspace-root` flag exactly as given, for callers that keep their own fallback.
    pub fn workspace_root_flag(&self) -> Option<&Path> {
        self.globals.workspace_root.as_deref()
    }

    /// The `--vault` flag exactly as given.
    pub fn vault_flag(&self) -> Option<&Path> {
        self.globals.vault.as_deref()
    }

    /// The vault to read or write: `None` under `--no-vault`, else the resolved one.
    /// Pass `vault.as_ref()` to `run::load_in` and `run::run_dir_for_in`.
    pub fn vault_override(&self) -> Option<aidp_core::vault::Vault> {
        if self.globals.no_vault {
            return None;
        }
        aidp_core::vault::resolve(self.vault_flag(), &self.workspace())
    }

    /// The `--format` flag, `None` when the command keeps its own default.
    pub fn format(&self) -> Option<Format> {
        self.globals.format
    }

    /// True when `--format footer` was given.
    pub fn footer(&self) -> bool {
        self.globals.footer
    }

    /// How many times `-v` was given on the command line.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the logging setup once it lands")
    )]
    pub fn verbosity(&self) -> u8 {
        self.globals.verbose
    }

    /// The skill root only when `AIDP_SKILL_ROOT` is set, never a probed fallback.
    /// `aidp-tickets` warns when it receives `None`, so probing would hide that warning.
    pub fn explicit_skill_root(&self) -> Option<&Path> {
        self.explicit_skill_root.as_deref()
    }

    /// `<skill root>/templates` and the project-local flow directory, in discovery order.
    pub fn pipeline_roots(&self, workspace: &Path) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(root) = &self.skill_root {
            roots.push(root.join(TEMPLATES_DIR));
        }
        roots.push(workspace.join(PROJECT_FLOW_DIR));
        roots
    }
}

/// Looks for a skill root above the running executable.
fn probe_skill_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    for depth in SKILL_ROOT_ANCESTOR_DEPTHS {
        let Some(ancestor) = exe.ancestors().nth(depth) else {
            continue;
        };
        let has_dev_template = ancestor.join(DEV_TEMPLATE_RELATIVE).exists();
        let has_manifest =
            ancestor.join(SKILL_MANIFEST).exists() && ancestor.join(TEMPLATES_DIR).exists();
        if has_dev_template || has_manifest {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
const TEST_NOW: &str = "2026-01-01T00:00:00Z";

#[cfg(test)]
impl Ctx {
    /// A hermetic context: fixed clock, recording git, scripted runner.
    pub fn for_test(cwd: PathBuf, skill_root: Option<PathBuf>) -> Self {
        use aidp_core::clock::FixedClock;
        use aidp_core::git::RecordingGit;
        use aidp_core::process_runner::ScriptedProcessRunner;
        Self {
            globals: GlobalFlags::default(),
            cwd,
            home: None,
            explicit_skill_root: skill_root.clone(),
            skill_root,
            clock: Box::new(FixedClock::from_rfc3339(TEST_NOW)),
            git: Box::new(RecordingGit::new()),
            runner: Box::new(ScriptedProcessRunner::new()),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/ctx_tests.rs"]
mod ctx_tests;
