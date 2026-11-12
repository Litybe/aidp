# AGENTS.md — aidp

Essential rules only. The development reference (layout, formatting, local gate, fast test loop,
test levels, versions) is [docs/development.md](docs/development.md); its `§` numbers are the ones
cited below.

## What this repo is

`aidp` (Rust, the cargo workspace at the repository root) compiles a YAML pipeline into a
deterministic JavaScript script run by Claude Code's `Workflow` tool, and runs the gateway daemon,
the local tickets and the flows. It is the harness of a solo Unity game
([docs/PLAN-GAMEDEV.md](docs/PLAN-GAMEDEV.md)). The pipelines themselves, the sub-agents and the
driver procedure are the skill, in its own local repository (`D:\AIDP\skill`); the binary reads it
through `AIDP_SKILL_ROOT`. End-to-end tests live in `D:\AIDP\src\aidp-e2e`.

| Path | Holds |
|---|---|
| `crates/` | `aidp-cli` (only crate that parses args and prints), `aidp-core`, `aidp-gateway` (daemon), `aidp-integrations`, `aidp-tickets`, `aidp-memory` (RAG + memory), `aidp-flows` (pure), `aidp-agents` (leaf), `aidp-testkit` (dev only), `aidp-workspace-hack` (cargo hakari) |
| `schema/` | JSON schemas `aidp` embeds and validates against |
| `fixtures/` | Test inputs: the frozen skill (`skill/`), mock answers (`mocks/`), template inputs |
| `scripts/`, `bootstrap.*` | Toolchain on D: (`setup-gamedev-toolchain.ps1`); build, install into `AIDP_INSTALL_DIR` (`D:\AIDP\bin`), PATH |
| `docs/` | `PLAN-GAMEDEV.md`, `development.md`, `gateway-api.md`, `tickets.md`, `flows.md`, `providers.md`, `native-execution.md`, `exit-codes.md`, `memory.md`, `doctor.md`, `adr/` |

## Rust commands

Run from the repository root.

- Check with `cargo check -p <crate>`; build the whole workspace only at the end of a task.
- Test with `cargo nextest run -p <crate> [-E 'test(<filter>)']`. A crate's integration tests are
  one binary: `--test integration -E 'test(/^<file>::/)'`.
- Unit vs integration: a unit test does no real I/O outside its own temp workspace (no
  process, port, network or git repository). An integration test runs a whole path with every
  external input mocked: the frozen skill under `fixtures/skill`, a temporary vault, the mock
  servers of `fixtures/mocks`, fake `git`/`claude`. Integration tests are every integration binary
  (`tests/*.rs`) and every test named `it_<name>`. `cargo nextest run` skips them; run them with
  `-P integration` (e.g. `cargo nextest run -p aidp-gateway -P integration -E 'test(/^concurrency::/)'`),
  all with `-P ci`. A new integration test goes at the root of the crate's `tests/` (a module of
  `tests/integration.rs`), never in `tests/unit/`. The `it_` prefix only marks existing
  unit-mounted tests not yet moved there.
- No test reads the real skill, the real vault or a file outside the repository:
  `workspace_boundary_gate` fails otherwise. Tests take templates from `aidp_testkit::fixtures`
  (§5).
- Before declaring done: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo nextest run --workspace`, `cargo nextest run --workspace -P integration`,
  `cargo test --workspace --doc`, and `cargo run -p aidp-cli -- validate --all` with
  `AIDP_SKILL_ROOT` pointing at the skill.
- Each checkout and worktree builds into its own `target/`. Never set or override `CARGO_TARGET_DIR`
  to a directory shared with another checkout: sessions block on its lock and rebuild each other's work.
- Thread counts come from `.config/nextest.toml` (`num-cpus` test threads) and
  `.cargo/config.toml` (8 jobs): never pass `--test-threads`, `-j`, `--jobs`, `NEXTEST_TEST_THREADS`
  or `RUST_TEST_THREADS`.
- Tests run through `cargo nextest` only; plain `cargo test` is for `--doc`.
- Never run two cargo commands at once on one target directory.
- Never `cargo build --release` to check a change (thin LTO, one codegen unit: minutes).
  The installed `aidp` (`D:\AIDP\bin`, `AIDP_INSTALL_DIR`) is refreshed only by `bootstrap.ps1`.
- Search with `rg` or the Grep tool, never `grep -r`: it walks `target/`.
- After adding or changing a dependency: `cargo hakari generate && cargo hakari manage-deps`.
- A new `tests/<name>.rs` needs a `mod <name>;` line in that crate's `tests/integration.rs`
  (`autotests = false`); a guard test fails otherwise.
- Never delete or loosen `rustfmt.toml`, `.cargo/config.toml` or `.config/hakari.toml`. Why each
  exists: §2, §4.

## Code conventions (enforced in review)

- No `//` comments in new code: `///` or `//!` docs only, two lines at most.
- No ticket or PR references (`LOC-xx`, `#nn`) in code or docs.
- No magic strings: constants for file names, input names and expected values, tests included.
  Constants live in the crate's `src/models/constants/`, one file per concept, at the top of files.
- Closed sets are enums in `src/models/enums/`, one enum per file; one data struct per file in
  `src/models/`, named after the type. Reuse an existing type before creating one.
- Serialize request bodies from a serde struct, never a hand-built `json!{}`.
- Explicit names everywhere, emitted JavaScript included (`reply`, `promptText`, not `r`, `p`).
- Unit tests: `tests/unit/<file>_tests.rs`, attached with
  `#[cfg(test)] #[path = "../tests/unit/<file>_tests.rs"] mod tests;`. Never an inline test module.
- Test YAML lives in `tests/fixtures/<topic>/`, loaded with `include_str!`.
- Assert exact statuses and codes (never `>= 500`).
- `unsafe` is forbidden (`unsafe_code = "forbid"` in the workspace lints).
- Logs go through `tracing` to stderr. Stdout is the machine contract (bare `run_id`, bare script
  path, JSON).

## Invariants the gate checks

- No `clap` in `aidp-integrations`, `aidp-gateway`, `aidp-tickets`; no `derive(Subcommand)` there.
- `aidp-flows` depends on none of `clap`, `aidp-core`, `aidp-integrations`.
- No `tokio` in the dependency tree (`cargo tree -i tokio` prints nothing).
- `cargo deny check` and `cargo hakari verify` pass.
- Every crate inherits workspace lints and has `license.workspace = true`.

## Changing schemas or behavior

- An emitter change shifts the emitted prompts: regenerate the goldens with
  `AIDP_UPDATE_GOLDEN=1 cargo nextest run -p aidp-core --lib -E 'test(golden)'` and read the diff.
- A new stage or settings field needs its `schema/*.json` entry, its emitter branch and a test.
- A change the skill can depend on bumps the workspace version; the skill then raises
  `min_aidp_version` once that binary is installed (§6).
- A behavior change the driver can see needs its doc update here (`docs/`) and a follow-up in the
  skill repository `D:\AIDP\skill` (`SKILL.md`, `docs/agents-reference.md`, `USER-GUIDE.md`).

## Git

- Work on a branch, then merge it into `main`; never commit to `main` directly.
- Run the gate of "Rust commands" before merging: there is no remote CI.
- Commit messages follow Conventional Commits.
