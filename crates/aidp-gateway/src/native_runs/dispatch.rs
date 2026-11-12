//! One daemon session per stage: prompt, launch, wait, reply validation, one repair, scope check.

use super::agent_job_builder::build_agent_job;
use super::external_session::{external_attempts, ExternalSpec};
use super::reply::{read_reply, repair_instruction, REPLY_FILE};
use super::scope_guard;
use super::state::{RunControl, RunStore};
use super::NativeDeps;
use crate::models::awaited_session::AwaitedSession;
use crate::models::constants::agent_jobs::{
    ENV_RUN, ENV_STAGE, REPAIR_HEADING, RESUME_HEADING, RESUME_INSTRUCTION,
};
use crate::models::constants::native_permissions::REFUSED_OVERRIDE;
use crate::models::constants::session_profile::{AUTONOMOUS_ENABLED, AUTONOMOUS_ENV};
use crate::models::enums::agent_executor::AgentExecutor;
use crate::models::enums::session_outcome::SessionOutcome;
use crate::models::enums::WorkspaceKind;
use crate::models::enums::{Engine, OriginKind, PermissionMode, SessionStatus};
use crate::models::native_run_view::NativeRunView;
use crate::models::session_job::SessionJob;
use crate::models::session_run::SessionRun;
use crate::models::WorkspaceRequest;
use crate::sessions::{Launch, Origin, SessionRecord, StartRequest};
use aidp_agents::{resolve_provider, ProvidersConfig, ResolveRequest};
use aidp_core::cost::{self, ModelTier, PriceTable};
use aidp_core::git::GitCli;
use aidp_core::models::constants::autonomy::AUTONOMY_PREAMBLE_V1;
use aidp_core::models::prompt_form::PromptForm;
use aidp_core::pipeline::{self, Pipeline};
use aidp_core::process_runner::OsProcessRunner;
use aidp_core::prompt::build_prompt_text_with_form;
use aidp_core::run::{ComplexityBucket, StageUsage, UsageSource};
use aidp_core::scope;
use aidp_core::tokens::TokenUsage;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

const PERSONA_EXTENSION: &str = "md";
const TITLE_SEPARATOR: &str = " / ";
const LAUNCH_RETRY_FIRST: Duration = Duration::from_millis(500);
const LAUNCH_RETRY_MAX: Duration = Duration::from_secs(5);
const BUDGET_WAIT: Duration = Duration::from_secs(2);
const LAST_TEXT_TAIL: usize = 300;
const PERSONA_HEADING: &str = "## Role persona\n\n";

/// Launches and supervises the sessions of one run.
pub struct Dispatcher {
    deps: Arc<NativeDeps>,
    store: Arc<RunStore>,
    pipe: Arc<Pipeline>,
    control: Arc<RunControl>,
}

/// True when claude reported another permission mode than the preferred one that was asked.
pub fn permission_refused(asked: PermissionMode, observed: Option<&str>) -> bool {
    asked == PermissionMode::STAGE_PREFERRED && observed.is_some_and(|mode| mode != asked.as_str())
}

fn requested_permission_mode(requested: Option<&str>) -> anyhow::Result<Option<PermissionMode>> {
    requested
        .map(|requested| {
            crate::sessions::validate_permission_mode(requested)?;
            PermissionMode::ALL
                .into_iter()
                .find(|mode| mode.as_str() == requested)
                .ok_or_else(|| anyhow::anyhow!(REFUSED_OVERRIDE))
        })
        .transpose()
}

/// The mode the stages of a run must run in; a headless run always names one, never `default`.
fn requested_stage_mode(
    requested: Option<&str>,
    headless: bool,
) -> anyhow::Result<Option<PermissionMode>> {
    let requested = requested_permission_mode(requested)?;
    Ok(if headless {
        Some(PermissionMode::headless(requested))
    } else {
        requested
    })
}

/// Directories a stage session may use: the run directory, plus the vault root for a headless run.
pub fn stage_add_dirs(run_dir: &Path, vault_root: Option<&Path>, headless: bool) -> Vec<PathBuf> {
    let mut directories = vec![run_dir.to_path_buf()];
    if headless {
        directories.extend(vault_root.map(Path::to_path_buf));
    }
    directories
}

