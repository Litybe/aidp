# Lot 9a — Dispatcher : des tickets aux sessions, 5 slots, cycle de vie complet

> Plan historique. La synchro Jira (lot 8a), son write-back et le CI autopilot ont été retirés ;
> l'état actuel est décrit dans [`docs/tickets.md`](../../tickets.md).

Règles couvertes : R4, R5, R6, R7, R8, R9, R10, R11, R13, R14. Dépend des lots 7a et 8a. Le
budget (R15) est branché ici sous forme de stub et remplacé au lot 10a.

## Objectif

Le démon `aidp gateway serve` dépile les tickets `todo` dans l'ordre de la vision (§5.3), lance au
plus cinq sessions de développement en parallèle sur des projets distincts, suit chaque ticket de
`queued` à `in-review` (PR ouverte) puis `done` (PR mergée), remet un ticket échoué en file ou le
bloque, et redémarre un ticket dès qu'un slot se libère. Un humain peut pousser un ticket devant
tout le monde avec `aidp ticket start`. Les sessions ne font qu'exécuter `/run-pipeline-v7
<template> <KEY>` ; la gate `pr` est approuvée d'avance pour qu'une session autonome aille jusqu'à
la PR.

## Périmètre

Dans le lot :

- `aidp_tickets::plan` (décision pure) et `aidp_tickets::lifecycle` (transitions sur événements de
  session) ;
- module `dispatcher.rs` du démon : boucle de tick, abonnement au bus, exécution du plan via
  `SessionManager::start`, état `.state/dispatcher.json` ;
- `StartRequest` : champ `autoApproveGates` (env `AIDP_AUTO_APPROVE_GATES` sur le process) et
  `inputsFile` ; origine `{ kind: "dispatcher", ref: <KEY> }` ;
- `aidp run start --approve-gate <stage>` (répétable, `*` accepté) et `--inputs-file <json>` dans
  `aidp-core`/`aidp-cli`, avec repli sur `AIDP_AUTO_APPROVE_GATES` ;
- détection de PR : `pr/pr.json` du run apparié, sinon `gh pr view` par nom de branche ;
- sonde de merge pour les tickets `in-review` ;
- routes HTTP `/api/tickets*`, `/api/dispatcher*` ; verbes `aidp ticket start|plan|dispatch`,
  `aidp gateway dispatcher status|pause|resume|tick` ;
- templates : mode `prompt` ajouté à `dev-lambda-ts.yaml` et `dev-angular.yaml` (double demande
  comme `dev-csharp.yaml`) ;
- docs : `docs/gateway-api.md` (nouvelles sections), `SKILL.md`/`AGENTS.md` (gates pré-approuvées,
  `--inputs-file`), `docs/tickets.md` (cycle de vie), et une ligne dans `docs/ORCHESTRATOR.md` du
  viewer (état du chantier).

Hors lot : verdict de budget réel (10a), viewer (7b/9b).

## Conception

### `plan.rs` — la décision, pure

```rust
pub struct RunningSlot { pub project: String, pub origin_kind: String, pub ticket_key: Option<String>, pub started_at_ms: i64 }
pub enum Demand { Jira { keys: Vec<String> }, Prompt { prompt: String, feature: String } }
pub struct Launch { pub key: String, pub service: String, pub template: String, pub track: String, pub demand: Demand, pub manual: bool }
pub enum SkipReason { ProjectBusy, NoSlot, NeedsRouting, NoTemplate, TemplateWithoutDemandMode, Budget(String), Paused }
pub struct DispatchPlan { pub launches: Vec<Launch>, pub skipped: Vec<(String, SkipReason)>, pub slots_used: usize, pub slots_max: usize }

