# Exit code migration

`aidp` now returns one of eight codes, defined once as `aidp_core::ExitStatus` and shared by every
subcommand. See the README's "Exit codes" table for the full list. This note is for anyone whose
scripts or hooks checked a specific exit code before this change.

## The CLI-wide change

Every usage or parse fault — a bad flag, a missing required argument, a malformed config value — moved
from exit **2** to exit **7**. Exit 2 stays reserved for hook wake (a hook signalling Claude Code
to act); no command returns it today.

## Per-command changes

- `aidp scope check` refusing a path outside scope: 1 -> 4
- `aidp scope check` on an unknown stage: 1 with empty stdout -> 3 with `{"ok":false,"reason":...}` on stdout
- `aidp scope verify` reporting violations: 1 -> 6
- `aidp scope verify` catching a vault leak: 2 -> 4
- `aidp run reconcile` catching a vault leak: 2 -> 4
- `aidp doctor` reporting a red check: 1 -> 6
- `aidp cost anomaly` reporting an outlier: 1 -> 6
- `aidp validate` on a missing pipeline: 1 -> 3
- `aidp run state` on a missing run: 1 -> 3
- `aidp findings merge` with no `--input`: 1 -> 7
- `aidp knowledge supersede` with an invalid slug: 1 -> 4
- `aidp ticket show` on a missing ticket: 1 -> 3; its error is no longer duplicated as JSON on stdout

## `aidp gh pr-status`

`aidp gh pr-status` is a read. It exits 0 whatever the gate verdict, including a blocked gate; the
verdict is in the JSON. A missing or unauthenticated `gh`, or a failed GitHub call, exits 5, the
upstream family shared by the other `aidp gh` verbs.

## PR action verbs

`pr-ready`, `pr-update-branch`, `pr-merge`, `thread-reply` and `thread-resolve` exit:

- 0 on success.
- 6 when `thread-resolve` resolves only some of the threads.
- 4 when `pr-merge` is refused: the gate is closed or squash merge is disabled. The gate requires
  an approval only when GitHub reports a review decision, or when a readable branch rule demands
  approving reviews. A repository whose rules cannot be read requires none.
  With `--report-conflict`, a refusal caused by a conflict with the base alone exits 0 instead and
  prints `merged: false`, `conflict: true`.
- 5 on a missing or unauthenticated `gh`, or a failed GitHub call.

## PR detail verbs

`pr-detail`, `pr-files`, `pr-file-diff`, `comment-create|edit|delete`,
`review-comment-create|edit|delete` and `review-submit` exit:

- 0 on success.
- 4 when GitHub refuses on policy: a comment that is not yours, or a review on your own PR.
- 5 on a missing or unauthenticated `gh`, or a failed GitHub call.
- 7 on a usage fault, such as an unknown review event or a line outside the diff.

## Local review verbs

`review-post` and `thread-reopen` use the existing `aidp gh` codes:

- 0 on success, including a `review-post` that finds its marker already posted (`posted: false`).
- 4 when GitHub refuses on policy.
- 5 on a missing or unauthenticated `gh`, or a failed GitHub call.
- 7 on a usage fault, such as an unreadable findings file.

Workflow loop contract: after `pr-review` and after each `verify-review`, the workflow posts
`new_findings` with `review-post` and reopens each `reopen` thread with `thread-reopen`. The loop
exits when a pass is clean, or after the third pass with status `escalated`.

## Headless sessions

With `AIDP_HEADLESS_SESSION` set, `aidp flow`, `aidp gateway` and the `aidp gh` verbs that publish, merge
or dispatch a workflow exit 4 before they run. The list is in the
[host API contract](gateway-api.md#execution-identities).

## What to update

Hook scripts that branch on `aidp`'s exit code must stop treating exit 2 as a generic
failure — it is reserved for the hook-wake protocol and would otherwise be misread as a session
request. Any script that special-cased exit 1 for a "not found" or "policy refusal" condition should
switch to checking for 3 or 4 respectively, since those failures no longer share exit 1 with genuine
internal errors.

## `aidp flow`

- `aidp flow validate` on an invalid definition: 6; on a missing file: 3; a usage fault: 7
- `aidp flow show` on a missing file: 3
- `aidp flow list|start|status|approve|verdict|retry|skip|pause|resume|abort` with the daemon unreachable: 5
- `aidp flow start` on a ticket that already has an active run: 4 (the daemon answered 409)
- `aidp flow approve|verdict` on a step that is not waiting for a human: 4 (409)
- A key or step the daemon does not know: 3 (404)

## Exit codes of flow actions

A flow `action` step runs `aidp ... --format json-compact` and maps the exit code to a step status:

| Family | Codes | Step status |
|---|---|---|
| Technical | 1, 5, 7, or a timeout | `failed` |
| Business | 3, 4, 6 | `escalated` |
| Success | 0 | `done` |

With `on_error: warn` a failed or escalated action records a warning and the run continues. See [flows.md](flows.md).
An action failure without structured output records a configuration and host-log hint instead of an empty error.
Child stderr remains in the host logs; the journal does not copy potentially sensitive diagnostics.

## Flow action verbs

`aidp flow git-branch|git-push|pr-create` exit:

- 0 on success, including when nothing changed (`changed: false`).
- 4 on a business refusal: a remote branch carries another author's commits.
- 5 on an upstream failure: `git` or GitHub.
- 7 on a usage fault, including `flow pr-create` without a body.
- `flow git-push` exits 3 when the repository has no `origin`, 4 when it refuses the push (a foreign author's commits on the remote branch), 5 on an upstream failure, and 6 when a new branch has nothing to publish.
- `flow pr-create` reuses an open PR for the branch instead of creating a second one, and exits 7 when it must create one and gets no body.