fn explicit_permission_refused(asked: PermissionMode, observed: Option<&str>) -> bool {
    observed.is_some_and(|mode| mode != asked.as_str())
}

/// What to append to a failure reason when claude ran a session under another mode than asked.
pub fn permission_note(asked: PermissionMode, observed: Option<&str>) -> String {
    match observed {
        Some(mode) if mode != asked.as_str() => format!(
            " (claude ran the session in `{mode}` permission mode instead of `{}`: check the permission policy that applies to claude)",
            asked.as_str()
        ),
        _ => String::new(),
    }
}

/// Environment switch that makes a launched session run under the autonomous profile.
pub fn autonomous_env() -> BTreeMap<String, String> {
    BTreeMap::from([(AUTONOMOUS_ENV.to_string(), AUTONOMOUS_ENABLED.to_string())])
}

/// Directory a session works in: the job's own (an isolated task worktree) or the run workspace.
pub fn session_workspace<'a>(job: Option<&'a Path>, run: &'a Path) -> &'a Path {
    job.unwrap_or(run)
}

/// Environment of a stage session: its run and stage, plus the autonomous switch when the run is.
pub fn session_env(run_id: &str, stage: &str, autonomous: bool) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        (ENV_RUN.to_string(), run_id.to_string()),
        (ENV_STAGE.to_string(), stage.to_string()),
    ]);
    if autonomous {
        env.extend(autonomous_env());
    }
    env
}

/// The tail of `text`, for a failure reason.
fn tail(text: &str, count: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars[chars.len().saturating_sub(count)..].iter().collect()
}

/// Token usage and cost of a finished session, `None` when the provider reported no tokens.
pub fn usage_of(record: &SessionRecord) -> Option<StageUsage> {
    let tokens = record.usage.as_ref()?;
    let counted = TokenUsage {
        input_tokens: tokens.input,
        output_tokens: tokens.output,
        cache_creation_input_tokens: tokens.cache_create,
        cache_read_input_tokens: tokens.cache_read,
        ..TokenUsage::default()
    };
    Some(priced_usage(
        &counted,
        record.model.as_deref(),
        record.cost_usd,
    ))
}

/// The usage of `counted` tokens priced by the tier of `model`, else the cost the provider reported.
pub fn priced_usage(
    counted: &TokenUsage,
    model: Option<&str>,
    reported_cost: Option<f64>,
) -> StageUsage {
    let tier = model.map(cost::tier);
    let prices = PriceTable::from_env_or_embedded();
    let priced = tier.filter(|tier| *tier != ModelTier::Unknown);
    let cost_usd = match priced {
        Some(tier) => Some(prices.cost_usd(tier.as_str(), counted, None)),
        None => reported_cost,
    };
    StageUsage {
        input: counted.input_tokens,
        output: counted.output_tokens,
        cache_creation: counted.cache_creation_input_tokens,
        cache_read: counted.cache_read_input_tokens,
        model: model.map(str::to_string),
        bucket: ComplexityBucket::default(),
        cost_usd,
        pricing_version: priced.map(|_| prices.pricing_version().to_string()),
        source: UsageSource::Jsonl,
        ..StageUsage::default()
    }
}

pub fn add_usage(total: Option<StageUsage>, extra: Option<StageUsage>) -> Option<StageUsage> {
    match (total, extra) {
        (Some(mut sum), Some(more)) => {
            sum.input += more.input;
            sum.output += more.output;
            sum.cache_creation += more.cache_creation;
            sum.cache_read += more.cache_read;
            sum.cost_usd = Some(sum.cost_usd.unwrap_or(0.0) + more.cost_usd.unwrap_or(0.0));
            Some(sum)
        }
        (sum, more) => sum.or(more),
    }
}

impl Dispatcher {
    pub fn new(
        deps: Arc<NativeDeps>,
        store: Arc<RunStore>,
        pipe: Arc<Pipeline>,
        control: Arc<RunControl>,
    ) -> Self {
        Dispatcher {
            deps,
            store,
            pipe,
            control,
        }
    }

