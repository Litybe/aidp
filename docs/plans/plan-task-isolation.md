---
title: Plan task isolation
---

# Plan task isolation

## Problem

Parallel tasks of a plan stage share the ticket worktree. `scope_guard::capture_task_baseline` and `changed_since` attribute to a task every path that changed in the whole worktree during its run window. Each concurrent task therefore sees the writes of its siblings. `exec_loop::flag_overlaps` then fails both tasks with `OverlappingWrites`, and the out-of-scope check fails a third task with `WroteOutsideFiles`. Observed on PIXEL-5840, run 20261016-145116-1980, stage `implement`.

## Decision

Each concurrent writer task runs in its own detached git worktree. Its writes are attributed exactly, because nothing else writes there. On success, its changes are applied back to the ticket worktree and the isolated worktree is removed.

## When a task is isolated

| Case | Isolated | Why |
| ---- | -------- | --- |
| `alone` (one task in the schedule) | No | Same path as today: the stage owns the workspace. |
| `!alone` and `schedule.effective_parallel() == 1` | No | Tasks run one after the other; the existing baseline attribution is exact. Sequential plans stay unchanged. |
| `!alone` and `effective_parallel() > 1` | Yes | Tasks can run at the same time. |

Isolation is decided once per stage in `exec_plan_tasks` and passed to `run_task`. No pipeline field is added: `StageSpec.isolation` (`Isolation::Worktree`) is for Claude Workflow mode and stays untouched. The native decision is purely `effective_parallel() > 1`.

## Where the isolated worktree lives

| Item | Value |
| ---- | ----- |
| Path | `<run_dir>/iso/<task-id>`. Short names keep Windows paths under the limit. |
| Why under the run dir | The session workspace request is `WorkspaceKind::Path`; `workspace_resolver::resolve_path` only accepts paths under the allowed roots (project paths, vault, `AIDP_ALLOWED_ROOTS`). The run dir lives in the vault or inside the workspace, so both are allowed. |
| Exemption | When the run dir is inside the ticket worktree, `run_dir_exemption` already hides it from the ticket's status. The isolated tree is a sibling of run state, so no new exemption is needed. |
| Branch | None. The worktree is detached, so no branch can leak or collide and nothing is left to delete. |

## Creating the worktree (`task_isolation::prepare`)

New module `crates/aidp-gateway/src/native_runs/task_isolation.rs`. It takes a `&dyn ProcessRunner` so unit tests use `ScriptedProcessRunner`. All git commands run with `cwd = ticket worktree`, so it works from a linked worktree and from a plain checkout prepared by the host: `git worktree add` works from any git working tree, and the only requirement is that the checkout has a `.git`.

| Step | Command (cwd) | Notes |
| ---- | ------------- | ----- |
| 1. Remove a leftover tree | `git worktree remove --force <iso>` then `git worktree prune` (ticket) | Idempotent. Covers restart and retry. Also delete the directory if git does not know it. |
| 2. Create | `git worktree add --detach <iso> HEAD` (ticket) | HEAD of the ticket worktree at task start. |
| 3. Carry the tracked dirty state | `git diff HEAD --binary --full-index` (ticket) piped to `git apply` (iso) | Skip when the diff is empty. Includes work from earlier tasks already applied to the ticket worktree and from earlier stages. |
| 4. Carry the untracked files | `git ls-files -o --exclude-standard -z` (ticket), then copy each file (not under the run-dir exemption) with `std::fs::copy`, creating parents | Files ignored by git are not copied. |
| 5. Pin the base | In iso: `git add -A`, then `git -c user.name=aidp -c user.email=aidp@localhost -c commit.gpgsign=false commit --no-verify --allow-empty -q -m "aidp task base"` | The base commit makes the isolated tree clean. Its sha is the task's baseline `head`. Skip the commit and keep HEAD when step 3 and 4 copied nothing. |

Step 5 is what makes attribution trivial: `capture_task_baseline(iso, ..)` sees no dirty path and records the base sha as `head`.

Failure of any step returns `NativeReason::IsolationFailed` ("could not prepare an isolated worktree for the task") with the git stderr on the log. The task fails, the worktree is removed.

The ticket-worktree reads in steps 2 to 4 run under a stage-level `Mutex` (`TaskIsolation::state`). Without it a concurrent apply-back (below) could change the ticket tree between `worktree add` and the diff.

## Session workspace

