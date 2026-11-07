# Lot 7a — Jira local : `aidp ticket` et le dossier `TODO/` du vault

> Plan historique. La synchro Jira (lot 8a), son write-back et le CI autopilot ont été retirés ;
> l'état actuel est décrit dans [`docs/tickets.md`](../../tickets.md).

Règles couvertes : R1, R2, R4 (côté stockage), R8, R9, R12. Voir
[00-vision-et-architecture.md](00-vision-et-architecture.md) §5 et §6 pour le modèle et la machine
à états, non répétés ici.

## Objectif

À la fin du lot, un ticket local se crée, se liste, se lit, se modifie et change de statut
uniquement par `aidp ticket …` ; il est stocké dans `<vault>/TODO/<service>/<KEY>.md` au format
Jira ; le vault porte un tableau de bord `_TODO.md` et un index `_<service>.md` régénérés à chaque
commande ; chaque transition est journalisée. Aucune session, aucun Jira, aucun démon dans ce lot.

## Périmètre

Dans le lot :

- nouveau crate `aidp-tickets` (modèle, store, tri, index, journal, config) ;
- sous-commande `aidp ticket` avec les verbes `new`, `list`, `show`, `edit`, `status`, `route`,
  `cancel`, `log`, `reindex`, `render` ;
- `TODO/config.yaml` (lecture, valeurs par défaut) et `TODO/.state/{counter.json,ticket-events.jsonl}` ;
- lien vers `TODO/_TODO.md` depuis le tableau de bord du vault (`Pipelines/_Pipelines.md`) ;
- documentation : `docs/tickets.md`, encart dans `SKILL.md` §Files et `USER-GUIDE.md`.

