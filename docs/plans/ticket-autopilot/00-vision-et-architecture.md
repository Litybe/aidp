# Vision et architecture — tickets locaux + dépilage automatique des tickets

> Plan historique. La synchro Jira (lot 8a), son write-back et le CI autopilot ont été retirés ;
> l'état actuel est décrit dans [`docs/tickets.md`](../../tickets.md).

## 1. Le besoin, en règles

Chaque règle est numérotée pour être tracée dans les plans de lots.

| # | Règle | Lot |
|---|---|---|
| R1 | Un système de tickets local pour les todo, au format Objectif / Impacts / Critères | 7a |
| R2 | Les tickets sont stockés dans le vault, dossier `TODO/`, **un sous-dossier par service** | 7a |
| R4 | Récupérer les tickets locaux en statut *todo* | 7a, 9a |
| R5 | Priorité : la priorité portée par le ticket | 9a |
| R6 | Dépiler **au plus 5 projets en parallèle** | 9a |
| R7 | Un ticket est fini quand il est **en PR** ; le slot libéré démarre le ticket suivant | 9a |
| R8 | Gérer le **statut** des tickets pour suivre leur avancement | 7a, 9a |
| R9 | Toutes les actions (lecture, gestion) passent par le **CLI `aidp`** | 7a, 9a |
| R10 | Le lancement de session passe par `aidp`, qui pilote le **CLI `claude`** | 9a (existant : démon) |
| R11 | `aidp` orchestre les sessions et leurs états, crée les nouvelles sessions | 9a |
| R12 | **Toutes les données** dont `aidp` a besoin sont dans le **vault** | 7a → 10a, 11 |
| R13 | Les agents IA ne font qu'**exécuter un run `aidp`** avec le ticket à faire | 9a (existant : prompt `/run-pipeline-v7`) |
| R14 | Lancer une session **à la main**, qui suit ensuite le déroulement classique (tickets prioritaires) | 9a |
| R15 | Surveiller le **budget de tokens** restant, ne pas lancer de développement quand il n'en reste presque plus, **attendre le reset** | 10a |

## 2. Principes

1. **`aidp` décide, l'agent obéit.** Le choix du prochain ticket, du template, des inputs, du
   moment de lancement et de la transition de statut est une commande déterministe, jamais un
   jugement de session Claude. C'est le principe fondateur du skill (`SKILL.md`, `AGENTS.md`).
2. **Le vault est la vérité.** Tickets, configuration du dépileur, table de routage, état du
   dispatcher, journal des transitions et statistiques de budget vivent sous `<vault>/TODO/`.
   Le démon **relit** le vault à chaque tick ; il ne garde pas de copie faisant foi en mémoire.
3. **Le CLI est la seule surface.** Chaque fonctionnalité existe d'abord comme verbe `aidp ticket …`
   ou `aidp gateway dispatcher …`. Les routes HTTP du démon et le viewer ne font que l'exposer.
4. **Abonnement Max, jamais l'API.** Décision structurante déjà en place dans le démon (clés API
   retirées de l'environnement des sessions). La ressource rare est l'utilisation des fenêtres
   5 h / 7 j : le dispatcher la traite comme un budget.
5. **Jamais de fichier `_*.md` écrit à la main.** Les index `_TODO.md` et `_<service>.md` sont
   générés par `aidp` et régénérés à chaque commande qui touche un ticket.
6. **Pas de deuxième file.** Le dispatcher ne dépose jamais un lancement dans la `queue.json`
   par projet du démon ; il attend qu'un projet soit libre avant d'appeler `start()`. Une seule
   source d'ordre : l'état des tickets dans le vault.

## 3. Composants

