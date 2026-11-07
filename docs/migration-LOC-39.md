# Migration note: the unified aidp flag contract

This release declares the shared flags once and validates them when clap parses the command line. Most changes loosen what `aidp` accepts. The breaking ones are marked.

## Global flags

`--workspace-root`, `--vault`, `--no-vault`, `--format` and `-v` are global. Put them before or after the verb: `aidp --format json-pretty run state --run X` and `aidp run state --run X --format json-pretty` are the same call. Per-subcommand redeclarations are gone.

`--vault` is now accepted on the read commands that used to reject it:

- `run state`, `run summary`, `run reconcile`, `run verify`, `run record-resume`
- `progress`
- `gate approve`, `gate abort`, `gate pending`
- `scope resolve`, `scope check`, `scope verify`
- `workflow emit`, `workflow plan-status`
- `cost show`, `cost counterfactual`
- `status`

## Formats

One `Format` enum replaces the three that disagreed. Values: `json-compact`, `json-pretty`, `ndjson`, `text`.

| Change | Effect |
|---|---|
| `--format ndjson` is real NDJSON (breaking) | One JSON value per line; an array prints one line per element. About 18 commands printed pretty JSON under this value before. Consumers that parsed the whole output as one document must read line by line. |
| `run state`, `run summary`, `run reconcile`, `progress` accept `json-compact`, `json-pretty`, `ndjson` | `text` stays the default, so hooks and skills that call `aidp run summary` with no flag still get Markdown. |
| `text` on a JSON-only command | Not refused: the output falls back to `json-pretty`. |
| Gateway commands | Default stays `json-pretty`. |

## Values checked at parse time (exit 2)

| Flag | Accepted |
|---|---|
| `ticket new --type` | `story`, `task`, `bug`, `spike` |
| `ticket new --priority`, `edit --priority` | `highest`, `high`, `medium`, `low`, `lowest`; the aliases `P1`..`P5` and `1`..`5` still work |
| `ticket list --status` | comma-separated statuses (`todo,queued`), case-insensitive, kebab-case names such as `in-progress` |
| `ticket list --source` | `jira`, `local` |
| `ticket new --track`, `edit --track`, `start --track` | `quick`, `standard`, `deep` |
| `ticket status <KEY> <STATUS>` | a known status |
| `git outcomes --overlap` | `0.0` to `1.0` |
| `git outcomes --window` | `>= 1` |
| `--input k=v` (`run init`, `run start`) | `k=v` with a non-empty key; unknown keys are still reported by `run init` as warnings |
| `gateway --bind` | an IP address (for example `127.0.0.1`), not a host name |

## Conflicts and removals

- `validate --all` conflicts with `--inputs`. Passing both is a usage error.
- `ticket route`: give either `<KEY> --service <repo>` or `--from-unrouted`, never both.
- `knowledge stale --since` is removed. It had no effect.
- `ticket start --service`, `--template` and `--track` are forwarded to the daemon and applied. They used to be accepted and ignored.
- `gate approve`, `gate abort` and `gate pending` refuse a gate name that no stage declares, before any I/O.
- `run summary` counts: the `failed` key is renamed `cancel`, and a new `in_progress` key is added. The counts sum to `total`.
- `aidp --version` reports the v7 package version.

## Find callers

```bash
grep -rn "aidp " --include=*.md --include=*.json D:/AIDP/skill/
```

Check each hit for `--format ndjson`, a `--priority`/`--type`/`--source`/`--track` value outside the lists above, `--bind` with a host name, `knowledge stale --since`, and a gate name that does not exist.

## Call sites in this repository

No change needed. These use `--format json-compact` or `json-pretty` (unchanged), `--vault`, or no format flag:

- `commands/pipeline-doctor-v7.md`, `pipeline-status-v7.md`, `pipeline-resume-v7.md`: `aidp doctor|status --format json-pretty`
- `commands/pipeline-setup-v7.md`, `pipeline-status-v7.md`, `run-pipeline-v7.md`: `aidp vault status --format json-pretty|json-compact`, `aidp vault init --vault <path>`
- `commands/pipeline-validate-v7.md`: `aidp pipeline list --format json-compact`
- `commands/run-pipeline-v7.md`, `pipeline-resume-v7.md`, `SKILL.md`: `aidp workflow plan-status --run <id> --format json-pretty`, `aidp run init --no-vault`
- `SKILL.md`: `aidp run summary` with no format flag (Markdown, unchanged)
- `hooks/`: `hooks/settings.example.json` calls `aidp run summary`; its arguments are unchanged.

Needs a change: none found. An external script that passes `--format ndjson` and parses one JSON document is the only known breakage.
