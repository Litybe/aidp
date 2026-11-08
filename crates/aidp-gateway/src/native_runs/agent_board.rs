//! The jobs of one run's agent sessions: publication, claims, leases, verdicts, persisted in `agent-jobs/`.

use super::agent_job_builder::prompt_of;
use super::agent_verdict::judge;
use super::{conflict, not_found};
use crate::models::agent_claim::AgentClaim;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_heartbeat::AgentHeartbeat;
use crate::models::agent_job::AgentJob;
use crate::models::agent_job_ack::AgentJobAck;
use crate::models::agent_job_listing::AgentJobListing;
use crate::models::agent_job_record::AgentJobRecord;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::agent_job_summary::AgentJobSummary;
use crate::models::constants::agent_jobs::{
    AGENT_JOBS_DIR, CLAIM_TIMEOUT_MS, ERROR_ALREADY_CLAIMED, ERROR_DIFFERENT_RESULT,
    ERROR_LEASE_LOST, ERROR_MALFORMED_ID, ERROR_NOT_PENDING, ERROR_UNKNOWN_JOB, ERROR_WRONG_TOKEN,
    ID_PREFIX, ID_SEPARATOR, JOB_FILE_EXTENSION, LEASE_SECONDS, MILLIS_PER_MINUTE,
    MILLIS_PER_SECOND, NEXT_IGNORED, NEXT_REPAIR, NEXT_SETTLED, REASON_HOST_TIMEOUT,
    REASON_INTERRUPTED_TWICE, REASON_NO_HOST, REASON_RUN_CANCELLED, REASON_UNKNOWN_OUTCOME,
    REPAIR_HEADING, RESUME_HEADING, RESUME_INSTRUCTION, RETRY_AFTER_NOTE, RETRY_AFTER_UNIT,
    SESSION_GRACE_MS,
};
use crate::models::enums::agent_job_kind::AgentJobKind;
use crate::models::enums::agent_job_state::AgentJobState;
use crate::models::enums::agent_outcome::AgentOutcome;
use crate::models::enums::agent_verdict::AgentVerdict;
use crate::session_error::SessionError;
use crate::store::write_json;
use crate::sync::lock;
use aidp_core::models::enums::agent_role::AgentRole;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::warn;

const REPAIR_ATTEMPT_STEP: u32 = 1;

/// What publishing a job for a label answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub job_id: String,
    /// True when a new initial job was published: the reply file left by an earlier job is stale.
    pub fresh: bool,
}

#[derive(Default)]
struct BoardState {
    run_dir: Option<PathBuf>,
    records: BTreeMap<u32, AgentJobRecord>,
}

/// The agent jobs of a run, shared by its executor threads and the HTTP handlers.
#[derive(Default)]
pub struct AgentBoard {
    inner: Mutex<BoardState>,
}

/// The id of job number `seq` of `run_id`.
pub fn job_id_of(run_id: &str, seq: u32) -> String {
    format!("{ID_PREFIX}{ID_SEPARATOR}{run_id}{ID_SEPARATOR}{seq}")
}

/// The run id and sequence number a job id names.
pub fn parse_job_id(job_id: &str) -> Option<(&str, u32)> {
    let rest = job_id.strip_prefix(ID_PREFIX)?.strip_prefix(ID_SEPARATOR)?;
    let (run_id, seq) = rest.rsplit_once(ID_SEPARATOR)?;
    Some((run_id, seq.parse().ok()?))
}

