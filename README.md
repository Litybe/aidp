# aidp — le harness de développement de jeu Unity

`aidp` est le binaire Rust qui pilote le développement d'un jeu Unity en solo. Il compile une
pipeline YAML en un script JavaScript **Workflow** déterministe que l'outil `Workflow` de Claude Code
exécute, ou l'exécute lui-même en mode natif (une session `claude -p` par stage). Il écrit les
artefacts des runs dans un vault Obsidian et fait tourner la gateway : sessions, automatisations,
tickets locaux, dispatcher, budget de tokens, flows, worktrees, boîte de PR.

Outils autour du harness : Unity (et son MCP), Aseprite (MCP), Pixellab (MCP), GitHub (issues et PR,
via `gh`) et `aidp`. Le plan d'adaptation au jeu est [`docs/PLAN-GAMEDEV.md`](docs/PLAN-GAMEDEV.md).

Le harness tient en trois dépôts git locaux, tous sur D: :

| Dépôt | Contenu |
|---|---|
| `D:\AIDP\src\aidp` (celui-ci) | le workspace Rust, les schémas JSON, les scripts d'installation, la doc de la gateway |
| `D:\AIDP\skill` | le skill Claude Code : templates, agents, commandes, hooks, `SKILL.md` |
| `D:\AIDP\src\aidp-e2e` | les tests de bout en bout (binaire installé + vrai skill, sans mock) |

Le binaire lit le skill via `AIDP_SKILL_ROOT`, sinon `~/.claude/skills/ai-dev-pipeline-v7`.

## Installation (100 % sur D:)

Rien ne s'installe sur C:. La toolchain (Rust GNU, MinGW portable, cargo-nextest) et les variables
d'environnement utilisateur se posent avec un script relançable :

```
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-gamedev-toolchain.ps1
```

Il pose notamment `CARGO_TARGET_DIR=D:\AIDP\target`, `AIDP_SRC=D:\AIDP\src\aidp`,
`AIDP_INSTALL_DIR=D:\AIDP\bin` et `AIDP_GATEWAY_DIR=D:\AIDP\gateway` (état du daemon). Le vault est
`D:\AIDP\Vault` (`AIDP_VAULT`).

Le bootstrap compile `aidp` et l'installe dans `AIDP_INSTALL_DIR` (`D:\AIDP\bin`), mis en tête du
PATH utilisateur :

```
powershell -NoProfile -ExecutionPolicy Bypass -File bootstrap.ps1
```

Mise à jour : commit ici, bootstrap, puis un nouveau terminal.

## Compatibilité avec le skill

`SKILL.md` déclare `min_aidp_version`. `aidp doctor` compare cette version à la sienne et signale
un binaire trop ancien par le check `skill_compat`.

## Développer

Les règles sont dans [`AGENTS.md`](AGENTS.md), la référence de développement (structure, vérification
locale, boucle de test rapide, niveaux de test, versions) dans [`docs/development.md`](docs/development.md).
Le template `dev-aidp` du skill se lance depuis ce dépôt.

```
cargo check -p <crate>
cargo nextest run -p <crate>
cargo nextest run --workspace -P integration
```

## Exit codes

`aidp` returns one of eight codes, defined once in `aidp_core::ExitStatus` and shared by every subcommand:

| Code | Family | Example command |
|---|---|---|
| 0 | Success | any command that completes normally; also `--help` / `--version` |
| 1 | Internal | an unexpected error, an I/O failure, a schema error |
| 2 | HookWake (reserved) | no command returns it today; the code stays reserved |
| 3 | NotFound | `aidp validate nope`, `aidp run state --run nope`, `aidp ticket show NOPE-9`, `aidp scope check --stage nosuch` |
| 4 | PolicyRefusal | `aidp scope check` on a path outside scope, `aidp knowledge supersede --slug ../evil --by x`, a vault leak caught by `aidp scope verify` / `aidp run reconcile` |
| 5 | UpstreamFailure | a GitHub or gateway upstream error |
| 6 | NegativeVerdict | `aidp doctor` red, `aidp cost anomaly` flags an outlier, `aidp scope verify` finds violations |
| 7 | Usage | any CLI parse fault, e.g. `aidp run state --workspace-rooot x`, `aidp findings merge --out f` with no `--input`, a missing required config variable |

`aidp run reconcile` still exits 0 even when it records scope violations; that is a kept decision, not an oversight.

`aidp --help` refers back to this table by name.