    /// The file-form prompt of `stage`, preceded by the persona of its role.
    pub fn stage_prompt(&self, stage: &aidp_core::exec_plan::StageSpec) -> anyhow::Result<String> {
        let state = self.store.state();
        let base = build_prompt_text_with_form(&self.pipe, &state, &stage.name, PromptForm::File)?;
        let prompt = match self.persona(stage.agent.as_str()) {
            Some(persona) => format!("{PERSONA_HEADING}{persona}\n\n{base}"),
            None => base,
        };
        Ok(if state.autonomous {
            format!("{AUTONOMY_PREAMBLE_V1}\n\n{prompt}")
        } else {
            prompt
        })
    }

    fn persona(&self, role: &str) -> Option<String> {
        let file = aidp_core::toolset::registry_dir()?.join(format!("{role}.{PERSONA_EXTENSION}"));
        std::fs::read_to_string(file).ok()
    }

    /// Directory of the stage in the vault, from the run state.
    pub fn stage_dir(&self, stage: &str) -> PathBuf {
        self.store
            .state()
            .stages
            .iter()
            .find(|state| state.name == stage)
            .map(|state| state.stage_dir.clone())
            .unwrap_or_else(|| self.store.view().run_dir.join(stage))
    }

    /// Runs the job: launch, wait, validate the reply, relaunch once to repair it, check the scope.
    pub fn run_job(&self, job: &SessionJob) -> anyhow::Result<SessionRun> {
        let started = Instant::now();
        let view = self.store.view();
        let external = view.request.agent_executor == AgentExecutor::External;
        let workspace = view.workspace;
        let exempt = scope_guard::run_dir_exemption(&workspace, &view.run_dir);
        let baseline = job
            .guard_scope
            .then(|| scope_guard::snapshot(&workspace, &GitCli, exempt.as_deref()));
        std::fs::create_dir_all(&job.reply_dir)?;
        let mut run = if external {
            self.run_external(job)?
        } else {
            let _ = std::fs::remove_file(job.reply_dir.join(REPLY_FILE));
            self.run_attempts(job)?
        };
        if let Some(baseline) =
            baseline.filter(|_| !matches!(run.outcome, SessionOutcome::Cancelled))
        {
            if let Some(reason) = self.scope_failure(job, &baseline, &workspace)? {
                run.outcome = SessionOutcome::Failed(reason);
            }
        }
        run.duration_ms = started.elapsed().as_millis() as u64;
        Ok(run)
    }

    /// Hands the session to a host as an agent job and waits for its verdict.
    fn run_external(&self, job: &SessionJob) -> anyhow::Result<SessionRun> {
        let view = self.store.view();
        let provider = self.provider_for(&job.stage, &view)?;
        let skill_roots = aidp_core::skills::default_search_paths();
        let template = build_agent_job(
            job,
            &self.with_resume_note(job),
            &view,
            &self.pipe,
            &self.store.state(),
            &provider,
            &skill_roots,
        );
        let attach = |session_id: &str| self.store.attach_session(&job.label, session_id);
        external_attempts(
            &self.control.board,
            &self.control,
            &attach,
            ExternalSpec {
                run_dir: &view.run_dir,
                label: &job.label,
                template,
            },
        )
    }

    fn run_attempts(&self, job: &SessionJob) -> anyhow::Result<SessionRun> {
        let mut usage: Option<StageUsage> = None;
        let mut prompt = self.with_resume_note(job);
        let finish = |outcome: SessionOutcome, usage: Option<StageUsage>| SessionRun {
            outcome,
            usage,
            duration_ms: 0,
        };
        for repairing in [false, true] {
            let Some(awaited) = self.run_session(job, &prompt, &mut usage)? else {
                return Ok(finish(SessionOutcome::Cancelled, usage));
            };
            let record = &awaited.record;
            usage = add_usage(usage.take(), usage_of(record));
            match record.status {
                SessionStatus::Cancelled => {
                    return Ok(finish(SessionOutcome::Cancelled, usage));
                }
                SessionStatus::Done => {}
                other => {
                    let text = record.last_text.as_deref().unwrap_or_default();
                    let reason = format!(
                        "session {}: {}{}",
                        other.as_str(),
                        tail(text, LAST_TEXT_TAIL),
                        permission_note(awaited.asked_mode, awaited.observed_mode.as_deref())
                    );
                    return Ok(finish(SessionOutcome::Failed(reason), usage));
                }
            }
            match read_reply(&job.reply_dir, job.stage.agent, &job.declared_outputs) {
                Ok(envelope) => return Ok(finish(SessionOutcome::Completed(envelope), usage)),
                Err(error) if repairing => {
                    let reason = format!(
                        "reply rejected after one repair: {error}{}",
                        permission_note(awaited.asked_mode, awaited.observed_mode.as_deref())
                    );
                    return Ok(finish(SessionOutcome::Failed(reason), usage));
                }
                Err(error) => {
                    prompt = format!(
                        "{}{REPAIR_HEADING}{}",
                        self.with_resume_note(job),
                        repair_instruction(&error)
                    );
                }
            }
        }
        Ok(finish(SessionOutcome::Cancelled, usage))
    }

