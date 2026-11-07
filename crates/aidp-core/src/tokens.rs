//! Extract token usage from Claude Code sub-agent JSONL session files.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// `input_tokens + output_tokens` ONLY. This is the public output of
    /// `aidp run stage-tokens` — do not change its meaning, a downstream
    /// consumer depends on it. The full billable total is `billable_total`.
    pub total_tokens: u64,
    /// input + output + cache_creation + cache_read. Additive field, does
    /// not replace `total_tokens`.
    #[serde(default)]
    pub billable_total: u64,
    /// Ephemeral 5-minute cache-write volume (part of
    /// `cache_creation_input_tokens`, priced separately — see `cost.rs`).
    #[serde(default)]
    pub cache_creation_5m_tokens: u64,
    /// Ephemeral 1-hour cache-write volume (part of
    /// `cache_creation_input_tokens`, priced separately — see `cost.rs`).
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
}

#[derive(Deserialize)]
struct JsonlLine {
    #[serde(default)]
    message: Option<MessageBlock>,
}

#[derive(Deserialize)]
struct MessageBlock {
    /// Streaming re-emits the same `message.id` 2-4 times as a partial
    /// response fills in; earlier tick counts (input/cache) stay frozen
    /// while output grows. This is the dedup key — last-wins per id.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<UsageBlock>,
}

#[derive(Deserialize, Default, Clone)]
struct UsageBlock {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation: Option<CacheCreationBlock>,
}

#[derive(Deserialize, Default, Clone)]
struct CacheCreationBlock {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

/// Aggregate a JSONL agent transcript's token usage, deduplicating by
/// `message.id` (LAST occurrence wins per id — streaming replays the same
/// id with growing `output_tokens` while input/cache stay frozen; summing
/// every line double/triple/quadruple-counts input and cache).
pub fn extract_from_agent_jsonl(jsonl_path: &Path) -> anyhow::Result<TokenUsage> {
    let file = std::fs::File::open(jsonl_path)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", jsonl_path.display()))?;
    let reader = std::io::BufReader::new(file);

    // last-wins per message.id; lines without an id get a synthetic key
    // derived from their line index so they are never merged with anything.
    let mut by_id: BTreeMap<String, UsageBlock> = BTreeMap::new();

    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonlLine>(&line) else {
            continue;
        };
        let Some(msg) = parsed.message else { continue };
        let Some(u) = msg.usage else { continue };
        let key = msg.id.unwrap_or_else(|| format!("__no_id_{idx}"));
        by_id.insert(key, u);
    }

    let mut usage = TokenUsage::default();
    for u in by_id.values() {
        usage.input_tokens += u.input_tokens;
        usage.output_tokens += u.output_tokens;
        usage.cache_creation_input_tokens += u.cache_creation_input_tokens;
        usage.cache_read_input_tokens += u.cache_read_input_tokens;
        if let Some(cc) = &u.cache_creation {
            usage.cache_creation_5m_tokens += cc.ephemeral_5m_input_tokens;
            usage.cache_creation_1h_tokens += cc.ephemeral_1h_input_tokens;
        }
    }

    // total_tokens = input + output ONLY: preserved for `aidp run
    // stage-tokens`, do not fold cache into it.
    usage.total_tokens = usage.input_tokens + usage.output_tokens;
    usage.billable_total = usage.input_tokens
        + usage.output_tokens
        + usage.cache_creation_input_tokens
        + usage.cache_read_input_tokens;
    Ok(usage)
}

pub fn find_agent_jsonl(
    claude_projects_dir: &Path,
    session_id: &str,
    agent_id: &str,
) -> anyhow::Result<std::path::PathBuf> {
    for entry in std::fs::read_dir(claude_projects_dir)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", claude_projects_dir.display()))?
    {
        let entry = entry?;
        let subagent_dir = entry.path().join(session_id).join("subagents");

        // v4 layout: <session>/subagents/agent-<id>.jsonl
        let flat = subagent_dir.join(format!("agent-{agent_id}.jsonl"));
        if flat.exists() {
            return Ok(flat);
        }

        // v7 (Workflow) layout: <session>/subagents/workflows/wf_<runId>/agent-<id>.jsonl
        let workflows_dir = subagent_dir.join("workflows");
        if let Ok(wf_entries) = std::fs::read_dir(&workflows_dir) {
            for wf in wf_entries.flatten() {
                let candidate = wf.path().join(format!("agent-{agent_id}.jsonl"));
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
    }
    anyhow::bail!(
        "agent JSONL not found for session={session_id} agent={agent_id} in {}",
        claude_projects_dir.display()
    )
}

/// Per-agent usage, attributed to a run/stage/task by scanning the JSONL
/// prompt text (line 0's `message.content`), so it works for any pipeline
/// version whose `prompt.rs`/`workflow_emit.rs` follow the same headers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentUsage {
    pub agent_id: String,
    pub wf_run_id: Option<String>,
    pub session_id: Option<String>,
    pub stage: Option<String>,
    pub task_id: Option<String>,
    pub model: Option<String>,
    pub agent_type: Option<String>,
    pub usage: TokenUsage,
    pub messages: u32,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
}

#[derive(Deserialize)]
struct RootLine {
    #[serde(default)]
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<RootMessage>,
}

#[derive(Deserialize)]
struct RootMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    /// Verified against real transcripts: `message.content` is always a
    /// plain string for these lines, never a list of content blocks.
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    usage: Option<UsageBlock>,
}

