# ADR: flow actions as subprocesses, probes as in-process outcomes

Status: accepted

## Context

The flow engine has empty registries. Steps must move a ticket from assignment to merge without an LLM, can be interrupted at any point, and must not repeat a side effect after a restart.

## Decision

1. An action is a `SubprocessAction` over an `aidp` verb. The verb owns idempotency: it reads the current state, writes only when needed, and prints `{..., changed}`.
2. Composite work (branch creation, branch publishing, PR creation) is an `aidp flow ...` verb, so it can be tested and killed from the CLI. The gateway holds no such logic.
3. A probe is an in-process reader. `poll` returns a `ProbeOutcome` carrying `satisfied`, `outputs` and the probe `state` the executor persists between polls.
4. Probes read facts through a `PrStatusSource` backed by the PR status cache. They never call GitHub on their own.
5. Exit codes keep the existing families: 3, 4 and 6 are business and set the step `escalated`; other failures set it `failed`. Optional steps use `on_error: warn`.
6. The merge stays behind the step's human gate. No action merges by itself; the gate decides.

## Consequences

- Restart safety comes from the verbs, not from the executor replaying effects.
- Probe state is persisted in the wait record. A serde default keeps older records loadable.
- A new action is a verb plus a registry entry; a new probe is one file.

## Alternatives rejected

- Actions as in-process calls: a hung call cannot be killed and the gateway grows business logic.
- Probes that query GitHub directly: every poll spends rate limit and results differ from what the viewer shows.
