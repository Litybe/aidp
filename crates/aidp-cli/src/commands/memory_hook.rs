//! The memory side of an ordinary Claude Code session: context in, journal and lessons out.
//!
//! A hook never fails the session: every error is logged and the hook stays silent.

use super::memory::{knowledge_dir, open, Opened};
use super::CommandOutput;
use crate::args::memory::ProjectArg;
use crate::ctx::Ctx;
use crate::models::constants::memory_hook::{
    DISTILL_LEDGER_FILE, DISTILL_LOG_FILE, DISTILL_MODEL, DISTILL_POLL_MS, DISTILL_REPLY_FILE,
    DISTILL_SCRATCH_DIR, DISTILL_TIMEOUT_SECS, ENV_DISTILLING, ENV_DISTILLING_VALUE,
    ENV_PIPELINE_RUN, EVENT_PROMPT_SUBMIT, EVENT_SESSION_START, GIT_DIR, POWERSHELL_PROGRAM,
};
use crate::models::enums::memory_hook_event::MemoryHookEvent;
use crate::models::hook_payload::HookPayload;
use aidp_core::Format;
use aidp_memory::distill::{distill_prompt, proposals_from_reply, should_distill};
use aidp_memory::models::constants::paths::{MEMORY_DIR, PROPOSALS_FILE};
use aidp_memory::models::constants::sessions::JOURNAL_DIR;
use aidp_memory::models::enums::DistillTrigger;
use aidp_memory::models::session_journal::SessionJournal;
use aidp_memory::session_journal::read_transcript;
use aidp_memory::text::short_hash;
use aidp_memory::MemoryManager;
use anyhow::{bail, Context};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Answers one hook event; silent on any failure and inside pipelines.
pub fn run(ctx: &Ctx, event: MemoryHookEvent) -> CommandOutput {
    if [ENV_PIPELINE_RUN, ENV_DISTILLING]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return CommandOutput::empty();
    }
    match handle(ctx, event) {
        Ok(Some((name, context))) => CommandOutput::json(
            Some(Format::JsonCompact),
            json!({ "hookSpecificOutput": { "hookEventName": name, "additionalContext": context } }),
        ),
        Ok(None) => CommandOutput::empty(),
        Err(error) => {
            tracing::warn!(%error, "memory hook failed");
            CommandOutput::empty()
        }
    }
}

fn handle(ctx: &Ctx, event: MemoryHookEvent) -> anyhow::Result<Option<(&'static str, String)>> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let payload: HookPayload = serde_json::from_str(&raw)?;
    let project = ProjectArg {
        project: Some(payload.cwd.clone()),
    };
    let opened = open(ctx, &project, false)?;
    let manager = MemoryManager::new(&opened.store);
    let key = opened.project.key.as_str();
    let answer = match event {
        MemoryHookEvent::SessionStart => (EVENT_SESSION_START, manager.session_digest(key)?),
        MemoryHookEvent::Prompt => (
            EVENT_PROMPT_SUBMIT,
            manager.prompt_recall(key, payload.prompt.as_deref().unwrap_or_default())?,
        ),
        MemoryHookEvent::Stop => {
            record_session(ctx, &opened, &payload, DistillTrigger::Turn)?;
            return Ok(None);
        }
        MemoryHookEvent::PreCompact | MemoryHookEvent::SessionEnd => {
            record_session(ctx, &opened, &payload, DistillTrigger::Boundary)?;
            return Ok(None);
        }
    };
    Ok((!answer.1.is_empty()).then_some(answer))
}

fn record_session(
    ctx: &Ctx,
    opened: &Opened,
    payload: &HookPayload,
    trigger: DistillTrigger,
) -> anyhow::Result<()> {
    let Some(transcript) = payload.transcript_path.as_deref() else {
        return Ok(());
    };
    if !opened.known_root || !opened.project.root.join(GIT_DIR).exists() {
        return Ok(());
    }
    let journal = read_transcript(Path::new(transcript))?;
    if !journal.worth_keeping() {
        return Ok(());
    }
    let Some(knowledge) = knowledge_dir(ctx, opened) else {
        return Ok(());
    };
    let target = knowledge.join(JOURNAL_DIR).join(journal.relative_path());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, journal.render(&opened.project.key))?;
    launch_distill_if_due(payload, &journal, &target, trigger)
}

fn memory_dir() -> anyhow::Result<PathBuf> {
    Ok(aidp_gateway::paths::state_dir()?.join(MEMORY_DIR))
}