fn iso_of(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|moment| moment.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn digest_of(result: &AgentJobResult) -> String {
    let session = result.agent.as_ref();
    let body = serde_json::to_string(&(
        &result.outcome,
        &result.reason,
        session.and_then(|report| report.session_id.as_ref()),
        session.and_then(|report| report.duration_ms),
        session.and_then(|report| report.usage.as_ref()),
    ))
    .unwrap_or_default();
    Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn with_note(mut job: AgentJob, heading: &str, note: &str) -> AgentJob {
    job.prompt = prompt_of(&format!("{}{heading}{note}", job.prompt.text));
    job
}

fn ack(job_id: &str, accepted: bool, duplicate: bool, next: &str) -> AgentJobAck {
    AgentJobAck {
        job_id: job_id.to_string(),
        accepted,
        duplicate,
        next: next.to_string(),
        repair_job_id: None,
        reason: None,
    }
}

fn failure_reason(verdict: &AgentVerdict) -> Option<String> {
    match verdict {
        AgentVerdict::Failed(reason) => Some(reason.clone()),
        AgentVerdict::Repair { error, .. } => Some(error.clone()),
        _ => None,
    }
}

fn persist(state: &BoardState, seq: u32) -> anyhow::Result<()> {
    let (Some(run_dir), Some(record)) = (state.run_dir.as_ref(), state.records.get(&seq)) else {
        return Ok(());
    };
    let file = run_dir
        .join(AGENT_JOBS_DIR)
        .join(format!("{seq}.{JOB_FILE_EXTENSION}"));
    write_json(&file, record)
}

fn load(directory: &Path) -> BTreeMap<u32, AgentJobRecord> {
    let mut records = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return records;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_job = path
            .extension()
            .is_some_and(|extension| extension == JOB_FILE_EXTENSION);
        let seq = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u32>().ok());
        let (true, Some(seq)) = (is_job, seq) else {
            continue;
        };
        let parsed = aidp_core::text::read_text_lenient(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(serde_json::from_str::<AgentJobRecord>(&text)?));
        match parsed {
            Ok(record) => {
                records.insert(seq, record);
            }
            Err(error) => warn!(file = %path.display(), %error, "agent job record skipped"),
        }
    }
    records
}

fn new_record(job: AgentJob, now: i64) -> AgentJobRecord {
    AgentJobRecord {
        job,
        state: AgentJobState::Pending,
        claim_token: None,
        lease_expires_ms: None,
        claims: 0,
        resumed: false,
        created_ms: now,
        pending_since_ms: now,
        claimed_ms: None,
        result_digest: None,
        report: None,
        verdict: None,
        repair_job_id: None,
        consumed: false,
    }
}

fn next_seq(state: &BoardState) -> u32 {
    state.records.keys().next_back().map_or(1, |last| last + 1)
}

fn fail_record(record: &mut AgentJobRecord, reason: &str) {
    record.state = AgentJobState::Done;
    record.verdict = Some(AgentVerdict::Failed(reason.to_string()));
    record.lease_expires_ms = None;
}

fn sweep_record(record: &mut AgentJobRecord, now: i64) -> bool {
    match record.state {
        AgentJobState::Claimed => {
            let lease_over = record.lease_expires_ms.is_some_and(|end| end <= now);
            let limit =
                i64::from(record.job.timeout_minutes) * MILLIS_PER_MINUTE + SESSION_GRACE_MS;
            let timed_out = record.claimed_ms.is_some_and(|start| now - start > limit);
            if lease_over && record.claims >= 2 {
                fail_record(record, REASON_INTERRUPTED_TWICE);
            } else if lease_over {
                record.state = AgentJobState::Pending;
                record.resumed = true;
                record.claim_token = None;
                record.lease_expires_ms = None;
                record.claimed_ms = None;
                record.pending_since_ms = now;
            } else if timed_out {
                fail_record(record, REASON_HOST_TIMEOUT);
            } else {
                return false;
            }
            true
        }
        AgentJobState::Pending if now - record.pending_since_ms > CLAIM_TIMEOUT_MS => {
            fail_record(record, REASON_NO_HOST);
            true
        }
        _ => false,
    }
}

fn sweep_all(state: &mut BoardState, now: i64) -> anyhow::Result<bool> {
    let changed: Vec<u32> = state
        .records
        .iter_mut()
        .filter_map(|(seq, record)| sweep_record(record, now).then_some(*seq))
        .collect();
    for seq in &changed {
        persist(state, *seq)?;
    }
    Ok(!changed.is_empty())
}

fn unknown_job(job_id: &str) -> anyhow::Error {
    not_found(format!("{ERROR_UNKNOWN_JOB} `{job_id}`"))
}