    /// Launches and waits, refusing explicit policy changes and falling back only for legacy requests.
    fn run_session(
        &self,
        job: &SessionJob,
        prompt: &str,
        usage: &mut Option<StageUsage>,
    ) -> anyhow::Result<Option<AwaitedSession>> {
        let request = self.store.view().request;
        let requested = requested_stage_mode(request.permission_mode.as_deref(), request.headless)?;
        loop {
            let mode = self.control.permission_mode(requested);
            let Some(id) = self.launch(job, prompt, mode)? else {
                return Ok(None);
            };
            self.store.attach_session(&job.label, &id)?;
            self.control.track_session(&id);
            let awaited = self.await_session(&id, mode, requested.is_some());
            self.control.untrack_session(&id);
            let Some(awaited) = awaited else {
                return Ok(None);
            };
            if !awaited.refused {
                return Ok(Some(awaited));
            }
            if requested.is_some() {
                anyhow::bail!(
                    "{REFUSED_OVERRIDE}{}",
                    permission_note(mode, awaited.observed_mode.as_deref())
                );
            }
            *usage = add_usage(usage.take(), usage_of(&awaited.record));
            warn!(
                session = id,
                asked = mode.as_str(),
                observed = awaited.observed_mode.as_deref().unwrap_or_default(),
                "claude refused the permission mode of a stage session; falling back"
            );
            self.control.fall_back_permission();
        }
    }

    fn with_resume_note(&self, job: &SessionJob) -> String {
        let resumed = self
            .store
            .row(&job.label)
            .is_some_and(|row| row.resumed && row.attempts > 0);
        if resumed {
            format!("{}{RESUME_HEADING}{RESUME_INSTRUCTION}", job.prompt)
        } else {
            job.prompt.clone()
        }
    }

    fn scope_failure(
        &self,
        job: &SessionJob,
        baseline: &crate::models::scope_baseline::ScopeBaseline,
        workspace: &Path,
    ) -> anyhow::Result<Option<String>> {
        let rules = scope::rules_for(&self.store.state(), &job.stage.name)?;
        let verdict = scope_guard::verify(baseline, &rules, workspace, &GitCli, &OsProcessRunner)?;
        if verdict.is_clean() {
            return Ok(None);
        }
        Ok(Some(format!(
            "wrote outside its scope (restored): {}",
            verdict.violations.join(", ")
        )))
    }

    /// Starts the session, waiting for the budget and for a free workspace. `None` when cancelled.
    fn launch(
        &self,
        job: &SessionJob,
        prompt: &str,
        mode: PermissionMode,
    ) -> anyhow::Result<Option<String>> {
        let view = self.store.view();
        let provider = self.provider_for(&job.stage, &view)?;
        let mut retry = LAUNCH_RETRY_FIRST;
        loop {
            if self.control.is_cancelled() {
                return Ok(None);
            }
            if !self.deps.budget.allows(&view.template_id) {
                self.control.wait_for_wake(BUDGET_WAIT);
                continue;
            }
            let request = self.start_request(job, prompt, &view, &provider, mode);
            match self.deps.sessions.start(request)? {
                Launch::Started(record) => return Ok(Some(record.id)),
                Launch::Queued(queued) => {
                    self.deps.sessions.queue_cancel(&queued.id);
                    self.control.wait_for_wake(retry);
                    retry = (retry * 2).min(LAUNCH_RETRY_MAX);
                }
            }
        }
    }

