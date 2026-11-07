# ADR: PR actions

Status: accepted

## Context

The viewer could read a PR's merge gate but not act on it.

## Decision

1. Put the PR mutations in `aidp-integrations::pr_actions`, shared by the `aidp gh` verbs and the daemon routes.
2. Recompute the merge gate uncached before every merge.
3. Squash only with `--match-head-commit`, so a push after the gate was read blocks the merge.
4. Delete the remote branch through the git refs API, not `--delete-branch`, which fails inside a worktree.

## Consequences

- The CLI and the daemon refuse the same merges for the same reasons.
- A merge from the viewer moves the ticket to `done` itself; the probe does not replay it.

## Alternatives rejected

- Merging on the cached gate: risks merging a stale verdict.