fn record_mut<'a>(
    state: &'a mut BoardState,
    job_id: &str,
) -> anyhow::Result<(u32, &'a mut AgentJobRecord)> {
    let (_, seq) = parse_job_id(job_id).ok_or_else(|| conflict(ERROR_MALFORMED_ID.to_string()))?;
    state
        .records
        .get_mut(&seq)
        .map(|record| (seq, record))
        .ok_or_else(|| unknown_job(job_id))
}

fn lease_end(now: i64) -> i64 {
    now + LEASE_SECONDS as i64 * MILLIS_PER_SECOND
}

fn publish_repair(
    state: &mut BoardState,
    parent_seq: u32,
    parent: &AgentJob,
    instruction: &str,
    now: i64,
) -> anyhow::Result<String> {
    let existing = state.records.iter().find(|(seq, record)| {
        **seq > parent_seq
            && record.job.label == parent.label
            && record.job.job_kind == AgentJobKind::Repair
            && record.state.is_open()
    });
    if let Some((_, record)) = existing {
        return Ok(record.job.id.clone());
    }
    let seq = next_seq(state);
    let mut job = with_note(parent.clone(), REPAIR_HEADING, instruction);
    job.id = job_id_of(&parent.run_id, seq);
    job.job_kind = AgentJobKind::Repair;
    job.attempt = parent.attempt + REPAIR_ATTEMPT_STEP;
    let job_id = job.id.clone();
    state.records.insert(seq, new_record(job, now));
    persist(state, seq)?;
    Ok(job_id)
}

fn summary_of(record: &AgentJobRecord) -> AgentJobSummary {
    let job = &record.job;
    AgentJobSummary {
        id: job.id.clone(),
        name: job.label.clone(),
        attempt: job.attempt,
        writes: !job.tools.read_only,
        workspace: job.workspace.root.clone(),
        created_at: iso_of(record.created_ms),
        agent: AgentJobListing {
            run_id: job.run_id.clone(),
            stage: job.stage.clone(),
            label: job.label.clone(),
            job_kind: job.job_kind,
            resumed: record.resumed,
            provider: job.agent.provider.clone(),
            tier: job.agent.tier.clone(),
            model: job.agent.model.clone(),
            effort: job.agent.effort.clone(),
            cwd: job.workspace.cwd.clone(),
        },
    }
}

impl AgentBoard {
    /// Loads the jobs persisted under `run_dir`, once per directory.
    pub fn ensure_open(&self, run_dir: &Path) -> anyhow::Result<()> {
        let mut state = lock(&self.inner);
        if state.run_dir.as_deref() == Some(run_dir) {
            return Ok(());
        }
        state.records = load(&run_dir.join(AGENT_JOBS_DIR));
        state.run_dir = Some(run_dir.to_path_buf());
        Ok(())
    }

    /// Attaches to the open job of `label`, replays its unconsumed verdict, or publishes a new job.
    pub fn publish_or_attach(
        &self,
        label: &str,
        kind: AgentJobKind,
        template: AgentJob,
        now: i64,
    ) -> anyhow::Result<Published> {
        let mut state = lock(&self.inner);
        let last = state
            .records
            .values()
            .rev()
            .find(|record| record.job.label == label);
        if let Some(record) = last {
            let reusable = match record.state {
                AgentJobState::Pending | AgentJobState::Claimed => true,
                AgentJobState::Done => !record.consumed,
                AgentJobState::Cancelled => false,
            };
            if reusable {
                return Ok(Published {
                    job_id: record.job.id.clone(),
                    fresh: false,
                });
            }
        }
        let seq = next_seq(&state);
        let mut job = template;
        job.id = job_id_of(&job.run_id, seq);
        job.job_kind = kind;
        job.attempt = match kind {
            AgentJobKind::Initial => job.attempt.max(1),
            AgentJobKind::Repair => job.attempt.max(1) + REPAIR_ATTEMPT_STEP,
        };
        let job_id = job.id.clone();
        state.records.insert(seq, new_record(job, now));
        persist(&state, seq)?;
        Ok(Published {
            job_id,
            fresh: true,
        })
    }

