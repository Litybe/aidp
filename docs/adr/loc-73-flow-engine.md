# ADR: ticket flow engine as a pure machine and a thin executor

Status: accepted

## Context

Nothing links several runs of one ticket into a sequence. The sequence cannot live in a Claude session, which would put an LLM in the loop, and the viewer must not hold business logic. The daemon already has sessions, a bus, an inbox, JSON stores and per-ticket worktrees.

## Decision

1. A new library crate `aidp-flows` holds the model, validation, the condition evaluator and `advance(def, state, event) -> (state, Vec<Effect>)`. It does no I/O and depends on neither `aidp-core` nor `aidp-integrations`.
2. `aidp-gateway/src/flows/` applies effects (action subprocess, session start, inbox entry, timer), feeds results back as events, and persists `state.json` plus a `journal.jsonl` line before each effect.
3. Each effect has a deterministic id `<step>:<iteration>:<kind>`. The executor owns `effects_applied` and never replays an applied id.
4. An unfinished session after a restart is resolved by lookup; if it cannot be found the step is `failed` with reason `orphaned`.
5. `escalated` is distinct from `failed`: a business exit-code family maps to `escalated`, technical failures to `failed`.
6. Definitions live in `templates/flows/` and are frozen with their version into `state.json`.
7. One run per ticket, and one writer per workspace through the admission lock shared with sessions.

## Consequences

- The machine is table-testable, and restart behavior is provable.
- One more library crate under the local gate: no `clap`, no I/O.
- The `state.json` format and effect ids become a compatibility contract.
- The registries of actions and probes are extension points filled by later lots.

## Alternatives rejected

- Compile a flow to a v7 pipeline template: needs an LLM session, has no human or timer steps outside a Workflow, and cannot resume without a session.
- Put the engine inside `aidp-gateway` only: machine tests would pull in the gateway, and `aidp flow validate` would depend on it.
