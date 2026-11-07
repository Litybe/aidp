# E2E Compilation Safety Net

## What the net covers

The safety net freezes the byte-level determinism of `aidp` compilation through two integration test files that lock down critical invariants without changing production behaviour.

**`crates/aidp-cli/tests/it_compile.rs`** exercises the full compilation pipeline:
- Two consecutive `workflow emit` passes produce identical output (byte determinism);
- Four insta goldens capture the emitted script structure and validate against regression;
- A six-element helper-block presence matrix (`RETRY_HELPERS`, `FANOUT_HELPERS`, `PLAN_HELPERS`, `POOL_HELPERS`, guard block, and the core `HELPERS` block) verifies that each helper is emitted if and only if the pipeline needs it (C8 correctness);
- A golden for `pipeline expand dev-composed` freezes the template-composition contract;
- All registered templates pass `validate --all` without error;
- Every template's emitted script survives `node --check` syntax validation via a wrapping transformation that replaces `export const meta` with `const meta` and encloses the body in an async IIFE.

**`crates/aidp-core/tests/prop_compile_invariants.rs`** applies property-based testing:
- Two proptests validate that the presence matrix remains correct across random template mutations and that emission determinism holds under realistic input variation.

This is a **freeze, not a fix** — no production behaviour changed. The net exists to surface any unintended regression during the LOC-37 `EmitCtx` refactor that rewrites the core emission logic.

## The template-count divergence

The LOC-16 ticket was written on 2026-09-17 against a measured `validate --all` result reporting `total: 39`. Counting `templates/*.yaml` files in the current tree yields 40 files, excluding the `templates/legacy/` directory (which is a directory, not a template file).

The test derives the expected template count from the filesystem at runtime (`template_ids().len()`) rather than hardcoding either number. This divergence is measured and reported — it is not silently resolved. The extra template between the ticket's 39 and the current 40 must be identified by whoever next touches this code. Use `git log --oneline -- templates/*.yaml` and `git diff HEAD~N templates/` to trace when and why a template was added after 2026-09-17.

## The `node --check` wrapping contract

The emitted script from `workflow_emit::assemble()` is a function **body**: it begins with `export const meta = {...}` and ends with a top-level `return { status: "done", ... }`. Running `node --check` on this raw output exits 1 with `SyntaxError: Illegal return statement`, because the return keyword is not inside a function scope.

The wrapping transformation (in `crates/aidp-cli/tests/common/mod.rs::wrap_for_node_check()`) performs:
1. Replace the first `export const meta` with `const meta` (removing the ES module export);
2. Wrap the entire file in `(async function(args){...});` (making it a valid function call expression).

After wrapping, `node --check` can validate the JavaScript syntax without executing it. The transformation is a test utility only — the emitted script itself is never executed through this path, and nothing in production runs `node --check` on the templates.

## Portability of the goldens

The goldens remain machine-independent through aggressive redaction:
- `HOME` and `USERPROFILE` are pointed at an empty tempdir during the test (`crates/aidp-cli/tests/common/mod.rs`), ensuring that `crates/aidp-core/src/skills.rs` resolves every skill to `(not found)` and no absolute path from the user's system survives in the snapshot.
- Run ids are redacted to `<RUN_ID>` in the expected output.
- Backslashes in paths are normalised to forward slashes so a golden generated on Windows matches one from Linux.

Without this redaction the goldens would be system-specific, machine-specific, and unportable.

## What is deliberately NOT frozen

The following are excluded from the safety net:
- `results[].path` returned by `validate --all` — these are absolute paths with mixed separators and are system-specific, so they are not frozen in the golden.
- The full shape of a `plan_tasks` template's pass-1 output from `plan/emit.rs` — only the presence of `PLAN_HELPERS` and `POOL_HELPERS` blocks is frozen; the rest of the plan-scheduling metadata belongs to LOC-17 and is left unfrozen.
- The executed behaviour of the emitted script — the test does not run the Workflow tool or invoke any stage. It validates syntax and structure only.

## How to re-bless a golden after an intentional compiler change

After modifying the emission logic in `crates/aidp-core/src/workflow_emit.rs` or related files, run the test and review the diff:

```bash
cargo insta test -p aidp-cli
```

Insta will generate `.snap.new` files for any diffs. Review each one carefully — the diff should match your intended change. Accept the goldens:

```bash
cargo insta accept
```

This workflow (generate, review, accept) is the intended path. **An unexplained golden diff during the LOC-37 refactor is the signal this net exists to raise.** If you do not expect a diff and one appears, stop and investigate before accepting.

## Verification

The file exists under `docs/`, every path it cites resolves in the repo, and it records both the ticket's measured count (39) and the current count (40) as separate facts, leaving the divergence documented for investigation rather than silently choosing one.