    /// The pending jobs, oldest first.
    pub fn summaries(&self, now: i64) -> anyhow::Result<Vec<AgentJobSummary>> {
        let mut state = lock(&self.inner);
        sweep_all(&mut state, now)?;
        Ok(state
            .records
            .values()
            .filter(|record| record.state == AgentJobState::Pending)
            .map(summary_of)
            .collect())
    }

    /// Gives the job to one host for a lease. A re-claim after an expired lease carries the resume note.
    pub fn claim(
        &self,
        job_id: &str,
        request: &AgentClaimRequest,
        now: i64,
    ) -> anyhow::Result<AgentClaim> {
        let mut state = lock(&self.inner);
        sweep_all(&mut state, now)?;
        let (seq, record) = record_mut(&mut state, job_id)?;
        match record.state {
            AgentJobState::Pending => {}
            AgentJobState::Claimed => return Err(conflict(ERROR_ALREADY_CLAIMED.to_string())),
            AgentJobState::Done | AgentJobState::Cancelled => {
                return Err(conflict(ERROR_NOT_PENDING.to_string()))
            }
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        let lease_ends = lease_end(now);
        record.state = AgentJobState::Claimed;
        record.claims += 1;
        record.claim_token = Some(token.clone());
        record.lease_expires_ms = Some(lease_ends);
        record.claimed_ms = Some(now);
        let resumed = record.resumed;
        let job = if resumed && !record.job.prompt.text.ends_with(RESUME_INSTRUCTION) {
            with_note(record.job.clone(), RESUME_HEADING, RESUME_INSTRUCTION)
        } else {
            record.job.clone()
        };
        tracing::info!(job = job_id, actor = ?request.actor, host = ?request.host_id, "agent job claimed");
        persist(&state, seq)?;
        Ok(AgentClaim {
            claim_token: token,
            lease_seconds: LEASE_SECONDS,
            lease_expires_at: iso_of(lease_ends),
            resumed,
            job,
        })
    }

    /// Renews the lease of the claimed job. A cancelled job answers `cancel_requested`.
    pub fn heartbeat(
        &self,
        job_id: &str,
        claim_token: &str,
        now: i64,
    ) -> anyhow::Result<AgentHeartbeat> {
        let mut state = lock(&self.inner);
        sweep_all(&mut state, now)?;
        let (seq, record) = record_mut(&mut state, job_id)?;
        let owned = record.claim_token.as_deref() == Some(claim_token);
        match (record.state, owned) {
            (AgentJobState::Claimed, true) => {
                let lease_ends = lease_end(now);
                record.lease_expires_ms = Some(lease_ends);
                persist(&state, seq)?;
                Ok(AgentHeartbeat {
                    lease_expires_at: iso_of(lease_ends),
                    cancel_requested: false,
                })
            }
            (AgentJobState::Cancelled, true) => Ok(AgentHeartbeat {
                lease_expires_at: iso_of(now),
                cancel_requested: true,
            }),
            _ => Err(conflict(ERROR_LEASE_LOST.to_string())),
        }
    }

    /// Judges the posted result. A repair verdict publishes the repair job in the same call.
    pub fn deliver(
        &self,
        job_id: &str,
        result: &AgentJobResult,
        now: i64,
    ) -> anyhow::Result<AgentJobAck> {
        let mut state = lock(&self.inner);
        let digest = digest_of(result);
        let token = result
            .agent
            .as_ref()
            .map(|report| report.claim_token.as_str())
            .unwrap_or_default();
        sweep_all(&mut state, now)?;
        let (seq, record) = record_mut(&mut state, job_id)?;
        if record.claim_token.as_deref() != Some(token) {
            return Err(conflict(ERROR_WRONG_TOKEN.to_string()));
        }
        if record.state == AgentJobState::Cancelled {
            let mut ignored = ack(job_id, false, false, NEXT_IGNORED);
            ignored.reason = Some(REASON_RUN_CANCELLED.to_string());
            return Ok(ignored);
        }
        if record.state == AgentJobState::Done {
            if record.result_digest.as_deref() != Some(digest.as_str()) {
                return Err(conflict(ERROR_DIFFERENT_RESULT.to_string()));
            }
            let next = if record.repair_job_id.is_some() {
                NEXT_REPAIR
            } else {
                NEXT_SETTLED
            };
            let mut duplicate = ack(job_id, true, true, next);
            duplicate.repair_job_id.clone_from(&record.repair_job_id);
            return Ok(duplicate);
        }
        let outcome = AgentOutcome::parse(&result.outcome).ok_or_else(|| {
            SessionError::Config(format!("{REASON_UNKNOWN_OUTCOME} `{}`", result.outcome))
        })?;
        let retry_after = result
            .agent
            .as_ref()
            .and_then(|report| report.retry_after_seconds);
        let reason = match (outcome, retry_after) {
            (AgentOutcome::RateLimited, Some(seconds)) => Some(format!(
                "{} ({RETRY_AFTER_NOTE} {seconds}{RETRY_AFTER_UNIT})",
                result.reason.as_deref().unwrap_or_default()
            )),
            _ => result.reason.clone(),
        };
        let verdict = judge(
            outcome,
            reason.as_deref(),
            &record.job.io.stage_dir,
            AgentRole::parse(&record.job.agent.role),
            &record.job.io.declared_outputs,
            record.job.attempt,
        );
        record.state = AgentJobState::Done;
        record.lease_expires_ms = None;
        record.result_digest = Some(digest);
        record.report.clone_from(&result.agent);
        record.verdict = Some(verdict.clone());
        let parent = record.job.clone();
        let repair_job_id = match &verdict {
            AgentVerdict::Repair { instruction, .. } => {
                Some(publish_repair(&mut state, seq, &parent, instruction, now)?)
            }
            _ => None,
        };
        if let Some(record) = state.records.get_mut(&seq) {
            record.repair_job_id.clone_from(&repair_job_id);
        }
        persist(&state, seq)?;
        let next = if repair_job_id.is_some() {
            NEXT_REPAIR
        } else {
            NEXT_SETTLED
        };
        let mut accepted = ack(job_id, true, false, next);
        accepted.repair_job_id = repair_job_id;
        accepted.reason = failure_reason(&verdict);
        Ok(accepted)
    }

    /// Applies lease expiry, the claim timeout and the session timeout. True when a job changed.
    pub fn sweep(&self, now: i64) -> anyhow::Result<bool> {
        sweep_all(&mut lock(&self.inner), now)
    }

    /// Closes every open job as cancelled and returns how many it closed.
    pub fn cancel_all(&self) -> anyhow::Result<usize> {
        let mut state = lock(&self.inner);
        let open: Vec<u32> = state
            .records
            .iter_mut()
            .filter(|(_, record)| record.state.is_open())
            .map(|(seq, record)| {
                record.state = AgentJobState::Cancelled;
                record.lease_expires_ms = None;
                *seq
            })
            .collect();
        for seq in &open {
            persist(&state, *seq)?;
        }
        Ok(open.len())
    }

    /// A copy of the record of `job_id`.
    pub fn record(&self, job_id: &str) -> Option<AgentJobRecord> {
        let (_, seq) = parse_job_id(job_id)?;
        lock(&self.inner).records.get(&seq).cloned()
    }

    /// Marks the verdict of `job_id` as read by the run thread, so a later attempt publishes a new job.
    pub fn consume(&self, job_id: &str) -> anyhow::Result<()> {
        let mut state = lock(&self.inner);
        let (seq, record) = record_mut(&mut state, job_id)?;
        record.consumed = true;
        persist(&state, seq)
    }

    /// True while a host holds a lease on any job of the run.
    pub fn has_claimed(&self) -> bool {
        lock(&self.inner)
            .records
            .values()
            .any(|record| record.state == AgentJobState::Claimed)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/agent_board_tests.rs"]
mod tests;