/// Encode a filesystem path the way Claude Code's project directory naming
/// does: every character outside `[A-Za-z0-9-]` becomes exactly one `-`,
/// case preserved, WITHOUT collapsing repeats (`C:/CODE` -> `C--CODE`). This
/// is NOT injective (`a.b` and `a-b` collide) — callers must treat a
/// directory-name match as a *candidate* and confirm it via the `cwd` field
/// recorded on every JSONL line.
fn encode_workspace_path(workspace_root: &Path) -> String {
    workspace_root
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn stage_header_regex() -> &'static regex::Regex {
    static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
    RE.get_or_init(|| regex::Regex::new(r"(?m)^# Stage `([^`]+)`").unwrap())
}

fn run_header_regex() -> &'static regex::Regex {
    static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
    RE.get_or_init(|| regex::Regex::new(r"\(run `([^`]+)`\)").unwrap())
}

fn task_header_regex() -> &'static regex::Regex {
    static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
    RE.get_or_init(|| regex::Regex::new(r"(?m)^## Plan task `([^`]+)`").unwrap())
}

/// Parse one `agent-<id>.jsonl` file into an [`AgentUsage`], attributing it
/// to `expected_run_id` via the header/machine-block found in the prompt
/// text on line 0. Returns `Ok(None)` when the file does not belong to
/// `expected_run_id` (or carries no run marker at all). Never panics.
fn parse_agent_file(
    path: &Path,
    agent_id: &str,
    expected_run_id: &str,
) -> anyhow::Result<Option<AgentUsage>> {
    let file =
        std::fs::File::open(path).map_err(|e| anyhow::anyhow!("open {}: {e}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut by_id: BTreeMap<String, UsageBlock> = BTreeMap::new();
    let mut session_id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut stage: Option<String> = None;
    let mut task_id: Option<String> = None;
    let mut run_id: Option<String> = None;
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;
    let mut messages: u32 = 0;

    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<RootLine>(&line) else {
            continue;
        };

        if session_id.is_none() {
            session_id = parsed.session_id.clone();
        }
        if let Some(ts) = &parsed.timestamp {
            if first_ts.is_none() {
                first_ts = Some(ts.clone());
            }
            last_ts = Some(ts.clone());
        }

        if idx == 0 {
            if let Some(msg) = &parsed.message {
                if let Some(content) = &msg.content {
                    // Primary: KEY=value machine block written by prompt.rs.
                    for l in content.lines() {
                        if let Some(v) = l.strip_prefix("AIDP_RUN=") {
                            run_id = Some(v.trim().to_string());
                        } else if let Some(v) = l.strip_prefix("AIDP_STAGE=") {
                            stage = Some(v.trim().to_string());
                        }
                    }
                    // Fallback: header regexes.
                    if run_id.is_none() {
                        if let Some(c) = run_header_regex().captures(content) {
                            run_id = c.get(1).map(|m| m.as_str().to_string());
                        }
                    }
                    if stage.is_none() {
                        if let Some(c) = stage_header_regex().captures(content) {
                            stage = c.get(1).map(|m| m.as_str().to_string());
                        }
                    }
                    if let Some(c) = task_header_regex().captures(content) {
                        task_id = c.get(1).map(|m| m.as_str().to_string());
                    }
                }
            }
        }

        if let Some(msg) = parsed.message {
            messages += 1;
            if model.is_none() {
                model = msg.model.clone();
            }
            if let Some(u) = msg.usage {
                let key = msg.id.unwrap_or_else(|| format!("__no_id_{idx}"));
                by_id.insert(key, u);
            }
        }
    }

    let Some(run_id) = run_id else {
        return Ok(None);
    };
    if run_id != expected_run_id {
        return Ok(None);
    }

    let mut usage = TokenUsage::default();
    for u in by_id.values() {
        usage.input_tokens += u.input_tokens;
        usage.output_tokens += u.output_tokens;
        usage.cache_creation_input_tokens += u.cache_creation_input_tokens;
        usage.cache_read_input_tokens += u.cache_read_input_tokens;
        if let Some(cc) = &u.cache_creation {
            usage.cache_creation_5m_tokens += cc.ephemeral_5m_input_tokens;
            usage.cache_creation_1h_tokens += cc.ephemeral_1h_input_tokens;
        }
    }
    usage.total_tokens = usage.input_tokens + usage.output_tokens;
    usage.billable_total = usage.input_tokens
        + usage.output_tokens
        + usage.cache_creation_input_tokens
        + usage.cache_read_input_tokens;

    // agent_type from the sidecar meta file, when present — free column.
    let agent_type = std::fs::read_to_string(path.with_extension("").with_extension("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("agentType")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });

    Ok(Some(AgentUsage {
        agent_id: agent_id.to_string(),
        wf_run_id: Some(run_id),
        session_id,
        stage,
        task_id,
        model,
        agent_type,
        usage,
        messages,
        first_ts,
        last_ts,
    }))
}

fn agent_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().to_string(); // "agent-<id>"
    stem.strip_prefix("agent-").map(|s| s.to_string())
}

/// Return the [`AgentUsage`] of every `agent-*.jsonl` under `projects_dir`
/// attributed (via its prompt header) to `run_id`. Candidates are filtered by
/// the directory-name encoding of `workspace_root`, then confirmed by the `cwd`
/// on their JSONL lines. Never panics: an unreadable or malformed file is skipped.
pub fn scan_run_agents(
    projects_dir: &Path,
    workspace_root: &Path,
    run_id: &str,
) -> Vec<AgentUsage> {
    scan_run_agents_since(projects_dir, workspace_root, run_id, None)
}

/// Like [`scan_run_agents`], skipping every transcript last written before
/// `run_started_at` (RFC 3339): a transcript untouched since before the run
/// began cannot hold that run's agents. An unparsable timestamp disables the filter.
pub fn scan_run_agents_since(
    projects_dir: &Path,
    workspace_root: &Path,
    run_id: &str,
    run_started_at: Option<&str>,
) -> Vec<AgentUsage> {
    let not_before: Option<std::time::SystemTime> = run_started_at
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc).into());
    let mut out = Vec::new();
    let encoded = encode_workspace_path(workspace_root);
    let workspace_str = workspace_root.to_string_lossy().to_string();

    let Ok(entries) = std::fs::read_dir(projects_dir) else {
        return out;
    };

    let mut candidate_dirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == encoded {
            candidate_dirs.push(entry.path());
        }
    }
    // Fallback: if the exact-encoded directory wasn't found, consider every
    // project dir — cwd confirmation below still filters correctly.
    if candidate_dirs.is_empty() {
        if let Ok(entries) = std::fs::read_dir(projects_dir) {
            for entry in entries.flatten() {
                candidate_dirs.push(entry.path());
            }
        }
    }

    for project_dir in candidate_dirs {
        let Ok(session_entries) = std::fs::read_dir(&project_dir) else {
            continue;
        };
        for session_entry in session_entries.flatten() {
            let subagents_dir = session_entry.path().join("subagents");
            let mut jsonl_files: Vec<PathBuf> = Vec::new();

            // v4 flat layout.
            if let Ok(flat) = std::fs::read_dir(&subagents_dir) {
                for f in flat.flatten() {
                    let p = f.path();
                    if p.extension().is_some_and(|e| e == "jsonl")
                        && p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("agent-"))
                    {
                        jsonl_files.push(p);
                    }
                }
            }
            // v7 workflow layout.
            let workflows_dir = subagents_dir.join("workflows");
            if let Ok(wf_dirs) = std::fs::read_dir(&workflows_dir) {
                for wf in wf_dirs.flatten() {
                    if let Ok(files) = std::fs::read_dir(wf.path()) {
                        for f in files.flatten() {
                            let p = f.path();
                            if p.extension().is_some_and(|e| e == "jsonl")
                                && p.file_name()
                                    .is_some_and(|n| n.to_string_lossy().starts_with("agent-"))
                            {
                                jsonl_files.push(p);
                            }
                        }
                    }
                }
            }

            for jsonl in jsonl_files {
                let Some(agent_id) = agent_id_from_filename(&jsonl) else {
                    continue;
                };

                if let Some(floor) = not_before {
                    let stale = std::fs::metadata(&jsonl)
                        .and_then(|m| m.modified())
                        .is_ok_and(|mtime| mtime < floor);
                    if stale {
                        continue;
                    }
                }

                // Confirm the candidate via cwd on line 0.
                if !confirm_cwd(&jsonl, &workspace_str) {
                    continue;
                }

                match parse_agent_file(&jsonl, &agent_id, run_id) {
                    Ok(Some(usage)) => out.push(usage),
                    Ok(None) => {}
                    Err(_) => {}
                }
            }
        }
    }

    out
}

