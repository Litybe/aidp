# Developing aidp

This repository holds `aidp`: the Rust workspace, the JSON schemas it embeds, the install scripts and
the docs of the gateway, tickets, flows and providers. It is the harness of a solo Unity game
([PLAN-GAMEDEV.md](PLAN-GAMEDEV.md)). Two other local repositories complete it, all on drive D:

| Repository | Holds |
|---|---|
| this one, `D:\AIDP\src\aidp` | the `aidp` binary and its contracts |
| `D:\AIDP\skill` | the Claude Code skill: templates, agents, commands, hooks, `SKILL.md` and its reference |
| `D:\AIDP\src\aidp-e2e` | end-to-end tests: the installed binary against the real skill, no mock |

The binary reads the skill through `AIDP_SKILL_ROOT`, else `~/.claude/skills/ai-dev-pipeline-v7`.
Nothing in this repository reads the skill at build or test time.

## 1. Layout

```
Cargo.toml / Cargo.lock # the workspace; every cargo command runs from the repository root
schema/                 # JSON schemas, embedded with include_str! (plan, pipeline, findings, reply...)
fixtures/               # test inputs only:
                        #   skill/   a frozen copy of the skill (SKILL.md, templates, agents, hooks,
                        #            commands) that unit and integration tests run against
                        #   mocks/   data directory of `aidp mock` (GitHub routes, state in memory)
                        #   template-inputs.json  inputs for the templates of fixtures/skill
scripts/                # aidp-install.ps1 / aidp-install.sh: sourceable install + PATH-migration
                        #   functions the bootstrap uses; covered by crates/aidp-cli/tests/it_bootstrap.rs
                        # setup-gamedev-toolchain.ps1: Rust GNU, MinGW and cargo-nextest on D:, and the
                        #   user variables (AIDP_INSTALL_DIR, AIDP_GATEWAY_DIR...); removes CARGO_TARGET_DIR
bootstrap.ps1 / .sh     # build, install into AIDP_INSTALL_DIR (D:\AIDP\bin), put it on PATH
docs/                   # PLAN-GAMEDEV, gateway-api, tickets, flows, providers, native-execution,
                        #   exit-codes, adr/
crates/                 # the aidp workspace. aidp-cli is the composition root:
                        #   src/args.rs (+ args/<domain>.rs) — the clap surface;
                        #   src/ctx.rs — `Ctx`, holding the Clock/Git/ProcessRunner/IdGen ports, with the
                        #     vault resolved lazily per command;
                        #   src/commands/<verb>.rs — one value-returning function per top-level subcommand.
                        #   aidp-tickets, aidp-gateway and aidp-integrations depend on neither clap nor print.
                        #   Pure helpers live in aidp-core: text::levenshtein, cost::anomaly,
                        #   cost::repricing, and the typed hooks merge. The local gate forbids clap in
                        #   those libraries (§3).
                        #   aidp-agents — leaf crate, no aidp-core dependency: the `AgentProvider` trait,
                        #   `AgentSpec`/`AgentHandle`, events and capabilities (docs/providers.md)
                        #   Crates: (aidp-cli, aidp-core, aidp-integrations,
                        #   aidp-workspace-hack — feature unification, managed by cargo hakari (§4),
                        #   aidp-gateway — resident control plane: sessions, automations, HTTP+SSE,
                        #   aidp-tickets — local ticket store behind `aidp ticket` (docs/tickets.md);
                        #   no dependency on aidp-integrations,
                        #   aidp-flows — pure ticket-flow engine behind `aidp flow` (docs/flows.md):
                        #   model, validation, conditions, `advance` state machine; no HTTP, no
                        #   subprocess, no dependency on aidp-core or aidp-integrations,
                        #   aidp-gateway/src/flows/ — the flow executor (effects, store, recovery),
                        #   aidp-testkit — dev-only, consumed exclusively via [dev-dependencies])
                        #   aidp-core/src/exec_plan/   — `compile_exec_plan`: the pure, JS-free plan of routing
                        #                                 decisions; workflow.js renders it byte for byte
                        #   aidp-core/src/guard.rs     — one prompt-injection pattern table, two
                        #                                 consumers: Rust `scan()` and the emitted
                        #                                 `__guard(txt, src)` JS helper (skill reference §6)
                        #   aidp-core/src/findings.rs  — `aidp findings merge`: deterministic findings
                        #                                 merge + three-state `checks[]` (skill reference §9.2)
                        #   aidp-core/src/toolset.rs   — per-stage `agentType` / tool allowlist (skill reference §10)
                        #   aidp-core/src/workflow_emit/ — submodules of the emitter (contract, plan-section
                        #                                 and schema builders); `EmitCtx` is the immutable
                        #                                 emitter context, `EmitOut` the mutable output
                        #   aidp-core/assets/js/       — the static Workflow helper JS (helpers, retry_helpers,
                        #                                 fanout_helpers, plan_helpers, pool_helpers), embedded
                        #                                 with `include_str!`; `tests/js_syntax.rs` runs
                        #                                 `node --check` on each file
                        #   aidp-core/src/clock.rs     — Clock/SystemClock/FixedClock port
                        #   aidp-core/src/id.rs        — IdGen/SystemIdGen/FixedIdGen port
                        #   aidp-core/src/git.rs       — Git/GitCli/RecordingGit port
                        #   aidp-core/src/process_runner.rs — ProcessRunner/OsProcessRunner/
                        #                                 ScriptedProcessRunner port
                        #   aidp-core/src/cost.rs      — PriceTable::from_env_or_embedded reads
                        #                                 AIDP_MODEL_PRICES explicitly, replacing
                        #                                 the old process-global OnceCell
                        #   aidp-core/src/run/         — run state, one submodule per concern; mod.rs
                        #                                 re-exports the public facade unchanged:
                        #                                 layout.rs (run dir resolution, load/save),
                        #                                 init.rs, gates.rs, cost.rs, render.rs,
                        #                                 fold.rs (pure `fold_workflow_result`: next
                        #                                 RunState from a Workflow result, no I/O),
                        #                                 reconcile.rs (fold, then git/scope only on
                        #                                 a done result, then cost and save).
                        #                                 Tests: co-located pure tests, plus
                        #                                 tests/run_fs.rs and tests/run_git.rs
                        #   aidp-gateway now depends on aidp-core; claude spawning lives in
                        #   aidp-gateway/src/claude_runner.rs behind SessionManager::load_with_runner
```