| Change | Location |
| ------ | -------- |
| New field `workspace: Option<PathBuf>` on `SessionJob` (`None` keeps today's behavior) | `crates/aidp-gateway/src/models/session_job.rs` |
| `start_request` uses `job.workspace` for `WorkspaceRequest.path` when set, else `view.workspace` | `dispatch.rs`, `start_request` |
| `run_job` keeps `guard_scope` false for tasks, so its snapshot is not taken on the ticket tree | unchanged |

`addDirs` is unchanged (`stage_add_dirs`: run dir and vault). The reply directory stays the real `<stage_dir>/tasks/<id>`; agents write it by absolute path. `shared_writer_group` stays the run id. The workspace key now differs per task, so the session queue admits the tasks in parallel on its own.

## Attributing writes

`run_task` uses `task_workspace = iso path` (isolated) or the ticket worktree (not isolated) for `capture_task_baseline`, `changed_since`, `verify_task_writes` and `restore_task_violations`. The functions in `scope_guard.rs` are reused as they are.

| Concern | Isolated behavior |
| ------- | ----------------- |
| `written` | Exactly the paths the task changed against the base commit. |
| Peer files allowance | `verify_task_scope` passes an empty `allowed_extra`. A sibling's files cannot appear in the isolated tree, so a write to them is a real out-of-scope write. |
| Out-of-scope check | Unchanged: owned files = task `files`; `shared_files` are merge-only. Violations are restored inside the isolated tree (`restore_task_violations` against the base sha), then the task fails with `WroteOutsideFiles`. The ticket worktree never sees them. |
| `flag_overlaps` | Still called; it finds nothing because isolated tasks have disjoint `written` sets. It stays for the non-isolated path. No change. |

## Applying changes back (`task_isolation::apply_back`)

Called by the worker right after a task succeeded and its scope check was clean, under the same stage-level mutex as `prepare`.

| Step | Command (cwd) | Notes |
| ---- | ------------- | ----- |
| 1. Stage everything | `git add -A` (iso) | The isolated index is private. Picks up new, modified and deleted files, including untracked ones. |
| 2. Build the patch | `git diff --cached --binary --full-index <base-sha>` (iso), output kept in memory | Empty patch: nothing to apply, success. Restricted to `outcome.written` by construction (violations were restored). |
| 3. Check | `git apply --check -` (ticket) with the patch on stdin | Rejects when context or file state differs from what the task started from. |
| 4. Apply | `git apply -` (ticket) | Working tree only, no `--index`: the changes stay unstaged, exactly as when the agent wrote there directly. |

Order: patches are applied in completion order, which is plan order for any two tasks linked by `depends_on` (a dependent only starts after its dependency applied). Independent tasks have disjoint declared files (the scheduler serializes tasks that share files), so their patches commute. The `--check` catches any remaining case.

`ProcessSpec` carries no stdin payload today. The implementer writes the patch to `<run_dir>/iso/<task-id>.patch` and passes the file path to `git apply --check <file>` and `git apply <file>`. The patch file is deleted with the worktree.

## Conflict handling

| Situation | Result |
| --------- | ------ |
| `git apply --check` fails | The task becomes `Failed`, `run_status = Paused`, `reason = NativeReason::ApplyConflict` ("its changes conflict with the work of an earlier task"), `violations` = the paths named by git on stderr (parsed from `error: patch failed: <path>:<line>` and `error: <path>: already exists in working directory` lines; fall back to `outcome.written`). The ticket worktree is untouched because the check precedes the apply. |
| `git apply` fails after a passing check | Treated like a conflict. Restore the ticket worktree for the task's `written` paths by `git checkout <ticket-head-at-apply> -- <path>` or delete when untracked (reuse `restore_file`), then fail with `ApplyConflict`. |
| Earlier tasks | Stay applied and `Success`. Only the later task fails, which replaces the false "overlap" of both. |

A failed task is retried through the existing `retry stage rows` path. The retry creates a fresh isolated tree from the current ticket state, which now includes the work of the tasks that succeeded.

## Cleanup

`task_isolation::release(ticket, iso)` runs `git worktree remove --force <iso>`, `git worktree prune`, and removes the patch file. It runs in all of these cases.

| Case | Where |
| ---- | ----- |
| Task succeeded (after apply-back) | End of `run_task` |
| Task failed (scope, conflict, session failure, error) | End of `run_task`, in a guard that also runs on `?` returns (use a small `Drop` guard struct `IsolatedTask` holding the runner and paths) |
| Cancel | `run_job` returns `SessionOutcome::Cancelled`; the guard removes the tree. Nothing is applied for a cancelled task. |
| Task thread panic | The `Drop` guard runs during unwinding; `drive_tasks` already catches the panic as `TaskPanicked`. |
| Daemon crash or restart | See below. |

Release failures are logged at `warn` and never change the outcome of the task.

## Restart and resume (`prepare_resume`)

`prepare_resume` puts `Running` rows back to `Pending` with `resumed = true`; `exec_plan_tasks` then re-runs every row that is not `Done`. Isolation adds one rule: a task is never resumed in its old isolated tree.

| Case | Behavior |
| ---- | -------- |
| Task interrupted before apply-back | Its isolated tree may still exist with partial work. `prepare` step 1 removes it and rebuilds from the ticket tree, which has none of that partial work. The resume note ("keep what is already done") then finds the ticket tree clean for this task, so the agent redoes it. Acceptable: partial work was never applied. |
| Task `Done` | Already applied to the ticket tree before the row became `Done`. Order in `run_task`: apply-back first, then `settle_sub_row`'s Done mark. Concretely, `settle_sub_row` runs after the session, so apply-back must happen before the row is committed `Done`. See the ordering rule below. |
| Orphan trees of a dead run | `exec_plan_tasks` starts by calling `task_isolation::sweep(ticket, <run_dir>/iso)`: for each entry under `iso/` not owned by a running task of this process, `git worktree remove --force`, then `git worktree prune`. It runs once before `drive_tasks`. |

Ordering rule to keep rows consistent: `run_task` must not mark the sub-row `Done` before the apply-back succeeded. Today `settle_sub_row` marks `Done` right after the session. The implementer moves the success decision: call `settle_sub_row` as now, but when apply-back fails afterwards call `self.store.fail(&label, ..)` the way `run_task` already does for `WroteOutsideFiles` (it overwrites the row to `Failed`; `settle_outcomes` also re-fails rows that are not `Failed`). A crash between the `Done` mark and the apply-back would lose the task's changes while the row says `Done`. To close it, apply back before `settle_sub_row`: restructure to `run_job` -> if the session completed, scope check and apply-back on the raw `SessionRun` -> then settle the row with the final result. Do this; it keeps the crash window empty.

## Failure reasons

Two new variants in `crates/aidp-gateway/src/models/enums/native_reason.rs` (enum file, text in `as_str`):

| Variant | Text |
| ------- | ---- |
| `IsolationFailed` | `could not prepare an isolated worktree for the task` |
| `ApplyConflict` | `its changes conflict with the work of an earlier task` |

Update `enums_tests.rs` if it lists every reason.

## Constants

New file `crates/aidp-gateway/src/models/constants/task_isolation.rs`, registered in `constants/mod.rs`, with the git arguments not already in `task_scope.rs`: `worktree`, `add`, `--detach`, `remove`, `prune`, `diff`, `--cached`, `--binary`, `--full-index`, `apply`, `--check`, `ls-files`, `-o`, `--exclude-standard`, `commit`, `--no-verify`, `--allow-empty`, `-q`, `-m`, `-c`, the base commit message and identity config values, the `iso` directory name and the `.patch` extension. Reuse `STATUS_ARG`, `HEAD_REF`, `NUL_SEPARATOR` from `task_scope.rs`.

New model `TaskWorktree { path: PathBuf, base: String }` in `crates/aidp-gateway/src/models/task_worktree.rs` (one type per file).

## Code changes

| File | Change |
| ---- | ------ |
| `native_runs/task_isolation.rs` (new) | `prepare`, `apply_back`, `release`, `sweep`, parse of conflict paths, `IsolatedTask` drop guard. All take `&dyn ProcessRunner`. |
| `native_runs/exec_loop.rs` | `exec_plan_tasks`: compute `isolate = !alone && schedule.effective_parallel() > 1`, create a `Mutex<()>` for ticket-tree access, call `sweep`. Pass both to `run_task`. `run_task`: when isolated, `prepare` before the baseline, set `job.workspace`, run the scope check against the isolated path, `apply_back`, release. `verify_task_scope` takes the workspace to inspect and an `isolated: bool` to drop the peer allowance. |
| `native_runs/dispatch.rs` | `start_request` honors `job.workspace`. |
| `models/session_job.rs` | New `workspace: Option<PathBuf>`; every `SessionJob { .. }` literal gets `workspace: None` (two in `exec_loop.rs`, test fixtures). |
| `models/enums/native_reason.rs` | Two variants. |
| `models/constants/task_isolation.rs`, `models/task_worktree.rs` | New. |
| `docs/native-execution.md` | Rewrite "Plan task scope and failures": concurrent tasks run in isolated detached worktrees under `<run_dir>/iso/`, applied back when they succeed, `ApplyConflict` and `IsolationFailed` reasons, the old line "Two concurrent tasks writing the same file both fail" replaced; the "share the run worktree" line narrowed to non-isolated tasks. |
| `docs/adr/native-plan-task-parallelism.md` | Add a short "Isolation" section linking here; delete statements the fix makes false (concurrent tasks attributed by window). |
| `docs/gateway-api.md` | Only if it lists the failure reasons; add the two. |

The CLAUDE.md/AGENTS.md rules apply: constants in `models/constants`, enums in `models/enums`, tests under `tests/unit` with fakes, no `unsafe`.

## Unit tests (default nextest profile, no network)

All in `crates/aidp-gateway/tests/unit/task_isolation_tests.rs` (new, wired with `#[path]` from `task_isolation.rs`) using `ScriptedProcessRunner` to assert the exact git commands and their order, plus `exec_loop_tests.rs` additions. Real-git tests must be named `it_*` and are not run by this task.

| Test | Asserts |
| ---- | ------- |
| `prepare_creates_a_detached_worktree_under_the_run_dir` | Commands in order: remove, prune, `worktree add --detach <run_dir>/iso/T1 HEAD`; returned path and no branch. |
| `prepare_skips_the_base_commit_for_a_clean_ticket_tree` | Empty diff and no untracked files: no `apply`, no `commit`; `base` is the ticket HEAD. |
| `prepare_carries_the_dirty_state_into_a_base_commit` | Non-empty diff: `apply` runs in iso; untracked list copied; `commit --allow-empty` runs; `base` is the sha printed by `rev-parse HEAD` in iso. |
| `prepare_runs_every_git_command_in_the_ticket_worktree_or_iso` | `cwd` of each spec; none in the aidp project path (host-prepared checkout case). |
| `prepare_failure_maps_to_isolation_failed_and_cleans_up` | A failing `worktree add` yields the error and still calls `release`. |
| `apply_back_with_an_empty_patch_does_not_call_apply` | Success, no `git apply`. |
| `apply_back_checks_before_applying` | `apply --check <patch>` precedes `apply <patch>`; the patch file is written from the iso diff. |
| `apply_back_conflict_returns_the_paths_git_names` | Scripted stderr `error: patch failed: src/a.cs:12` gives `["src/a.cs"]`; `apply` is not run. |
| `apply_back_conflict_without_paths_falls_back_to_written` | Unparseable stderr gives the task's written paths. |
| `release_removes_the_tree_and_the_patch_file` | Remove, prune, patch deleted; a failing remove is swallowed. |
| `sweep_removes_every_leftover_tree` | Two directories under `iso/` give two removals and one prune. |
| `isolation_guard_releases_on_drop` | Dropping `IsolatedTask` after an early return calls `release` once. |
| `isolated_task_ignores_a_sibling_files_write_attribution` (exec_loop) | `verify_task_scope` with `isolated = true` flags a write to a peer-owned path as `WroteOutsideFiles` (peer allowance off); with `false` it is still allowed. |
| `isolation_is_chosen_only_for_parallel_plans` (exec_loop) | Pure helper `should_isolate(alone, effective_parallel)`: false for alone, false for 1, true for 2 and 3. |
| `native_reason_texts_cover_isolation_failures` (enums) | The two new `as_str` values. |
| `session_job_workspace_override_reaches_the_start_request` (dispatch) | A job with `workspace = Some(p)` builds a request whose workspace path is `p`; `None` keeps the run workspace. |

Existing tests that must still pass: `exec_loop_tests.rs` (`flag_overlaps`), `scope_guard_task_writes.rs`, `native_plan_tasks.rs`. `native_plan_tasks.rs` runs tasks through `ScriptedProvider` on a real git workspace: its parallel cases now run in isolated trees and the fake provider writes into the session workspace path it receives, so check that the fake honors the request path (`Call` records it). If a case relied on the shared tree, adjust the fixture, not the product.

## Verification commands

```
cd "C:\CODE\Tools\aidp\.claude\worktrees\plan-task-isolation"
rtk cargo check -p aidp-gateway
rtk cargo nextest run -p aidp-gateway -E 'test(task_isolation) | test(exec_loop) | test(enums) | test(dispatch)'
rtk cargo clippy -p aidp-gateway --all-targets -- -D warnings
rtk cargo fmt
```

Do not run the `integration` profile or `it_*` tests. One command at a time.

## Out of scope

| Item | Reason |
| ---- | ------ |
| Serializing tasks | The user chose real isolation. |
| `StageSpec.isolation` for native runs | Workflow-mode field; the native decision is the plan's parallelism. |
| Three-way merge of overlapping tasks | The scheduler already serializes tasks that declare the same file; a leftover conflict fails the later task with a clear reason. |
| Copying git-ignored files (`node_modules`, `bin/`, `obj/`) into the isolated tree | A task that builds restores its own outputs. Revisit if tasks need prebuilt artifacts. |

## Open risk to check while implementing

| Risk | Check |
| ---- | ----- |
| Large untracked trees slow `prepare` | Copy only the files `ls-files -o --exclude-standard` returns; nothing ignored. |
| Windows long paths in `<run_dir>/iso/<task-id>` | Task ids are short (`T1`); enable no long-path hack. Log the path on `IsolationFailed`. |
| `git apply` line-ending conversion (CRLF repos) | Pass `--binary --full-index` on diff; apply without `--whitespace=fix`; set `-c core.autocrlf=false` on the iso `add`/`diff` when the ticket tree does not set it, so patches match the ticket's bytes. |