Hors lot : synchro Jira (8a), `routing.yaml` complet (8a : ici seulement `repos:` pour déduire le
template d'un service), plan de dispatch (9a), budget (10a), viewer.

## Conception

### Crate `rust/crates/aidp-tickets/`

```
Cargo.toml          deps : serde, serde_json, serde_yaml, anyhow, chrono, clap (pour Op), regex
src/lib.rs          pub mod model, note, store, order, index, ledger, config, cli ; pub use Op
src/model.rs        Ticket, Status, Priority, Source, IssueType, Autopilot, JiraMeta ; transitions légales
src/note.rs         parse(text) -> (Ticket, body) ; render(ticket, body) -> String ; préservation des clés inconnues et de `## Notes locales`
src/store.rs        TicketStore::open(vault_root) ; list/get/save/move_to_service/allocate_key ; écriture atomique tmp+rename
src/order.rs        sort_key(&Ticket) -> (u8, u8, DateTime, String) ; tests d'ordre
src/index.rs        render_board(&[Ticket], &DispatcherView) -> String ; render_service_index ; write_if_changed
src/ledger.rs       Event { ts, key, from, to, actor, reason, session_id } ; append ; read(key, limit)
src/config.rs       Config (défauts du §8 de la vision) ; load(vault_root) tolérant aux clés absentes
src/cli.rs          enum Op (clap) + run(op, vault_root, fmt) ; sortie JSON compacte sur stdout, progrès sur stderr
```

Le crate ne dépend pas de `aidp-core`. Le frontmatter est lu/écrit par `serde_yaml` avec un
`serde_yaml::Mapping` complet pour préserver les clés inconnues ; ne pas réutiliser le parseur
minimal de `aidp-core::knowledge::frontmatter` (il ne connaît que quelques clés).

### Modèle

- `Status` : `Todo | Queued | InProgress | InReview | Done | Blocked | Cancelled`, sérialisé en
  kebab-case (`in-progress`). `Status::can_transition(from, to, actor)` encode la machine à états de
  la vision ; `Human` peut tout faire sauf sortir d'`InProgress` sans `--force`.
- `Priority` : `Highest | High | Medium | Low | Lowest` + `rank()` 1..5, parse tolérant
  (`highest`, `P1`, `1`).
- `Source` : `Jira | Local` avec `rank()` 0/1.
- `Ticket` : les champs du §5.1 ; `autopilot: Autopilot` avec tous les champs `Option`/défaut pour
  que les notes écrites avant un futur champ se relisent (même règle de compatibilité que
  `SessionRecord`).
- `tags` recalculés à chaque `save` : `aidp/ticket`, `aidp/ticket/<status>`, `service/<service>`,
  `source/<source>`.

### Store

- `open(vault_root)` : crée `TODO/`, `TODO/.state/` et un `config.yaml` commenté si absent.
- `list(filter)` : parcourt `TODO/*/` (dossiers non préfixés `_`, `unrouted/` inclus, `.state/`
  exclu, `archive/` exclu sauf `--archived`), parse chaque `*.md` portant `aidp: ticket` ; une note
  illisible est signalée sur stderr et ignorée, jamais fatale.
- `save(ticket, body)` : rend la note, écrit en `tmp` puis `rename`, met à jour `updated`, puis
  `index::refresh(store)` best-effort (comme `run::save → obsidian::refresh`).
- `allocate_key()` : `max(counter.json, max des LOC-n existants) + 1`, écrit le compteur.
- `move_to_service(key, service)` : déplace le fichier, met `service`, réindexe les deux dossiers.
- `archive_sweep()` : déplace `done`/`cancelled` plus vieux que `archive_after_days` dans
  `<service>/archive/` ; appelé par `reindex`.

### Index générés

- `TODO/_TODO.md` : frontmatter `aidp: ticket-board`, puis sections : **Dispatcher** (placeholder
  « inactif » jusqu'au lot 9a, lu depuis `.state/dispatcher.json` s'il existe), **À faire** (tableau
  trié par la clé d'ordre : clé, source, priorité, service, résumé, âge), **En cours**, **En
  revue** (avec lien PR), **Bloqués** (avec raison), **Terminés (7 j)**, un tableau par service avec
  compteurs, et la légende de recherche par tag. Wikilinks vers chaque note.
- `TODO/<service>/_<service>.md` : même chose restreint au service.
- Écriture via un `write_if_changed` local (copie du helper de `obsidian.rs`) pour ne pas faire
  battre Obsidian.
- Le tableau de bord du vault (`obsidian::write_dashboard`) ajoute une ligne
  `[[TODO/_TODO|Tickets]]` si le dossier existe : seule modification dans `aidp-core`.

### CLI

```
aidp ticket new     --service <repo> --summary "<texte>" [--type story|task|bug|spike]
                    [--priority highest|high|medium|low|lowest] [--body-file <md> | --body "<md>" | --stdin]
                    [--labels a,b] [--template <id>] [--track quick|standard|deep] [--epic <KEY>]
                    → crée LOC-n, corps = gabarit create-ticket si aucun corps fourni ; imprime {key, path}
aidp ticket list    [--status s1,s2] [--service X] [--source jira|local] [--archived] [--format text|json-compact|json-pretty]
                    → triés dans l'ordre de dépilage ; `text` = une ligne par ticket
aidp ticket show    <KEY> [--format]           → frontmatter + corps ; json = Ticket + body
aidp ticket render  <KEY>                      → le corps rendu tel qu'il sera passé en `prompt` (lot 9a) ; utile pour vérifier
aidp ticket edit    <KEY> [--priority] [--service] [--template] [--track] [--summary] [--labels] [--epic]
                    → frontmatter seulement ; `--service` délègue à `route`
aidp ticket status  <KEY> <status> [--reason "<texte>"] [--force]
                    → transition manuelle, actor=human, journalisée ; refus explicite si illégale
aidp ticket route   <KEY> --service <repo> [--template <id>]
aidp ticket cancel  <KEY> [--reason]
aidp ticket log     [<KEY>] [--limit 50] [--format]   → lecture du journal
aidp ticket reindex                            → régénère _TODO.md et tous les _<service>.md, archive
```

Sortie : un objet JSON compact sur stdout (règle du dépôt, `emit_json`), erreurs `{"error": …}`
et code de sortie non nul. `--vault` et `AIDP_VAULT` suivent la précédence de `aidp-core::vault`.

### Template par service

`TODO/routing.yaml` est introduit ici avec sa seule section `repos:` :

```yaml
repos:
  cheat-alerts-service:  { template: dev-csharp }
  cheat-data-alert:      { template: dev-lambda-ts }
  aidp-viewer:           { template: dev-aidp-viewer }
  ai-dev-pipeline-v7:    { template: dev-aidp }
```

`new` et `route` résolvent `template` depuis cette table quand il n'est pas passé ; sans entrée, le
ticket est créé avec `template: null` et `list` le signale (le lot 9a refusera de le lancer).
La section `rules:` (routage des tickets Jira) arrive au lot 8a.

## Étapes

1. Créer le crate, l'ajouter aux `members` du workspace, `cargo build`.
2. `model.rs` + `note.rs` : round-trip parse/render, tests sur une note complète, une note
   minimale, une note avec clés inconnues et une section `## Notes locales`.
3. `store.rs` : ouverture, liste, sauvegarde atomique, allocation de clé, déplacement, archivage.
4. `order.rs` : clé de tri ; tests : jira avant local, priorité, FIFO, stabilité.
5. `ledger.rs` + `Status::can_transition` ; tests des transitions légales/illégales.
6. `config.rs` : défauts + surcharge partielle ; test « fichier absent = défauts ».
7. `index.rs` : rendu du board et des index de service ; test d'idempotence (`write_if_changed`).
8. `cli.rs` + branchement dans `aidp-cli/src/main.rs` (`Cmd::Ticket { op: aidp_tickets::Op }`),
   résolution du vault comme `aidp vault status`.
9. Ligne `Tickets` dans `obsidian::write_dashboard`.
10. `docs/tickets.md` (format, verbes, exemples), encarts `SKILL.md` / `USER-GUIDE.md` / `AGENTS.md`
    (un agent ne doit **jamais** éditer un statut de ticket : seul `aidp` le fait).
11. `cargo test --workspace`, `cargo fmt --all -- --check`, `aidp doctor`.

## Tests

- Inline `#[cfg(test)]` par module (convention du dépôt, pas de `tests/`), sur un vault temporaire
  (`tempfile` déjà présent ? sinon `std::env::temp_dir()` + uuid).
- Cas à couvrir : round-trip de note ; préservation des clés inconnues ; clé locale après
  suppression du compteur ; ordre de dépilage ; transitions ; index idempotent ; `list` ignore
  `.state/`, `archive/` et les fichiers `_*.md` ; `status --force` requis depuis `in-progress`.

## Critères d'acceptation

- `aidp ticket new --service aidp-viewer --summary "Page Tickets"` crée `TODO/aidp-viewer/LOC-1.md`
  lisible dans Obsidian, avec frontmatter et gabarit Jira, et `TODO/_TODO.md` le liste.
- `aidp ticket list --format text` affiche les tickets dans l'ordre de dépilage.
- `aidp ticket status LOC-1 blocked --reason "attente specs"` journalise et met à jour les index.
- Une note éditée à la main (corps) puis `aidp ticket edit LOC-1 --priority high` conserve le corps
  et les clés inconnues à l'octet près hors frontmatter.
- Aucun fichier `_*.md` n'est écrit ailleurs que par `index.rs`.

## Risques et points d'attention

- **Nom de service ≠ nom de projet du démon** : refuser un `--service` qui ne correspond à aucun
  dossier `Pipelines/<repo>/` ni `Knowledge/<repo>/` du vault sauf `--allow-unknown-service`.
- **Encodage** : notes en UTF-8 sans BOM, fins de ligne `\n` ; Windows ne doit pas introduire de
  `\r\n` (écriture binaire).
- **Concurrence CLI / démon** (à partir du lot 9a) : écriture atomique dès maintenant ; jamais de
  cache d'état dans le crate.

## Dépendances et taille

Aucune dépendance sur un autre lot. Taille **M** : un run `dev-aidp` en `track: deep`.