## 2. Formatting (`rustfmt.toml`)

`rustfmt.toml` exists and must not be deleted or "simplified away" — without it the `cargo
fmt --all -- --check` step of the local gate (§3) and of the `dev-aidp` template fails permanently
on style drift that has nothing to do with the change under review, defeating the gate's purpose
(a real regression becomes invisible in a wall of expected failures). It pins `edition = "2021"`
to match the workspace `Cargo.toml`; measuring the actual line-width distribution across
`crates/*/src/**.rs` showed >97% of lines under 100 columns, so stable rustfmt's default
`max_width` already matches this repo's style — the file exists to lock that in explicitly, not to
raise the limit. The pre-2026-09 drift (37 files failing `--check`) was not a width problem: it was
compact single-line blocks (`fn f() -> T { x }`, inline `#[attr] field: T`) that stable rustfmt has
no option to preserve, meaning those files had simply never been run through `cargo fmt`. They were
normalized with one mechanical `cargo fmt -p <crate>` pass per crate, committed separately from any
behavioral change. Formatting is enforced by the local gate (§3), and `rustfmt.toml` remains pinned
to `edition = "2021"` to ensure consistent formatting across the workspace.

## 3. The local gate

There is no remote CI: run the gate yourself before merging into `main`. The Rust workspace is at the repository root; every cargo command runs from there. The toolchain is pinned by `rust-toolchain.toml` to `1.95.0` (installed on D: as `1.95.0-x86_64-pc-windows-gnu` by `scripts/setup-gamedev-toolchain.ps1`), and the MSRV is enforced in `Cargo.toml` via `rust-version = "1.95"`.