```
                  ┌────────────────────────────── vault D:/AIDP/Vault ──────────────────────────────┐
                  │  TODO/config.yaml   TODO/routing.yaml   TODO/<service>/<KEY>.md   TODO/.state/  │
                  └───────▲──────────────────────────▲──────────────────────────▲───────────────────┘
                          │ lit/écrit                │ lit/écrit                │ lit/écrit
   humain ──► aidp ticket … ──────────────► crate aidp-tickets ◄────────────── aidp gateway serve
              (CLI, un process)             (modèle, store, tri,               (démon résident)
                                             plan pur, budget pur)                │
                                                                                  │ Dispatcher (Lot 9a)
                                                                                  │ tick 60 s + bus
                                                                     SessionManager::start()  (existant)
                                                                                  │
                                                                     claude -p "/run-pipeline-v7 <template> <KEY>"
                                                                                  │
   GitHub ◄── gh (PR)  ◄──────────────────────── run v7 dans le vault ◄───────────┘
```

| Composant | Rôle | Où |
|---|---|---|
| `aidp-tickets` (nouveau crate) | modèle de ticket, lecture/écriture des notes, tri, routage, plan de dispatch (pur), verdict de budget (pur), index générés | `rust/crates/aidp-tickets/` |
| `aidp ticket …` | verbes CLI ; résout le vault via `aidp-core::vault`, appelle le crate | `rust/crates/aidp-cli/src/main.rs` |
| `Dispatcher` (nouveau module du démon) | boucle de tick, abonnement au bus, exécution du plan via `SessionManager::start`, cycle de vie ticket ↔ session | `rust/crates/aidp-gateway/src/dispatcher.rs` |
| routes HTTP `/api/tickets`, `/api/dispatcher` | exposition pour le viewer et les verbes `aidp gateway …` | `http.rs`, `client.rs`, `lib.rs` |
| `aidp run start --approve-gate` | approbation pré-enregistrée d'une gate, pour qu'une session autonome ne s'arrête pas sur la gate `pr` | `aidp-core/src/run.rs`, `aidp-cli` |

**Dépendances de crates.** `aidp-tickets` ne dépend ni de `aidp-core` ni de `aidp-gateway` : il
reçoit le chemin du vault (`TicketStore::open(vault_root)`). Le CLI le résout avec
`aidp_core::vault::resolve` ; le démon le lit comme il le fait déjà pour la découverte des projets
(`~/.aidp/config.json`, clé `vault`). Le démon gagne une dépendance vers `aidp-tickets`, ce qui ne
rompt pas le précédent « pas de types de domaine `aidp-core` dans la passerelle ».

## 4. Ce qui existe et est réutilisé

