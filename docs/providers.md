# Agent providers

## Overview

A provider runs agent conversations and streams their events. Claude Code is the only provider built
in; `fake`, from `aidp-testkit`, replays scripted events in tests. The native Rust executor and the
legacy Workflow execution both run on Claude Code.

`aidp-agents` is a leaf crate. It does not depend on `aidp-core`.

## Contract

```rust
trait AgentProvider {
    fn id(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn start(&self, spec: AgentSpec) -> AgentHandle;
}
```

`AgentSpec` carries everything a provider needs to start:

- `work_dir`, the directory the agent runs in.
- `prompt`, the user prompt.
- `persona`, passed as the system prompt.
- the resolved model id and the effort.
- allowed and denied tools, taken from `toolset.rs`.
- environment: `AIDP_RUN`, `AIDP_STAGE`, `AIDP_BIN`.
- the permission policy, a timeout and a budget.

`AgentHandle` is a synchronous event stream with session controls:

- Events include text, tools, usage, permission requests, raw stream-json lines and the exit.
- `cancel()` stops the agent.
- `reply_permission()` answers a `PermissionRequest`.
- `wait()` blocks until the agent exits.

`Capabilities` tells the engine what a provider can do: native skills, hooks, permission host, structured output, session resume, cost reporting. The engine adapts to what is declared instead of assuming Claude behaviour.

Implementations:

- `ClaudeCodeProvider` in `aidp-gateway` wraps the existing `ClaudeProcessRunner` and `ProcessRunner`. The `claude` argv it builds is pinned by a golden test and is unchanged.
- `FakeProvider` in `aidp-testkit` plays a scripted list of events and files.

## Config

Providers are configured under the `providers` key of `~/.aidp/config.json`:

```json
{
  "providers": {
    "default": "claude",
    "claude": {
      "program": "claude",
      "args": [],
      "models": { "opus": "claude-opus-5-5" }
    }
  }
}
```

- `default` names the provider used when nothing else selects one.
- `program` and `args` give the executable and its extra arguments.
- `models` maps a tier (`fable`, `opus`, `sonnet`, `haiku`) to a model id. A tier with no mapping is passed unchanged.

Check `crates/aidp-agents/src/config.rs` for the exact shape.

## Resolution

The provider for a stage is the first of:

1. the stage `provider:` field,
2. the template `settings.providers.<role>`, where role is `analyzer`, `planner`, `executor` or `reviewer`,
3. the native launch's `providerByRole.<role>`, then its `provider` override,
4. `providers.default`,
5. `claude`.

Only `claude` is built in (and `fake`, registered by tests). Any other id, `codex` and `copilot`
included, is refused with a message that names it: by the config loader for `providers.default`,
by `aidp validate` for the template fields, and at launch for a stage, a session or a native run.
An id declared under `providers` without an implementation passes validation and is refused at launch.

Both template fields are optional and declared in `schema/pipeline.schema.json`. No shipped template uses them.

## Autonomous profile

Sessions of an autonomous native run use the `autonomous` profile: `AskUserQuestion` is not available, the permission policy is `allow` with a deny list, and the session never asks. Free autonomous sessions share the profile. See [native-execution.md](native-execution.md).

## Health

- `aidp doctor` has a `providers` check: for each configured provider, the binary is found and answers `--version`.
- `GET /api/health` returns a `providers` array of `{id, program, found, version}`. See [gateway-api.md](gateway-api.md).

## Testing with FakeProvider

`FakeProvider` replays a scripted sequence of events and writes the files a real agent would write (`reply.md`, declared outputs). Tests that need an agent run without an LLM, a network or a `claude` binary.

## Compatibility boundaries

Restricted launches accept `strictMcpConfig`, `mcpConfigId`, `mcpServers`, `deniedShellCommands` and
`deniedMcpTools`. An explicit empty server list disables ambient MCP configuration.
Nonempty inline server definitions are refused before a native run or flow is persisted.
Named `mcpConfigId` references resolve an in-memory configuration at session launch.
A gateway session resume inherits its parent's reference when the caller omits it; unavailable
references refuse the resumed launch before saving a new session.
Provider adapters receive an immutable snapshot; replacing the named configuration affects future
launches. Missing references fail explicitly, including after daemon restart unless the host supplies
`initialMcpConfigurations` before recovery.

Claude uses `--strict-mcp-config`; named definitions use environment placeholders in the config
argument and secret values in the child environment. Shell prefixes become both `Bash(prefix:*)`
and `Bash(rtk prefix:*)` deny rules. Named MCP tools become `mcp__server__tool` deny rules.
Existing autonomous deny rules are preserved.

Shell rules match command prefixes. They are not an OS sandbox: an absolute executable path,
another wrapper or a command absent from the list can bypass a prefix rule.

Claude `default` and `acceptEdits` launches host tool approvals over stdio, unless the launch is
headless. A headless Claude launch runs `--permission-mode dontAsk` when it asked no mode or
`default`, with no `--permission-prompt-tool`: Claude denies any tool call, edits included, that the
`permissions.allow` rules of its settings do not grant. The host owns that allowlist. aidp also denies a headless Claude
launch the verbs that publish, merge or deploy; see the [host API contract](gateway-api.md#execution-identities). A headless Claude launch also appends a denial note to its system prompt, after any persona: a denial hits one exact command, so the agent retries without an `rtk` prefix, splits compound commands or uses a dedicated tool before it reports a stage blocked. Requests of other launches appear as
serialized Allow/Deny questions. An accepted response preserves the original tool input.
Malformed or stale answers fail synchronously and leave the pending request intact.
Question replies may include `toolUseId` to bind the answer to the displayed request.
A mismatched identifier fails before consuming any request; clients omitting it remain compatible.
Session cancellation and daemon shutdown wait for the provider to confirm exit before publishing a terminal status.

Native runs honor explicit model, effort and permission-mode overrides on every stage.
An explicitly requested permission mode is never replaced by the historical fallback; a reported
mismatch stops the session and fails the stage. Requests without a permission override keep the
existing bypass-to-auto fallback, unless they are headless: those ask `dontAsk` on every stage.