Run, in order:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo hakari generate --diff && cargo hakari manage-deps --dry-run && cargo hakari verify
cargo nextest run --workspace
cargo nextest run --workspace -P integration
cargo test --workspace --doc
cargo deny check
```

The toolchain script installs `cargo-nextest`. The other tools are installed once, into `CARGO_HOME`, on drive D:

```bash
cargo install cargo-deny
cargo install cargo-hakari --locked
```

nextest does not run doc-tests, hence the separate `cargo test --workspace --doc`.

With `node` on PATH, the test run also includes the `js_syntax` check (`node --check` on each static Workflow helper); without it, that test prints a `SKIP: node not found on PATH` line.

`tokio` must stay out of the dependency tree: `cargo tree -i tokio` must print no match.

The gate also keeps `clap` out of the library crates `aidp-tickets`, `aidp-gateway` and `aidp-integrations` (only `aidp-cli` parses arguments and prints), and `aidp-flows` pure. Each command below must report no match:

```bash
cargo tree -p aidp-integrations -p aidp-gateway -p aidp-tickets -i clap -e normal
cargo tree -p aidp-flows -i clap -e normal
cargo tree -p aidp-flows -i aidp-core -e normal
cargo tree -p aidp-flows -i aidp-integrations -e normal
grep -rn 'derive(Subcommand' crates/aidp-integrations crates/aidp-gateway crates/aidp-tickets
```

`unsafe_code = "forbid"` is the workspace default for every crate.

Test and logging conventions have three deliberate exceptions. Most unit tests live in `tests/unit/<module>_tests.rs`, attached from the source file with `#[path]`, and the files at a crate's `tests/` root (`aidp-cli`'s `it_*.rs` included) are modules of one `integration` binary per crate, declared in `tests/integration.rs` under `autotests = false` (§4). A new integration file is one more `mod` line there, not a new binary. Both layouts remain valid. The PR status tests are the exception: they live under `src/tests/units/` of their crate (`pr_tests.rs` in `aidp-integrations`, `prs_tests.rs` and `pr_routes.rs` in `aidp-gateway`, `it_gh_pr_status.rs` in `aidp-cli`), with fixtures in `src/tests/units/fixtures/`. A unit test there is attached with `#[path = "tests/units/<module>_tests.rs"]`. An integration binary placed there is declared by a `[[test]]` `path` entry in the crate's `Cargo.toml` and reaches the shared helpers in `tests/common/` through `#[path]`. Logging goes through `tracing` with the default `module_path!()` target; only `aidp-cli`'s `main.rs` sets a `LOG_TARGET`, because the binary crate name differs from the library module path.

## 4. Building and running tests fast while developing

Measured on Windows (12 cores), before and after the tuning below:

| Loop | Before | After |
|---|---|---|
| Edit `aidp-core`, rebuild every test target | 357 s | 87 s |
| Edit `aidp-core`, `cargo build -p aidp-core --tests` | 62 s | 18 s |
| Switch from `-p <crate>` to `--workspace` | 22+ crates rebuilt | 0 |
| One in-process `reconcile` test | 467 s | 0.3 s |
| `cargo nextest run --workspace` | 598 s | 248 s |

Commands, from the repository root:

- Check with `cargo check -p <crate>`; it skips codegen and linking. Build the whole workspace only at the end.
- Test the crate you changed: `cargo nextest run -p <crate>`. Narrow further with `--test integration -E 'test(/^run_fs_reconcile::/)'` or `--lib -E 'test(<name>)'`.
- Run `cargo nextest run --workspace` and `cargo test --workspace --doc` once, at the end, never in a loop. Plain `cargo test` works but is slower, and it runs a crate's integration tests as threads of one process (see below).
- Never run two cargo commands at once on the same target directory: they block on its lock. Each checkout and worktree uses its own `target/`: `scripts/setup-gamedev-toolchain.ps1` removes the user variable `CARGO_TARGET_DIR` because a shared target made sessions block on its lock and rebuild everything each time another one built. Never set it to a shared directory.
- `concurrency_background::scenario_10_scheduler_never_overlaps_one_automation` is `#[ignore]` (about 2 min waiting for cron boundaries). Run it with `cargo nextest run -p aidp-gateway --test concurrency_background --run-ignored only --profile ci`, only when touching the automation scheduler.

