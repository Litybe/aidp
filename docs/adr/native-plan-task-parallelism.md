# ADR: Native plan task parallelism

Status: accepted

## Context

The native executor ran the tasks of a `plan_tasks` stage one session at a time, although the plan already declares each task's write set. Workflow mode runs disjoint tasks in parallel.

## Decision

1. Schedule tasks with a std-thread, dependency-driven scheduler. A task starts when its `needs` are done and its write set is disjoint from the running tasks; the rest wait in plan order.
2. Cap concurrency at `min(plan_tasks.max, settings.max_parallel)`. No new setting is introduced.
3. Attribute writes per task: git status plus a content hash before and after each task, compared with the task's `files`. Out-of-scope writes are restored and the task fails. `shared_files` are written only by the merge. Concurrent tasks run in isolated worktrees (see Isolation).
4. Add a shared-writer lane: the optional `StartRequest` field `shared_writer_group` lets the sessions of one run write the run worktree together. Other runs stay serialized per workspace.
5. `on_item_failure: pause` stops launching and waits for running tasks; `continue` lets the merge see every task state.
6. After a daemon crash, running rows go through recovery, and a resume reruns the rows that are not `Done`.

## Consequences

- Plans with disjoint tasks finish faster, with the same scope guarantees as the sequential path.
- Attribution costs one status and hash pass per task.
- A change that conflicts with an earlier task fails the later task, not as a silent overwrite.

## Alternatives rejected

- Wave chunks: waiting for the slowest task of each wave idles the cap, while a dependency-driven scheduler starts a task as soon as it is free.

## Isolation

Attributing writes by run window failed whenever two tasks ran at once: each saw the writes of the other, so both failed. Concurrent tasks (more than one task, effective parallelism above one) now each run in a detached git worktree under `<run_dir>/iso/<task-id>`, built from the ticket worktree's HEAD plus its uncommitted state and committed as a base. A succeeding task is applied back to the ticket worktree with `git apply` (checked first), so the merge and later stages still read one tree. A real conflict fails the later task with a clear reason. Sequential plans and a single task are unchanged. The design is in [plan-task-isolation](../plans/plan-task-isolation.md).
