---
description: Health-check the ai-dev-pipeline-v7 install — binary, templates, hooks, schemas.
argument-hint: ""
---

Set `AIDP_SKILL_ROOT` to `~/.claude/skills/ai-dev-pipeline-v7`, then run `aidp doctor --format json-pretty`
and present the result to the user in plain language.

Procedure:

1. Run `aidp doctor --format json-pretty` with `AIDP_SKILL_ROOT` pointing at the **v7** skill root. (If the
   command itself fails because `aidp` is not on PATH, suggest `/pipeline-setup-v7`.)
2. Parse the JSON. It has shape `{ "ok": bool, "checks": [{ "name", "ok", "detail", "fix" }] }`.
   v7 adds the `vault` check (where pipeline output lands, and whether that directory exists),
   plus the observations `vault_ambiguous` (several vaults were in reach — pin one) and
   `legacy_runs` (pre-v7 runs still in the repo; they stay readable, nothing to do).
3. Sanity-check the `skill_root` check: its `detail` must point at `ai-dev-pipeline-v7`. If it points at
   another skill (e.g. v4), the env var is wrong — flag it and show the fix.
4. Render a markdown summary:
   - Header: `Doctor: <N>/<TOTAL> checks pass` (green if `ok: true`, red otherwise).
   - One line per check: `✅` or `❌` + check name + detail.
   - For every failed check, print a "Fix:" block with the `fix` field on a new line.
   - A red `aidp_on_path` lists issue slugs in its `detail`. Explain each: `absent` (no `aidp` on PATH),
     `target_build` (PATH resolves to a `target/debug` or worktree build), `mismatch` (PATH resolves a
     different binary than the one running), `older_than_running` (that binary is also older),
     `older_than_sources` (older than the newest Rust sources or `Cargo.toml`/`Cargo.lock`). Recommend
     `/pipeline-setup-v7`. Note that running doctor from a `cargo run` or worktree build is red by design.
5. If everything is OK, mention the next step: `/run-pipeline-v7 <template>` to start work, or
   `/pipeline-status-v7` to see existing runs.
