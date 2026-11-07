# ADR: one flag contract for the aidp CLI

Status: accepted

## Context

`--format`, `--vault` and `--workspace-root` were redeclared on many subcommands. Three format enums disagreed. `--format ndjson` printed pretty JSON. Read commands rejected `--vault`. Enumerated values (`--type`, `--priority`, `--track`) were checked late or not at all.

## Decision

1. Declare the shared flags once, as clap globals on the top-level `Cli` struct.
2. Type them with `ValueEnum` mirrors that live in `aidp-cli`. Each mirror converts to its domain enum with `From`. `aidp-core`, `aidp-tickets` and `aidp-gateway` keep no clap dependency, as the local gate requires.
3. `aidp_core::Format` is the single format type, with `json-compact`, `json-pretty`, `ndjson` and `text`. `aidp_tickets::Fmt` is deleted.
4. One `print_value(format, &Value)` prints every result. `ndjson` prints one line per array element.
5. Keep accepted spellings: `Priority` keeps the aliases `P1`..`P5` and `1`..`5`.
6. `text` stays the default only where the output is Markdown (`run summary`, `run state`, `run reconcile`, `progress`). Gateway commands keep `json-pretty`.

## Consequences

- `--format ndjson` changes on about 18 commands. This is breaking and is listed in `docs/migration-LOC-39.md`.
- Invalid values fail at parse time with clap's usage exit code (2); the migration note lists each one.
- Mirror enums can drift from the domain enums. A round-trip test per enum guards this.
- The snapshots regenerate and are reviewed line by line. `workflow.js`, the run-state schema and the frozen exit codes do not change.
- Help output documents the accepted values, so the contract is self-describing.

## Alternatives rejected

- Derive `ValueEnum` on the domain types: breaks the no-clap rule for library crates.
- Split into several changes: churns snapshots three times and leaves intermediate contracts inconsistent.