pub fn plan(candidates: &[Ticket], running: &[RunningSlot], budget: &Verdict, templates: &TemplateCatalog, cfg: &DispatcherCfg) -> DispatchPlan
```

Règles, dans l'ordre :

1. `cfg.paused` → tout est `Paused`.
2. `slots_used` = sessions `running` d'origine `dispatcher` ; `free = slots − slots_used`, borné
   par `budget.max_launches()` (stub 9a : `usize::MAX` si `utilization_block` est nul, sinon `0`).
3. Candidats = tickets `todo` et `queued` non bloqués, triés : `manual_priority` d'abord, puis
   la clé d'ordre de la vision.
4. Pour chaque candidat : service sans projet enregistré ou `unrouted/` → `NeedsRouting` ;
   `template` nul → `NoTemplate` ; projet occupé par **n'importe quelle** session `running` ou déjà
   présent dans `launches` → `ProjectBusy` ; `free == 0` → `NoSlot` ; sinon `Launch`, `free −= 1`.
5. Demande : si le template déclare `jira_keys` et le ticket est un miroir → `Demand::Jira`,
   sinon `Demand::Prompt { prompt: render(ticket), feature: key }`. Un template sans `jira_keys`
   ni `prompt` → `TemplateWithoutDemandMode`.

`TemplateCatalog` est une vue minimale (`id → inputs déclarés`) construite depuis
`aidp pipeline list/show` par le CLI et depuis le même code par le démon (lecture des YAML du
`AIDP_SKILL_ROOT`, sans `aidp-core` : un parse `serde_yaml` de la clé `inputs`).

### `lifecycle.rs` — les transitions sur événements

```rust
pub enum SessionOutcome { Done, Failed { rate_limited: bool }, Cancelled, Orphaned }
pub struct PrInfo { url, number, state }
pub fn on_started(t: &mut Ticket, session_id, run_id: Option<String>, branch) -> Event
pub fn on_ended(t: &mut Ticket, outcome, pr: Option<PrInfo>, cfg) -> Event        // §7.2 de la vision
pub fn on_gate(t: &mut Ticket, stage) -> Event                                   // blocked: gate:<stage>, session conservée
pub fn on_gate_answered(t: &mut Ticket) -> Event                                 // retour in-progress
pub fn on_pr_state(t: &mut Ticket, state: Merged | Closed | Open) -> Option<Event>
```

Chaque fonction renvoie l'événement à journaliser ; l'écriture (note + ledger + bus) est faite par
l'appelant. Tests table-driven.

### `dispatcher.rs` — le démon

```rust
pub struct Dispatcher { store_root: PathBuf, sessions: Arc<SessionManager>, projects: Arc<Projects>, bus: Arc<Bus>, state: Mutex<DispatcherState>, skill_root: PathBuf }
impl Dispatcher {
  pub fn spawn(self: Arc<Self>)            // thread : tick toutes les cfg.tick_seconds, réveil sur bus
  pub fn tick(&self, reason: &str) -> TickReport
  fn launch(&self, l: &Launch) -> anyhow::Result<String /* session id */>
  fn on_session_event(&self, ev: &BusEvent)   // session.ended, session.question, session.answered
  fn probe_reviews(&self)                     // gh pr view pour les in-review, toutes les 10 min
}
```

- **Tick** (§7.1 de la vision) : relit `config.yaml`, `routing.yaml`, le store ; construit `running` depuis `sessions.list(Some("running"))` ; calcule le
  verdict (stub) ; `plan()` ; exécute chaque `Launch` ; écrit `.state/dispatcher.json` ; émet
  `dispatcher.tick`. Un tick ne dure jamais plus que le temps des `start()` : aucune attente.
- **Lancement** : ticket → `queued` (note + ledger), puis `StartRequest` :

  ```json
  { "projectId": "<id du projet dont name == service>", "engine": "v7", "templateId": "dev-csharp",
    "target": "PIXEL-5677", "inputsFile": "<vault>/TODO/.state/launch/PIXEL-5677.inputs.json",
    "permissionMode": "acceptEdits", "autoClose": true,
    "origin": { "kind": "dispatcher", "ref": "PIXEL-5677", "label": "<summary>" },
    "autoApproveGates": ["pr"] }
  ```

  Le fichier d'inputs est écrit par le dispatcher juste avant : `{"jira_keys": ["PIXEL-5677"],
  "track": "standard"}` ou `{"prompt": "<corps rendu>", "feature": "LOC-12", "track": "standard"}`,
  en **JSON échappé ASCII** (`\uXXXX` pour tout non-ASCII : un corps de ticket accentué passé en
  ligne de commande a déjà fait refuser un Workflow par mojibake). Le prompt de la session devient
  `/run-pipeline-v7 <template> <KEY>` suivi de « inputs décidés dans `<fichier>` : passe
  `--inputs-file <fichier>` à `aidp run start` ».

  Si `start()` renvoie une entrée de file (`status == "queued"`, course perdue contre un autre
  lanceur), le dispatcher annule immédiatement cette entrée (`queue.cancel`) et remet le ticket en
  `todo` (D9). Sinon ticket → `in-progress` avec `session_id`, `branch: feat/<KEY>`, événement
  `ticket.started`.
- **Fin de session** (§7.2) : `origin.kind == dispatcher` → ticket par `origin.ref` → PR, trois
  sources dans l'ordre : `runs::for_session(id, cwd, started_at)` / `runs::resolve_run` donnent le
  `run_dir` → `<run_dir>/pr/pr.json` (aucun schéma n'existe pour ce fichier : tolérer `url`,
  `html_url`, `pr_url`, `number`) ; sinon `outputs.pr_url` de l'enveloppe du stage `pr` dans
  `run-state.json` (forme documentée dans `AGENTS.md` §outputs) ; sinon
  `changes::pr_for_branch(cwd, "feat/<KEY>")`, extension de `changes::pr` passant le nom de
  branche à `gh pr view` (jamais appelée sur un chemin par tour, même règle que `pr`). Puis
  `lifecycle::on_ended`. `rate_limited` est calculé par le lot 10a (stub : `false`). Tick immédiat
  ensuite.
- **Gate non approuvée** : `session.question` sur une session du dispatcher → `on_gate(stage)`.
  Le nom de stage vient de `run-state.json` (`pending_gate`, déjà lu par
  `runs::run_is_gate_blocked(run_dir)`), avec repli sur le texte de la question puis `gate:?`. La
  session reste `running` et occupe son slot ; `session.answered` → `on_gate_answered`. Au-delà de
  `gate_timeout_hours`, le dispatcher annule la session et bloque le ticket (`gate-timeout`).
- **Redémarrage du démon** : les sessions passent `orphaned` ; au premier tick, tout ticket
  `in-progress` dont la session n'est plus `running` passe par `on_ended(Orphaned)` (PR cherchée
  d'abord).
- **Sonde de merge** : pour chaque `in-review`, `gh pr view <number> --json state,mergedAt` dans le
  cwd du projet, toutes les 10 minutes ; `MERGED` → `done`, `CLOSED` → `blocked: pr-closed`.
- **État** `.state/dispatcher.json` :
  `{ mode: normal|waiting|paused, paused, waitUntil, lastTickAt, lastReason, slots: {used, max},
  running: [{key, sessionId, project, startedAt}], lastPlan: {launched: [], skipped: [{key, reason}]} }`.
  Relu par `aidp ticket list`/`_TODO.md` pour la section Dispatcher.

### `SessionManager` et `aidp-core`

- `StartRequest` gagne `auto_approve_gates: Option<Vec<String>>` et `inputs_file: Option<String>`
  (serde default). Au spawn : `AIDP_AUTO_APPROVE_GATES=<liste jointe par virgule>` dans l'env du
  process ; `build_prompt` ajoute la phrase sur `--inputs-file`. `SessionRecord` ne gagne aucun
  champ : le lien vers le ticket est `origin.ref`.
- `aidp run start` : `--approve-gate <stage>` répétable ; sans flag, lit `AIDP_AUTO_APPROVE_GATES`.
  Après `run init`, pour chaque gate listée existant dans la pipeline effective, appelle
  `record_resume(approved_gate)` **avant** le premier `workflow emit`, qui la cuit dans
  `__approved`. `*` = toutes les gates. Une gate inconnue = `warning:` sur stderr, pas une erreur.
  La sortie JSON gagne `auto_approved_gates: [...]` et `gates` n'en liste plus que les restantes.
- `aidp run start --inputs-file <path>` : `run start` accepte déjà `--inputs <json>` (chaîne) et
  `--input k=v` (`aidp-cli/src/main.rs`, `RunOp::Start`). `--inputs-file` lit le fichier et
  l'injecte dans le même chemin que `--inputs` ; le seul ajout est la lecture du fichier et son
  test avec un contenu accentué échappé ASCII.

### Templates

`dev-lambda-ts.yaml` et `dev-angular.yaml` : `jira_keys` devient `required: false, default: []`,
ajout de `prompt` et `feature` (mêmes commentaires que `dev-csharp.yaml`), transmission au
sous-pipeline `dev-discovery` (`with: { jira_keys, prompt, track }`). `aidp validate --all` doit
rester vert. Les templates `dev-aidp` et `dev-aidp-viewer` restent en mode prompt : un miroir Jira
qui les cible part en `Demand::Prompt` (rendu du miroir), ce que la règle 5 du plan couvre.

### HTTP et CLI

| Route | Rôle |
|---|---|
| `GET /api/tickets?status=&service=&source=` | tableau nu de tickets (frontmatter public, sans corps), ordre de dépilage |
| `GET /api/tickets/:key` | `{ ticket, body }` |
| `POST /api/tickets` | création locale (corps de `aidp ticket new`) |
| `POST /api/tickets/:key/start` `{force?}` | lancement manuel (§7.3) |
| `POST /api/tickets/:key/cancel`, `/route`, `/status` | comme les verbes CLI |
| `GET /api/dispatcher` | `dispatcher.json` + verdict courant |
| `GET /api/dispatcher/plan` | plan à blanc (rien n'est lancé) |
| `POST /api/dispatcher/pause`, `/resume`, `/tick` | commande |

Conventions du démon respectées : camelCase, listes nues, `{"error": …}`, pas de 201. Toute route
mutante est soumise au token (Lot 5a) et au mode lecture seule.

```
aidp ticket start    <KEY> [--force] [--service] [--template] [--track] [--url]
aidp ticket plan     [--format]                 # GET /api/dispatcher/plan ; hors démon : plan() local avec running = []
aidp ticket dispatch [--dry-run] [--url]        # POST /api/dispatcher/tick (ou plan si --dry-run)
aidp gateway dispatcher status|pause|resume|tick [--url] [--format]
```

### Événements du bus

`ticket.created`, `ticket.updated`, `ticket.status {key, from, to, reason, actor}`,
`ticket.started {key, sessionId, project}`, `ticket.reviewed {key, pr}`, `ticket.done {key}`,
`dispatcher.tick {launched: [...], skipped: [...], slotsUsed, slotsMax}`, `dispatcher.paused`,
`dispatcher.resumed`. Une automation
`event` peut donc notifier « ticket en revue » par le webhook générique existant.

## Étapes

Coupe en deux runs conseillée : **B** d'abord (petit, débloque A), ou en parallèle (fichiers
disjoints).

Run B — `aidp-core`, `aidp-cli`, templates :

1. `run start --approve-gate` + repli env ; tests dans `run.rs` (gate cuite, gate inconnue, `*`).
2. `run start --inputs-file` (ou lecture de fichier devant `--inputs` existant) ; test JSON
   échappé ASCII avec accents.
3. Templates `dev-lambda-ts`, `dev-angular` en double demande ; `aidp validate --all`.
4. `SKILL.md` §Procedure (note : une gate pré-approuvée ne produit jamais `awaiting_gate`),
   `AGENTS.md` (contrat `AIDP_AUTO_APPROVE_GATES`, `--inputs-file`).

Run A — crate et démon :

5. `plan.rs` + `TemplateCatalog` minimal ; tests exhaustifs (voir Tests).
6. `lifecycle.rs` ; tests table-driven des transitions.
7. `StartRequest` (`autoApproveGates`, `inputsFile`), env au spawn, `build_prompt`, test
   `old_record_json_still_deserializes` inchangé.
8. `changes::pr_for_branch`. `runs::link` réutilisé pour `run_dir`.
9. `dispatcher.rs` : tick, lancement, gestion de la course `queued`, événements de session, gates,
   orphelins, sonde de merge, état persistant, réveil sur bus.
10. Routes HTTP + verbes CLI (`client.rs`, `lib.rs`) ; `aidp ticket start/plan/dispatch`.
11. `_TODO.md` : section Dispatcher réelle (slots, mode, dernier plan, raisons de skip).
12. `docs/gateway-api.md` (sections Tickets, Dispatcher, événements, verbes, décisions),
    `docs/tickets.md`, ligne « État du chantier » dans `docs/ORCHESTRATOR.md` d'`aidp-viewer`.
13. `cargo test --workspace`, `cargo fmt --all -- --check`, scénario d'acceptation manuel.

## Tests

- `plan()` : jira avant local ; priorité ; FIFO ; manuel devant ; un seul ticket par projet par
  tick ; projet occupé par une session UI → `ProjectBusy` ; plafond 5 avec 3 sessions déjà à
  nous → 2 lancements ; `paused` ; `NeedsRouting`/`NoTemplate` ; choix `Demand` selon les inputs du
  template.
- `lifecycle` : done+PR → in-review ; done sans PR → blocked no-pr ; failed 1/2 → todo ;
  failed 2/2 → blocked ; rate_limited → todo sans tentative ; cancelled ; orphaned avec PR →
  in-review ; gate → blocked gate:pr, answered → in-progress ; merged → done ; closed → blocked.
- Démon : le dispatcher ne laisse jamais d'entrée dans `queue.json` (test : `start()` simulé en
  `queued` → `cancel` appelé, ticket `todo`) ; `dispatcher.json` écrit à chaque tick ;
  `ticket.started` porte le `sessionId`.
- `run start` : approbation cuite avant le premier emit ; script sans `awaiting_gate` pour `pr`.

## Critères d'acceptation

Scénario de recette, démon démarré, budget non contraint :

1. Trois tickets : `PIXEL-A` (miroir, High, cheat-alerts-service), `LOC-1` (Highest, aidp-viewer),
   `LOC-2` (Medium, aidp-viewer). Ordre attendu : `PIXEL-A`, `LOC-1`, `LOC-2`.
2. Au tick : `PIXEL-A` et `LOC-1` démarrent (deux projets), `LOC-2` est `NoSlot`/`ProjectBusy`
   selon la config ; `_TODO.md` reflète les trois états.
3. La session de `LOC-1` termine avec une PR : ticket `in-review`, PR renseignée, `LOC-2` démarre
   dans la minute sans intervention.
4. `aidp ticket start LOC-9` sur un ticket Medium le place devant les `todo` restants et démarre
   dès qu'un slot se libère ; `--force` le démarre immédiatement si son projet est libre.
5. Une session autonome n'attend jamais sur la gate `pr` ; une autre gate met le ticket en
   `blocked: gate:<stage>` et `aidp gateway reply` le débloque.
6. `aidp gateway serve` redémarré en cours de run : au premier tick les tickets orphelins
   repartent en `todo` (ou `in-review` si la PR existe).
7. Aucune entrée dans `aidp gateway queue list` à aucun moment du scénario.

## Risques et points d'attention

- **`autoClose` et Workflow** : une session autonome ferme son stdin après le premier tour sans
  question ; le driver v7 exécute tout le Workflow dans ce tour (c'est déjà le mode des
  automations). Vérifier au premier run réel qu'aucune `AskUserQuestion` implicite du driver
  (choix de template, inputs manquants) ne survient : le prompt dit déjà « template et cible
  finals », et `--inputs-file` fournit `track`.
- **Sessions UI sur le même projet** : elles occupent le projet (règle du démon) et font `skip`
  le ticket ; c'est voulu (D4), à afficher clairement comme raison.
- **Nom de projet** : `Projects::find(id_or_name)` par nom ; un service inconnu du démon →
  `NeedsRouting` avec message « enregistrer le projet ou corriger `service` ».
- **Écritures concurrentes CLI/démon** sur une même note : écriture atomique + relecture avant
  transition (comparer `updated`) ; en cas de conflit, le démon relit et rejoue au tick suivant.
- **Type fermé `OriginKind` côté viewer** : `dispatcher` s'affichera comme inconnu jusqu'au lot 9b ;
  le démon ne valide pas `origin.kind`, rien ne casse.
- **Ne pas dupliquer la règle une-session-par-projet** : le dispatcher s'en sert, il ne la refait pas.

## Dépendances et taille

Dépend de 7a et 8a. Taille **L** ; deux runs `dev-aidp` (B puis A) en `track: deep`.