    fn start_request(
        &self,
        job: &SessionJob,
        prompt: &str,
        view: &NativeRunView,
        provider: &str,
        mode: PermissionMode,
    ) -> StartRequest {
        StartRequest {
            project_id: view.request.project_id.clone(),
            engine: Some(Engine::Prompt.as_str().to_string()),
            template_id: Some(view.template_id.clone()),
            target: view.request.target.clone(),
            prompt: Some(prompt.to_string()),
            permission_mode: Some(mode.as_str().to_string()),
            model: view
                .request
                .model
                .clone()
                .or_else(|| Some(job.stage.model.as_str().to_string())),
            effort: view.request.effort.clone(),
            strict_mcp_config: view.request.strict_mcp_config,
            mcp_config_id: view.request.mcp_config_id.clone(),
            mcp_servers: view.request.mcp_servers.clone(),
            denied_shell_commands: view.request.denied_shell_commands.clone(),
            denied_mcp_tools: view.request.denied_mcp_tools.clone(),
            provider: Some(provider.to_string()),
            origin: Some(Origin {
                kind: OriginKind::Run.as_str().to_string(),
                r#ref: Some(view.run_id.clone()),
                label: Some(job.label.clone()),
            }),
            env: session_env(&view.run_id, &job.stage.name, view.autonomous),
            auto_close: Some(true),
            title: Some(format!("{}{TITLE_SEPARATOR}{}", view.run_id, job.label)),
            workspace: Some(WorkspaceRequest {
                kind: WorkspaceKind::Path,
                ticket: None,
                name: None,
                path: Some(
                    session_workspace(job.workspace.as_deref(), &view.workspace)
                        .to_string_lossy()
                        .to_string(),
                ),
            }),
            shared_writer_group: Some(view.run_id.clone()),
            add_dirs: stage_add_dirs(
                &view.run_dir,
                self.store.state().vault_root.as_deref(),
                view.request.headless,
            ),
            headless: view.request.headless,
            ..StartRequest::default()
        }
    }

    /// The provider of a stage: the stage's own, the template's per role, the run's, the config default.
    fn provider_for(
        &self,
        stage: &aidp_core::exec_plan::StageSpec,
        view: &NativeRunView,
    ) -> anyhow::Result<String> {
        let request = ResolveRequest {
            stage_provider: pipeline::find_stage(&self.pipe, &stage.name)
                .and_then(|found| found.provider.clone()),
            role: stage.agent.as_str().to_string(),
            template_role_providers: self.pipe.settings.providers.clone(),
            launch_override: view
                .request
                .provider_by_role
                .get(stage.agent.as_str())
                .cloned()
                .or_else(|| view.request.provider.clone()),
            tier: Some(stage.model.as_str().to_string()),
        };
        let config = aidp_core::vault::home_dir()
            .map(|home| ProvidersConfig::load(&home).unwrap_or_default())
            .unwrap_or_default();
        Ok(resolve_provider(&request, &config)?.provider)
    }

    /// Waits for the session to leave `running`. A cancelled run stops the session once, and so does a
    /// permission mode claude refused.
    fn await_session(
        &self,
        id: &str,
        asked: PermissionMode,
        explicit: bool,
    ) -> Option<AwaitedSession> {
        let mut stopped = false;
        let mut refused = false;
        let mut observed: Option<String> = None;
        loop {
            let record = self.deps.sessions.get(id)?;
            if observed.is_none() {
                observed.clone_from(&record.effective_permission_mode);
                refused = if explicit {
                    explicit_permission_refused(asked, observed.as_deref())
                } else {
                    permission_refused(asked, observed.as_deref())
                };
            }
            if !matches!(
                record.status,
                SessionStatus::Running | SessionStatus::Queued
            ) {
                return Some(AwaitedSession {
                    record,
                    asked_mode: asked,
                    observed_mode: observed,
                    refused,
                });
            }
            if (self.control.is_cancelled() || refused) && !stopped {
                stopped = true;
                if let Err(error) = self.deps.sessions.cancel(id) {
                    warn!(session = id, %error, "cancel of a stage session failed");
                }
            }
            std::thread::sleep(self.deps.poll_interval);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/dispatch_tests.rs"]
mod tests;