/// Bytes of a transcript's first line inspected by [`confirm_cwd`]. The first
/// line embeds the whole prompt and can weigh megabytes, but Claude Code writes
/// `cwd` before `message`, so it sits within the first few hundred bytes.
const CWD_PREFIX_BYTES: u64 = 16 * 1024;

fn confirm_cwd(jsonl_path: &Path, workspace_root: &str) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(jsonl_path) else {
        return false;
    };
    let mut prefix = Vec::new();
    if file
        .take(CWD_PREFIX_BYTES)
        .read_to_end(&mut prefix)
        .is_err()
    {
        return false;
    }
    let prefix = String::from_utf8_lossy(&prefix);
    let Some(cwd) = extract_cwd(&prefix) else {
        return false;
    };
    // Normalize separators for a robust comparison.
    let norm_cwd = cwd.replace('\\', "/");
    let norm_root = workspace_root.replace('\\', "/");
    norm_cwd.eq_ignore_ascii_case(&norm_root)
}

/// Pull the first `"cwd":"<json string>"` out of a possibly truncated JSONL
/// prefix, decoding JSON escapes (Windows paths carry `\\`).
fn extract_cwd(prefix: &str) -> Option<String> {
    const KEY: &str = "\"cwd\"";
    let after_key = &prefix[prefix.find(KEY)? + KEY.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
    let body = after_colon.strip_prefix('"')?;
    let mut end = None;
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        match (escaped, c) {
            (true, _) => escaped = false,
            (false, '\\') => escaped = true,
            (false, '"') => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    serde_json::from_str::<String>(&format!("\"{}\"", &body[..end?])).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/agent-jsonl/").to_string() + name,
        )
        .unwrap()
    }

    fn setup_tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aidp-core-tokens-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn it_dedup_last_wins_per_message_id() {
        let tmp = setup_tmp("dedup-last-wins");
        let jsonl = tmp.join("agent-fx01.jsonl");
        std::fs::write(&jsonl, fixture("agent-fx01.jsonl")).unwrap();

        let usage = extract_from_agent_jsonl(&jsonl).unwrap();

        assert_eq!(usage.input_tokens, 3);
        assert_eq!(usage.output_tokens, 728);
        assert_eq!(usage.cache_creation_input_tokens, 60_317);
        assert_eq!(usage.cache_read_input_tokens, 19_236);
        assert_eq!(usage.billable_total, 80_284);
        assert_eq!(usage.total_tokens, 731);

        // Locks the dedup behavior: must NOT equal the naive raw sum over
        // every line (which double/triple-counts input and cache).
        let raw_sum_input = 5u64;
        let raw_sum_output = 740u64;
        let raw_sum_cache_write = 96_061u64;
        let raw_sum_cache_read = 57_708u64;
        assert_ne!(usage.input_tokens, raw_sum_input);
        assert_ne!(usage.output_tokens, raw_sum_output);
        assert_ne!(usage.cache_creation_input_tokens, raw_sum_cache_write);
        assert_ne!(usage.cache_read_input_tokens, raw_sum_cache_read);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn it_scan_run_agents_attributes_stage_and_task() {
        let tmp = setup_tmp("scan-run-agents");

        let wf_dir = tmp
            .join("projects")
            .join("C--ws-demo")
            .join("s1")
            .join("subagents")
            .join("workflows")
            .join("wf_fx");
        std::fs::create_dir_all(&wf_dir).unwrap();

        std::fs::write(wf_dir.join("agent-fx01.jsonl"), fixture("agent-fx01.jsonl")).unwrap();
        std::fs::write(
            wf_dir.join("agent-fx01.meta.json"),
            fixture("agent-fx01.meta.json"),
        )
        .unwrap();
        std::fs::write(wf_dir.join("agent-fx02.jsonl"), fixture("agent-fx02.jsonl")).unwrap();
        std::fs::write(wf_dir.join("agent-fx03.jsonl"), fixture("agent-fx03.jsonl")).unwrap();
        std::fs::write(wf_dir.join("journal.jsonl"), fixture("journal.jsonl")).unwrap();

        let workspace_root_for_cwd = PathBuf::from("C:/ws/demo");
        let results = scan_run_agents(
            &tmp.join("projects"),
            &workspace_root_for_cwd,
            "20260101-000000-0001",
        );

        assert_eq!(
            results.len(),
            2,
            "expected fx01 + fx02 only, fx03 excluded by run filter"
        );
        assert!(!results.iter().any(|a| a.agent_id == "fx03"));

        let fx02 = results
            .iter()
            .find(|a| a.agent_id == "fx02")
            .expect("fx02 present");
        assert_eq!(fx02.stage.as_deref(), Some("implement"));
        assert_eq!(fx02.task_id.as_deref(), Some("t2"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn confirm_cwd_reads_a_bounded_prefix_of_a_huge_first_line() {
        let tmp = setup_tmp("confirm-cwd-huge");
        let jsonl = tmp.join("agent-big.jsonl");
        let huge = "x".repeat(8 * 1024 * 1024);
        let line =
            format!(r#"{{"cwd":"C:\\ws\\demo","sessionId":"s","message":{{"content":"{huge}"}}}}"#);
        std::fs::write(
            &jsonl,
            format!(
                "{line}
"
            ),
        )
        .unwrap();

        assert!(confirm_cwd(&jsonl, "C:/ws/demo"));
        assert!(!confirm_cwd(&jsonl, "C:/ws/other"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn confirm_cwd_rejects_a_transcript_without_cwd_in_the_prefix() {
        let tmp = setup_tmp("confirm-cwd-late");
        let jsonl = tmp.join("agent-late.jsonl");
        let pad = "x".repeat(CWD_PREFIX_BYTES as usize);
        std::fs::write(
            &jsonl,
            format!(r#"{{"message":{{"content":"{pad}"}},"cwd":"C:/ws/demo"}}"#),
        )
        .unwrap();

        assert!(!confirm_cwd(&jsonl, "C:/ws/demo"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn it_scan_run_agents_since_skips_transcripts_older_than_the_run() {
        let tmp = setup_tmp("scan-since");
        let wf_dir = tmp
            .join("projects")
            .join("elsewhere")
            .join("s1")
            .join("subagents");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(wf_dir.join("agent-fx01.jsonl"), fixture("agent-fx01.jsonl")).unwrap();
        let ws = PathBuf::from("C:/ws/demo");

        let all = scan_run_agents_since(&tmp.join("projects"), &ws, "20260101-000000-0001", None);
        assert_eq!(all.len(), 1, "fallback scan still attributes by cwd");

        let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
        let none = scan_run_agents_since(
            &tmp.join("projects"),
            &ws,
            "20260101-000000-0001",
            Some(&future),
        );
        assert!(none.is_empty());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn find_agent_jsonl_finds_the_workflow_layout() {
        let tmp = setup_tmp("find-agent-jsonl");
        let projects_dir = tmp.join("projects");

        // v4 flat layout, non-regression.
        let v4_dir = projects_dir
            .join("proj-v4")
            .join("sess-v4")
            .join("subagents");
        std::fs::create_dir_all(&v4_dir).unwrap();
        std::fs::write(v4_dir.join("agent-v4id.jsonl"), "{}").unwrap();

        // v7 workflow layout.
        let v7_dir = projects_dir
            .join("proj-v7")
            .join("sess-v7")
            .join("subagents")
            .join("workflows")
            .join("wf_run1");
        std::fs::create_dir_all(&v7_dir).unwrap();
        std::fs::write(v7_dir.join("agent-v7id.jsonl"), "{}").unwrap();

        let found_v4 = find_agent_jsonl(&projects_dir, "sess-v4", "v4id").unwrap();
        assert_eq!(found_v4, v4_dir.join("agent-v4id.jsonl"));

        let found_v7 = find_agent_jsonl(&projects_dir, "sess-v7", "v7id").unwrap();
        assert_eq!(found_v7, v7_dir.join("agent-v7id.jsonl"));

        std::fs::remove_dir_all(&tmp).ok();
    }
}
