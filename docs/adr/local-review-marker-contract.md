# ADR: Local review marker contract

Status: accepted

## Context

A local PR review must post its findings on GitHub, then re-check them on later passes. A retry or a resumed run must not post a finding twice, and a thread the author fixed wrongly must come back.

## Decision

1. Compose the existing GitHub write primitives into two verbs: `aidp gh review-post` and `aidp gh thread-reopen`. No new GitHub client.
2. Make posting idempotent with a marker in each comment, `<!-- aidp:finding id=<fid> run=<run_id> pass=<n> -->`. The key is (run, pass, finding id); a comment already carrying the marker is skipped.
3. Let the workflow own the loop and the pass counter. After each review pass it posts `new_findings` and reopens `reopen` threads; it stops when clean, or after pass 3 with status `escalated`.
4. Put out-of-diff findings in the review body with their `location`, because GitHub accepts inline comments only on diff lines.
5. Keep templates read-only toward GitHub: agents write `findings.json` and `verify.json`, and only the workflow calls the verbs.

## Consequences

- A retry posts nothing twice, and a later pass can find the thread of an earlier finding from its marker.
- The pass counter lives in one place, so a template cannot loop forever.
- A finding outside the diff has no thread to reopen; the next pass reports it again.

## Alternatives rejected

- Agents posting through `gh` themselves: no idempotence, and a resumed stage would duplicate comments.
- Idempotence keyed on the comment text: an edited finding would be posted again.
- A pass counter held by the agent: the agent can miscount, and the loop would not stop.