| Existant | Réemploi |
|---|---|
| `SessionManager::start()` et la règle **une session par projet** (mutex d'admission) | seul point de lancement ; le dispatcher n'ajoute pas de spawn |
| `StartRequest.origin` | `{ kind: "dispatcher", ref: "<KEY>", label: "<summary>" }` : lien session → ticket sans nouveau champ |
| `RateLimitInfo` (5 h / 7 j, `resetsAt`), `rate-limit.jsonl`, `utilization_block` | entrée du verdict de budget (Lot 10a) |
| `GET /api/sessions/:id/run` (appariement `workspace_root` + `started_at`) | retrouver le run vault d'une session pour lire `pr/pr.json` |
| `changes::pr(cwd)` (`gh pr view`) | détection de PR de repli quand `pr.json` manque |
| `store.rs` (`write_json` atomique, `append_jsonl`) et `obsidian::write_if_changed` | même hygiène d'écriture pour `.state/` et les index |
| templates `dev-*` en mode `prompt` + `feature` | un ticket local part en `prompt` |
| prompt `/run-pipeline-<gen> <template> <target>` + `AIDP_SKILL_ROOT` injecté | inchangé : l'agent n'exécute qu'un run (R13) |

## 5. Modèle de données dans le vault

```
<vault>/TODO/
  _TODO.md                     généré : tableau de bord (par statut, par service, dispatcher, budget)
  config.yaml                  édité : slots, seuils, prefix local
  routing.yaml                 édité : ticket → service, service → template/track
  unrouted/                    tickets dont le service n'a pas pu être déduit
    LOC-14.md
  pixelhood/                   un dossier par service = nom du dossier repo du vault = nom du projet du démon
    _pixelhood.md              généré : index du service
    LOC-12.md                  ticket local (source: local)
  aidp-viewer/
    LOC-13.md
  .state/                      caché pour Obsidian, machine seulement
    counter.json               prochain numéro local
    dispatcher.json            état courant : slots occupés, mode (normal/waiting/paused), wait_until, dernier tick
    budget-stats.json          Δ d'utilisation observés par template (apprentissage, Lot 10a)
    ticket-events.jsonl        journal append-only des transitions (qui, quand, pourquoi)
```

Le nom du service est le **pivot** : il doit être identique au dossier `Pipelines/<repo>/` du vault,
au dossier de la copie de travail et au `name` du projet enregistré dans le démon. C'est déjà la
convention du viewer (`path.basename(projectPath)`) et de la découverte de projets.

### 5.1 Note de ticket

Frontmatter machine (réécrit par `aidp`, clés inconnues préservées), corps humain avec une section
`## Notes locales` toujours préservée.

```markdown
---
aidp: ticket
key: LOC-12
source: local
type: Story                  # Story | Task | Bug | Spike
summary: "Ajouter le dash au contrôleur du joueur"
status: todo                 # voir §6
priority: High               # Highest | High | Medium | Low | Lowest
priority_rank: 2             # 1..5, dérivé
service: pixelhood
template: dev-unity          # résolu par routing.yaml, surchargeable
track: standard
labels: [player]
components: []
epic: LOC-10
created: 2026-09-10T08:12:00Z
updated: 2026-09-10T09:40:00Z
autopilot:
  attempts: 0
  queued_at: null
  started_at: null
  session_id: null
  run_id: null
  branch: null
  pr: null                   # { url, number, state }
  blocked_reason: null
tags: [aidp/ticket, aidp/ticket/todo, service/pixelhood, source/local]
---
# LOC-12 — Ajouter le dash au contrôleur du joueur

## 🎯 Objectif
…
## 🔧 Impacts techniques
…
## ✅ Critères d'acceptation
- …

## Notes locales
(jamais modifié par aidp)
```

Le gabarit du corps (Objectif, Impacts techniques, Critères d'acceptation) est compris tel quel par
`discover_prompt`.

### 5.2 Clés

- Local : `<prefix>-<n>`, `prefix` = `local.key_prefix` (défaut `LOC`), `n` = compteur global
  `.state/counter.json`, contrôlé par `max(clés existantes)` à chaque allocation pour survivre à une
  suppression du fichier d'état.

### 5.3 Ordre de dépilage

Clé de tri, dans l'ordre : `source_rank` (un ancien miroir Jira avant un ticket local) → `priority_rank` croissant →
`created` croissant → `key`. Les égalités sont donc FIFO. Un `aidp ticket start` manuel place le
ticket **devant tout le monde** (`autopilot.manual_priority: true`).

## 6. Machine à états d'un ticket

```
                    new
                    │
                    ▼
   ┌────────────► todo ──────────────┐ plan() le retient
   │                                 ▼
   │  retry (attempts < max)      queued  ── budget/slot/projet libre ──► in-progress
   │                                                                          │
   │                          session.ended                                   │
   │      ┌──────────────────────────┼──────────────────────────┐             │
   │      ▼                          ▼                          ▼             │
   │   failed (attempt++)      done + PR détectée        done sans PR / question
   │      │                          │                          │
   └──────┘                          ▼                          ▼
   attempts ≥ max ──► blocked    in-review ──(PR mergée)──► done       blocked (gate / no-pr)
                                     │
                                (PR fermée non mergée) ──► blocked

   cancelled : à tout moment par `aidp ticket cancel`
```

| Statut | Signification | Qui écrit |
|---|---|---|
| `todo` | à faire, candidat au dépilage | `new`, retry |
| `queued` | retenu par le plan, en attente d'un slot, d'un projet libre ou du budget | dispatcher |
| `in-progress` | une session tourne (`autopilot.session_id`) | dispatcher |
| `in-review` | la session a fini et une PR existe : **fini pour l'autopilote** (R7), slot libéré | dispatcher |
| `done` | PR mergée (sonde `gh pr view`) | dispatcher |
| `blocked` | besoin d'un humain : `blocked_reason` ∈ `needs-routing`, `failed`, `gate:<stage>`, `no-pr`, `pr-closed`, `cancelled-by-user` | dispatcher |
| `cancelled` | retiré | humain |

Invariants :

- Seul `aidp` transitionne (CLI ou démon). Une session Claude n'écrit jamais un statut.
- Toute transition est journalisée dans `.state/ticket-events.jsonl` :
  `{ts, key, from, to, actor: cli|dispatcher|human|flow, reason, session_id?}`.
- Une transition manuelle (`aidp ticket status`) sur un ticket `in-progress` est refusée sans
  `--force` ; `--force` annule d'abord la session via le démon.
- `in-review` libère le slot ; `done` n'est qu'informatif.

## 7. Séquences

### 7.1 Tick du dispatcher (toutes les 60 s et sur événement)

1. Si `dispatcher.paused` → rien. Si `mode = waiting` et `now < wait_until` → rien.
2. Relire l'état des tickets et des sessions : sessions `running` d'origine `dispatcher` = slots
   occupés ; projets occupés = tout projet ayant une session `running` quelle qu'en soit l'origine.
3. `budget::verdict(rate_limit, slots_occupés, cfg)` (Lot 10a). Si refus → `mode = waiting`,
   `wait_until = resetsAt` de la fenêtre bloquante, événement `dispatcher.waiting`, fin du tick.
4. `plan::plan(candidats triés, slots libres, projets occupés)` → `launches[]`, `skipped[]`
   avec raison (`project-busy`, `no-slot`, `needs-routing`, `budget`).
5. Pour chaque `launch` : ticket → `queued` puis `StartRequest` → `start()`. Si la réponse est
   `queued` (course perdue), annuler l'entrée de file et remettre le ticket en `todo`. Sinon
   ticket → `in-progress` avec `session_id`, événement `ticket.started`.

### 7.2 Fin de session (`session.ended` d'une session d'origine `dispatcher`)

1. Retrouver le ticket par `origin.ref`.
2. Chercher la PR : run vault apparié → `<run_dir>/pr/pr.json` ; sinon `changes::pr(cwd)` sur la
   branche `feat/<KEY>`.
3. `status == done` et PR → `in-review` (`autopilot.pr` renseigné) ; événement `ticket.reviewed`.
4. `status == done` sans PR → `blocked: no-pr`.
5. `status == failed` : si le dernier texte ou le code de sortie indique une limite d'usage →
   `todo` sans incrémenter `attempts` et `mode = waiting` ; sinon `attempts++`, puis `todo` si
   `attempts < max_attempts`, `blocked: failed` sinon.
6. `status == cancelled` → `blocked: cancelled-by-user`. `orphaned` (redémarrage du démon) →
   étape 2 puis `todo` si pas de PR.
7. Déclencher un tick immédiat (le slot est libre).

### 7.3 Lancement manuel (`aidp ticket start <KEY>`)

1. Routage (`--service`/`--template` explicites sinon `routing.yaml`) ; sans service → erreur
   nommant `aidp ticket route`.
2. `autopilot.manual_priority = true`, statut `queued`, puis `POST /api/dispatcher/tick`.
   `--force` ignore le budget et le plafond de slots, jamais la règle une session par projet.
3. Le ticket suit ensuite 7.2 comme les autres (R14).

### 7.4 Attente du reset (R15)

1. Verdict refusé sur la fenêtre 5 h (ou 7 j) → `mode = waiting`, `wait_until = resetsAt + 60 s`.
2. Les sessions déjà en cours continuent ; aucun nouveau lancement, même si un slot se libère.
3. À `wait_until` le tick reprend ; si un instantané plus récent montre encore un dépassement, on
   recalcule `wait_until`. Événement `dispatcher.resumed` sinon.
4. `_TODO.md` affiche « En attente du reset (fenêtre 5 h) jusqu'à 14:35 » ; `aidp ticket budget`
   explique le verdict.

## 8. Configuration

`<vault>/TODO/config.yaml` (non secret, édité à la main, lu à chaque tick) :

```yaml
dispatcher:
  slots: 5
  tick_seconds: 60
  permission_mode: acceptEdits
  auto_approve_gates: [pr]       # "*" pour toutes ; les autres gates bloquent le ticket (gate:<stage>)
  max_attempts: 2
  gate_timeout_hours: 12
  paused: false
budget:
  max_utilization_5h: 0.80
  max_utilization_7d: 0.90
  reserve_per_run_5h: 0.10       # coût estimé d'un run dev sur la fenêtre 5 h
  reserve_per_run_7d: 0.02
  unknown_policy: conservative   # aucun instantané : une seule session à la fois
  learn: true                    # affiner la réserve avec les Δ observés
local:
  key_prefix: LOC
  archive_after_days: 30         # done/cancelled déplacés dans <service>/archive/
```

Aucun secret dans le vault. `~/.aidp/config.json` ne fait que pointer le vault.

## 9. Décisions par défaut, à confirmer

| # | Décision | Alternative écartée |
|---|---|---|
| D1 | Clé locale `LOC-<n>` avec un compteur unique | un préfixe par service (routage inutile pour un local) |
| D3 | La gate `pr` est approuvée d'avance pour les sessions du dispatcher ; les autres gates bloquent le ticket | répondre à la question de gate par motif texte depuis le démon : fragile |
| D4 | Le plafond de 5 compte les sessions **du dispatcher** ; une session UI occupe son projet mais pas un slot | compter toute session : une conversation bloquerait le dépilage |
| D5 | Un ticket local est fourni au pipeline en mode `prompt` (corps rendu verbatim, `feature = <KEY>`) | un stage `discover_ticket` dédié : plus propre, plus de templates à toucher ; gardé en suite |
| D6 | `in-review` (PR ouverte) libère le slot ; `done` à la merge | attendre la merge : gaspille les slots |
| D7 | Seuils 0,80 (5 h) / 0,90 (7 j), réserve 0,10 par run, 5 slots, tick 60 s, 2 tentatives, gate 12 h | — |
| D8 | Les données du système de tickets sont 100 % dans le vault ; l'état préexistant du démon (`~/.aidp/gateway/`) y reste sauf Lot 11 | tout déplacer d'emblée : migration à part |
| D9 | Le dispatcher n'utilise pas la `queue.json` du démon | s'y appuyer : deux ordres concurrents |

## 10. Hors périmètre

- Aucune messagerie : notifications via les webhooks génériques existants.
- Pas de multi-utilisateur : un vault, un abonnement, un poste.
- Pas de résolution automatique des gates autres que celles de `auto_approve_gates`.

## 11. Glossaire

- **Ticket** : une note `TODO/<service>/<KEY>.md`.
- **Service** : le pivot nom-de-repo (vault, copie de travail, projet du démon).
- **Slot** : une session `running` d'origine `dispatcher`. **Plafond** : `dispatcher.slots`.
- **Plan** : sortie pure de `plan()` : `launches[]` + `skipped[]`.
- **Verdict** : sortie pure de `budget::verdict()` : autoriser, ou attendre jusqu'à `wait_until`.
- **Fenêtre** : fenêtre glissante 5 h ou 7 j de l'abonnement, telle que remontée par
  `rate_limit_event`.
