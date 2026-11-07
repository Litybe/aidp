# ADR: PR status and merge gate, read-only

Status: accepted

## Context

The daemon reports a pull request only as a state and raw checks (`aidp-gateway/src/changes.rs`). No tool says whether a PR can merge, or why not. `aidp gh pr-view`, `pr-checks` and `pr-review-threads` each give a part, with no verdict.

## Decision

1. Put the pure logic in `aidp-integrations/src/pr.rs`: `checks_verdict`, `review_state` and `merge_gate`. They take plain data and return values, so table tests cover them. The PR status GraphQL query and the gate verdicts both live in `crates/aidp-integrations/src/pr.rs`, shared by `aidp gh pr-status` and the gateway.
2. Put the GitHub reads beside the other `gh` verbs: one GraphQL query, one REST compare when the PR is open, one read of the base branch rules. `aidp gh pr-status` prints the result and exits 0 whatever the verdict.
3. Put the daemon side in `aidp-gateway/src/prs.rs`: a `Mutex<HashMap>` cache keyed by repo and number (20 s), a second one for branch rules (1 h), the routes `GET /api/prs/status` and `GET /api/tickets/:key/pr`, and the `pr.status` bus event.
4. Invalidate from the existing GitHub webhook handler on the six PR events, independent of automation matching.
5. Add no dependency. The caches are std maps behind a `Mutex`, with timestamps.
6. `changes.rs` keeps its rule against per-request GitHub calls. `prs.rs` is the one exception, bounded by the TTL cache, and the note in `changes.rs` says so.

## Consequences

- The merge gate reads the same in the CLI, the daemon and later clients.
- Without a webhook, a status can be up to 20 s stale; `refresh=1` forces a read.
- Unreadable checks or reviews block the gate; unreadable threads or rules do not, as on GitHub.
- Expired entries are replaced on read; the entry count is bounded by the PRs clients ask about.

## Alternatives rejected

- A caching crate: a new dependency for two small maps.
- Evaluating the gate in each client: duplicates the rules and drifts.
- Calling GitHub on every request: costs rate limit and latency.

## Amendment: approval rule

The `approved` item trusts a non-empty GitHub `reviewDecision`. Without one, it requires approvals only when a readable `pull_request` branch rule sets `required_approving_review_count` above 0, and compares the approving reviewers to that count.

Unreadable rules (403, or a repository without protection) stay degraded: `degraded` lists `rules`, but the item passes with the reason "no approval rule on this repository". Failing closed would make every PR on an unprotected repository unmergeable. `gate.items[].reason` now appears on passing items too; `gate.reason` remains the first failing item's sentence.
