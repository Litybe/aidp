use super::CommandOutput;
use crate::args::SkillRulesOp;
use crate::ctx::Ctx;
use aidp_core::process_runner::ProcessSpec;
use std::path::Path;

const GIT_PROGRAM: &str = "git";
const MERGE_BASE_ARGS: [&str; 1] = ["merge-base"];
const HEAD_REF: &str = "HEAD";
const DIFF_ARGS: [&str; 3] = ["diff", "--unified=0", "--no-color"];
const EMPTY_RUN_ID: &str = "";

/// `aidp skill-rules check`: writes the findings.json of the rule scan to `--out`.
pub fn run(ctx: &Ctx, op: SkillRulesOp) -> anyhow::Result<CommandOutput> {
    match op {
        SkillRulesOp::Check {
            base,
            skill,
            repo,
            diff_file,
            out,
            run_id,
        } => {
            let diff = match diff_file {
                Some(path) => std::fs::read_to_string(&path)
                    .map_err(|error| anyhow::anyhow!("read {}: {error}", path.display()))?,
                None => git_diff(ctx, repo.as_deref().unwrap_or(&ctx.cwd), &base)?,
            };
            let violations = aidp_core::skill_rules::scan_diff(&diff, &skill);
            let findings = aidp_core::skill_rules::to_findings(
                &violations,
                run_id.as_deref().unwrap_or(EMPTY_RUN_ID),
                &ctx.clock.now_rfc3339(),
            );
            write_findings(&out, &findings)?;
            Ok(CommandOutput::text(out.display().to_string()))
        }
    }
}

/// Runs git in `repo` and returns its stdout, failing on a non-zero exit.
fn git_output(ctx: &Ctx, repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let spec = ProcessSpec::new(GIT_PROGRAM)
        .args(args.iter().copied())
        .cwd(repo);
    let output = ctx
        .runner
        .output(&spec)
        .map_err(|error| anyhow::anyhow!("run git: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The unified diff from the merge-base of `base` and HEAD to the working tree.
fn git_diff(ctx: &Ctx, repo: &Path, base: &str) -> anyhow::Result<String> {
    let mut merge_base_args: Vec<&str> = MERGE_BASE_ARGS.to_vec();
    merge_base_args.extend([base, HEAD_REF]);
    let merge_base = git_output(ctx, repo, &merge_base_args)?;
    let mut diff_args: Vec<&str> = DIFF_ARGS.to_vec();
    diff_args.push(merge_base.trim());
    git_output(ctx, repo, &diff_args)
}

fn write_findings(out: &Path, findings: &aidp_core::findings::Findings) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(findings)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, text).map_err(|error| anyhow::anyhow!("write {}: {error}", out.display()))
}