fn launch_distill_if_due(
    payload: &HookPayload,
    journal: &SessionJournal,
    note: &Path,
    trigger: DistillTrigger,
) -> anyhow::Result<()> {
    let ledger_path = memory_dir()?.join(DISTILL_LEDGER_FILE);
    let mut ledger: BTreeMap<String, usize> = std::fs::read_to_string(&ledger_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default();
    let already = ledger.get(&journal.session_id).copied().unwrap_or(0);
    if !should_distill(trigger, journal.prompt_count, already) {
        return Ok(());
    }
    ledger.insert(journal.session_id.clone(), journal.prompt_count);
    std::fs::write(&ledger_path, serde_json::to_vec(&ledger)?)?;
    let mut command = detached_distill_command(&journal.session_id, note, &payload.cwd)?;
    command
        .env(ENV_DISTILLING, ENV_DISTILLING_VALUE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().context("extraction not started")?;
    Ok(())
}

/// A plain child would keep the pipes of the hook open on Windows, so `Start-Process` launches it.
#[cfg(windows)]
fn detached_distill_command(session: &str, note: &Path, cwd: &str) -> anyhow::Result<Command> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let quoted = |text: &str| text.replace('\'', "''");
    let arguments = format!(
        "memory distill --session \"{session}\" --journal \"{}\" --project \"{cwd}\"",
        note.display()
    );
    let script = format!(
        "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList '{}'",
        quoted(&std::env::current_exe()?.to_string_lossy()),
        quoted(&arguments)
    );
    let mut command = Command::new(POWERSHELL_PROGRAM);
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW);
    Ok(command)
}

#[cfg(not(windows))]
fn detached_distill_command(session: &str, note: &Path, cwd: &str) -> anyhow::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["memory", "distill", "--session", session, "--journal"])
        .arg(note)
        .args(["--project", cwd]);
    Ok(command)
}

/// Extracts the durable memories of a journal; the outcome is appended to the extraction log.
pub fn distill(
    ctx: &Ctx,
    journal: &Path,
    session: &str,
    project: &ProjectArg,
) -> anyhow::Result<CommandOutput> {
    let outcome = distill_inner(ctx, journal, session, project);
    let line = match &outcome {
        Ok(report) => format!("{session} ok {report}\n"),
        Err(error) => format!("{session} failed {error:#}\n"),
    };
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(memory_dir()?.join(DISTILL_LOG_FILE))
    {
        let _ = log.write_all(line.as_bytes());
    }
    let report = outcome?;
    Ok(CommandOutput::json(Some(Format::JsonCompact), report))
}

fn distill_inner(
    ctx: &Ctx,
    journal: &Path,
    session: &str,
    project: &ProjectArg,
) -> anyhow::Result<serde_json::Value> {
    let opened = open(ctx, project, false)?;
    let note = std::fs::read_to_string(journal)
        .with_context(|| format!("journal not readable: {}", journal.display()))?;
    let scratch = memory_dir()?
        .join(DISTILL_SCRATCH_DIR)
        .join(short_hash(session));
    std::fs::create_dir_all(&scratch)?;
    let result = (|| {
        let reply = ask_model(&distill_prompt(&note), &scratch)?;
        let target = scratch.join(PROPOSALS_FILE);
        std::fs::write(&target, proposals_from_reply(&reply, session)?)?;
        let report = MemoryManager::new(&opened.store).ingest(&opened.project.key, &target)?;
        Ok(serde_json::to_value(report)?)
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn ask_model(prompt: &str, scratch: &Path) -> anyhow::Result<String> {
    let arguments: Vec<String> = [
        "-p",
        "--model",
        DISTILL_MODEL,
        "--tools",
        "",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--setting-sources",
        "project",
        "--output-format",
        "text",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    let reply_path = scratch.join(DISTILL_REPLY_FILE);
    let mut command = aidp_gateway::sessions::claude_command(
        &arguments,
        &scratch.to_string_lossy(),
        &[(ENV_DISTILLING.to_string(), ENV_DISTILLING_VALUE.to_string())],
    )?;
    command
        .stdout(Stdio::from(std::fs::File::create(&reply_path)?))
        .stderr(Stdio::null());
    let mut child = command.spawn().context("claude not started")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    let deadline = Instant::now() + Duration::from_secs(DISTILL_TIMEOUT_SECS);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("claude exited with {status}");
            }
            return Ok(std::fs::read_to_string(&reply_path)?);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            bail!("claude did not answer within {DISTILL_TIMEOUT_SECS} s");
        }
        std::thread::sleep(Duration::from_millis(DISTILL_POLL_MS));
    }
}