What makes it fast, and what must stay true:

- **One integration test binary per crate.** Each `tests/*.rs` file used to be its own binary, 84 in all, each linking the whole dependency graph. They are now modules of `tests/integration.rs` (`autotests = false`, explicit `[[test]]` entries). Shared helpers are `mod common;` in `integration.rs` and `use crate::common;` in each file. insta snapshot names gained an `integration__` prefix. Files that call `std::env::set_var` behind a file-local lock keep a binary of their own, since a lock cannot isolate them from other files in one process: `clock_ports`, `invalid_prices_reconcile`, `ledger_error_reconcile`, `run_fs_vault`, `vault_resolve_global` (`aidp-core`), `home_precedence` (`aidp-integrations`). `concurrency_background` (`aidp-gateway`) keeps its own too, for the command above. Tests that change the environment through `aidp_testkit::EnvGuard` share its process-wide lock and can be merged. nextest runs every test in a process of its own, so env changes never leak there.
- **A test that re-executes its own binary** must name itself through `module_path!()`, not a hard-coded `module::test` path (see `daemon_child_test_name` in `aidp-gateway/tests/http_hardening.rs`).
- **`aidp-workspace-hack`** (managed by `cargo hakari`, config in `.config/hakari.toml`) pins one feature set per third-party dependency. Without it, `-p aidp-tickets` and `--workspace` compiled `log`, `tracing-core`, `jsonschema` or `rustls` with different features, so every switch rebuilt them. After adding or changing a dependency, run `cargo hakari generate && cargo hakari manage-deps`; `cargo hakari verify` fails on a stale hack. clap is excluded so the library crates keep no clap dependency, and `aidp-flows` / `aidp-agents` do not depend on the hack.
- **`aidp-testkit` does not depend on `aidp-integrations`**, so `aidp-core`'s tests no longer wait for `aidp-tickets` and `aidp-integrations` to compile. Boot the mock server with `aidp_integrations::mock::serve_on` directly.
- **Hermetic transcripts.** `.cargo/config.toml` sets `AIDP_CLAUDE_PROJECTS_DIR` to the empty `.cargo/test-claude-projects/` for every test run. Without it, in-process `reconcile` tests scanned the real `~/.claude/projects` (3 GB here). A test that needs transcripts sets the variable itself.
- **Dev profile.** `debug = "line-tables-only"` for the workspace and no debuginfo for dependencies keep `target/debug` small (full debuginfo reached 61 GB). Incremental compilation stays on: turning it off tripled the edit loop. `opt-level` for dependencies was measured and rejected: the slow tests spend their time in `aidp-core` itself, and it cost a 5-minute rebuild of every dependency.
- **sccache, opt-in.** With `rustc-wrapper = "sccache"` in your own `~/.cargo/config.toml` (never in the repo: other machines do not have it), a new worktree reuses the dependencies another one compiled. `.cargo/config.toml` sets `SCCACHE_BASEDIRS` to the checkout, without which the cache keys hold the target path and nothing ever hits. Measured: a new worktree's first build went from 180 s to 147 s (159 of 184 units from the cache). The workspace crates themselves are incremental, which sccache does not cache.
- `.cargo/config.toml` sets `[build] jobs = 8`, so two sessions building at once leave the 16-core machine usable, caps `RUST_TEST_THREADS` at 4 for plain `cargo test` (doc-tests), and links with `rust-lld` on the Windows MSVC target.
- nextest concurrency is pinned to `num-cpus` (`.config/nextest.toml`); cargo builds are capped at 8 jobs (`.cargo/config.toml`). Do not pass thread flags or variables.
- A nextest test group capping the daemon-backed `aidp-gateway` tests at 2 was measured slower (282 s vs 192 s) because it serialized the longest tests.
- **The fake `gh` of the `aidp gh` tests** is compiled once per source hash into `$CARGO_TARGET_TMPDIR/gh_shim_cache`, under a per-hash file lock, then copied into each test. It used to be rustc once per test (about 30 s each). Cold cache: 3 compiles; warm: the 24 `it_gh_` tests run in under 2 s.
- On a network that intercepts TLS, `cargo install cargo-nextest` (or `cargo-hakari`) can fail with `CRYPT_E_NO_REVOCATION_CHECK`; the one-off workaround is `CARGO_HTTP_CHECK_REVOKE=false` on that command only.
- Tests that loop over templates run them on a bounded `std::thread::scope` pool: at most 4 workers, one temp workspace each, no environment mutation inside a worker. Failures are collected and reported in a single panic that lists every failing template.
- **Source rules live in one test**, `repository_sources_respect_every_gate`, which walks the repository once, reads files in parallel and reports every violation as `[rule] path:line`. Add a new rule there, not a new scanning test.
- **Tests that need a git repository use `aidp_testkit::GitFixture`**: the template is built once under `target/aidp-git-fixture/` and copied per test, instead of running `git init`, `commit` and `push` in each test.
- **The doctor tests run on the in-process fake git** (`crates/aidp-gateway/tests/fake_git/`); one `_on_real_git` twin per module keeps the real path covered. A new git command on the doctor path needs its fake branch.
- **Never call `wait_for` while a run sits at a gate**: wait for the `WaitingGate` status instead.
- No cross-test cache is shared between these tests, because nextest runs each test in its own process. `rayon` is deliberately not a dependency.

