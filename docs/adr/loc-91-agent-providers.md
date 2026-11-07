# ADR: Agent provider seam

Status: accepted

## Context

The workspace launches agents by building a `claude` command line inside the gateway. Any other agent program would need the engine to change. There is no neutral description of what an agent turn needs or produces.

## Decision

1. Add the leaf crate `aidp-agents` with the `AgentProvider` trait, `AgentSpec`, `AgentHandle`, the event enum and `Capabilities`. It has no dependency on `aidp-core`.
2. Choose Option B: the provider layers above `ProcessRunner`. `ClaudeCodeProvider` in `aidp-gateway` wraps the existing `ClaudeProcessRunner`. It does not replace `SessionManager.runner`.
3. Pin the `claude` argv with a golden test. It does not change.
4. Add `FakeProvider` to `aidp-testkit` for tests without an LLM.
5. Read configuration from the `providers` key of `~/.aidp/config.json`. Resolve in this order: stage `provider:`, template `settings.providers.<role>`, launch override, `providers.default`, `claude`. Refuse an unknown provider by name.
6. Report provider health in `aidp doctor` and in `GET /api/health`.

## Consequences

- Existing sessions behave exactly as before.
- A second provider needs one trait implementation and a config entry.
- Pipeline execution through a provider and a second real provider remain to be done.
- Template authors get two optional fields, `provider:` and `settings.providers`.

## Alternatives rejected

- Replace `SessionManager.runner` with the provider: touches every session path at once and risks the pinned argv.
- Put the trait in `aidp-core`: couples the seam to the engine and blocks reuse by the gateway and testkit.