## 5. Test levels

| Level | What it is | Where |
|---|---|---|
| Unit | One unit of code, inputs given by the test, no real I/O outside its own temp workspace | `tests/unit/<file>_tests.rs`, run by `cargo nextest run` |
| Integration | A whole path with every external input mocked: the frozen skill of `fixtures/skill`, a temp vault, `fixtures/mocks`, scripted processes | each crate's `tests/*.rs` and every test named `it_<name>`, run by `cargo nextest run -P integration` |
| End to end | The same paths with no mock: the installed binary, the real skill, real tools | `D:\AIDP\src\aidp-e2e` |

`AIDP_TEST_LAN=1` lets `crates/aidp-gateway/tests/access_guard.rs` bind the daemon to the machine's LAN IP. Off by default: each new test binary would otherwise trigger a Windows firewall prompt. Without it those tests skip with a message.

Tests reach `fixtures/` through `aidp_testkit::fixtures` (`skill_root()`, `templates_dir()`,
`template(id)`, `mocks_dir()`, `template_inputs()`), never through a relative path of their own.

`repository_sources_respect_every_gate` (`crates/aidp-core/tests/repository_source_gates.rs`) enforces this: every `include_str!`,
`include_bytes!`, `#[path]` and `CARGO_MANIFEST_DIR` path must stay inside the repository, and none
may name `templates`, `agents`, `commands`, `hooks`, `SKILL.md`, `USER-GUIDE.md` or `benchmark` at
the repository root.

`fixtures/skill` is a snapshot, not a mirror. Refresh it from the skill repository when a test needs
a template feature it lacks, in its own commit, then regenerate the goldens:
`AIDP_UPDATE_GOLDEN=1 cargo nextest run -p aidp-core --lib -E 'test(golden)'`.

## 6. Versions and compatibility with the skill

The workspace version (`[workspace.package] version` in `Cargo.toml`) is the version `aidp --version`
prints. Bump it in any change the skill can depend on: a new template field, stage kind, subcommand
or flag.

The skill's `SKILL.md` declares `min_aidp_version` in its frontmatter. `aidp doctor` reads it from
the resolved skill root and reports a binary older than it as a red `skill_compat` check. A skill
change that needs a new binary raises `min_aidp_version` once that binary is installed.

To ship a version: bump it on a branch, merge it into `main`, then run `bootstrap.ps1`, which builds
the release binary and installs it into `AIDP_INSTALL_DIR` (`D:\AIDP\bin`).

## Agents, commands and skills: one source

Agent personas, slash commands and skills are read from Claude's own directories
(`~/.claude/agents`, `~/.claude/commands`, `~/.claude/skills`), never from the skill root.
The skill root (`AIDP_SKILL_ROOT`) holds only the pipeline templates and flows.
