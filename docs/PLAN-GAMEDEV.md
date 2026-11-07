# aidp gamedev — plan d'adaptation du harness au développement de jeu Unity perso

Date : 2026-11-06. Base analysée : kit `aidp-perso-all` (aidp 7.5.19, skill `ai-dev-pipeline-v7`
branche `feat/perso-profile`), commande `/issue-queue`, projet `PixelHood`
(`D:\CLAUDE_UNITY\mixandjam-celestemovement`, Unity 6000.6.7f1, Input System 1.20, LDtk, Aseprite).

Outils cibles : Unity (+ MCP Unity), Aseprite (MCP), Pixellab (MCP), GitHub (issues, PR), aidp.
Hors périmètre : Jira, Confluence, Sonar, GitHub Actions, CI, Teams, déploiements.

Contrainte machine : **rien ne s'installe sur C:** (disque de jeu). Toolchains, binaires, caches,
état du daemon, vault et front vont sur D: (voir §8).

---

## 1. Constat

### 1.1 Ce que le harness sait déjà faire (et qu'on garde)

| Brique | Où | Intérêt pour le jeu |
|---|---|---|
| Compilateur de pipelines YAML → plan d'exécution typé | `aidp-core` (`compose`, `pipeline`, `exec_plan`, `workflow_emit`, `prompt`) | Socle de tous les pipelines Unity, art, map |
| Exécution **native** par le daemon (une session `claude -p` par stage, reply validée `agent-reply.v4`, réparation 1×, autonomie sans question) | `aidp-gateway/src/native_runs` | Les pipelines tournent sans session Claude Desktop ouverte |
| Moteur de **flows** (machine à états pure, reprise après redémarrage, `action` / `pipeline` / `wait` / `human` / `loop`) | `aidp-flows` + `aidp-gateway/src/flows` | Remplace l'orchestration « à la main » de `/issue-queue` |
| **Dispatcher** de tickets avec slots, file d'attente, priorités, bloqueurs `waiting-on:` | `aidp-gateway/src/dispatcher.rs`, `aidp-tickets` | Choisit la prochaine issue à jouer |
| **Budget tokens** : fenêtres 5 h / 7 j, seuils, réserve par run apprise par template, verdict `allow` / `wait` | `aidp-tickets/src/budget.rs`, `GET /api/dispatcher/budget` | Base de la gestion de consommation |
| **Usage** agrégé par jour / projet / template / origine, journal des rate limits | `aidp-gateway/src/usage.rs`, `GET /api/usage`, `/api/usage/rate-limit` | Tableaux de bord du front |
| Ledger de coût par stage, détection d'anomalies, re-pricing | `aidp-core/src/cost`, `aidp cost` | Coût par issue |
| Sessions, flux SSE, réponses, interruptions, worktrees, santé, bus d'événements | API gateway (`docs/gateway-api.md`) | Tout ce dont le front a besoin |
| Lecture de PR (détail, fils, commentaires, marques) | `aidp-gateway/src/pr_*.rs` | Écran « PR » du front |
| Vault Obsidian des runs | `aidp-core/src/obsidian.rs`, `vault.rs` | Historique lisible des runs |
| Pipelines Unity : `dev-unity`, `review-unity`, `fix-review-unity`, `pixelart` ; flow `perso-delivery` | skill `templates/` | Point de départ des routes code et art |
| Agents Unity et pixel art (implementer, reviewer, fixer, playtester, art-director, artist, animator, reviewer, fixer) | `~/.claude/agents` | Gardés, retouchés |

### 1.2 Ce qui coince aujourd'hui

1. **`/issue-queue` orchestre à la main.** Une session Claude « dispatcher » démarre des sessions
   Desktop, attend des `SendMessage`, s'appuie sur un cron de 15 min pour se réveiller, et tient
   l'état dans les labels. C'est fragile (réveils manqués, mémoire `issue-queue-autonomy`), ça coûte
   des tokens pour rien (le dispatcher relit tout à chaque réveil) et ça ne survit pas à la fermeture
   de l'app. Le moteur de flows du daemon fait exactement ce travail, de façon déterministe.
2. **Les tickets d'aidp viennent de Jira ou de notes locales**, pas des issues GitHub. Ta source de
   vérité est GitHub ; l'état vit dans les labels `aidp:*` posés par `/issue-queue`, que le daemon
   ignore.
3. **Le test dans Unity dépend du computer use.** Le playtester joue au clavier dans la Game view par
   captures d'écran : lent, cher en tokens, fragile, et disponible seulement dans une session Claude
   Desktop, donc impossible dans un run natif du daemon.
4. **Contention sur l'éditeur.** `implement` compile en batchmode (éditeur fermé), le playtest ouvre
   l'éditeur et le laisse ouvert ; deux lanes qui touchent Unity en même temps se marchent dessus.
5. **Le merge est le goulot.** Aujourd'hui 10 issues portent `aidp:done` mais sont encore ouvertes,
   avec 10 PR ouvertes : rien ne te présente « ces PR attendent ton clic ».
6. **Les routes s'arrêtent à `dev` et `pixelart`.** Les maps LDtk (skills `map-create`,
   `map-world-design`, `map-room-design`, `ldtk`) et la BIBLE n'ont pas de pipeline.
7. **Aucun budget Pixellab.** Les générations sont un deuxième budget, à côté des tokens, que rien
   ne suit.
8. **Pas de front.** `aidp-viewer` (React + Express) n'est pas dans le kit.
9. **Environ 40 % du code et 45 templates servent le contexte pro** (Jira, Sonar, CI, Teams,
   Mongo, SQS, Copilot, Codex, .NET/FFI).

---

## 2. Cible

```
GitHub issues (labels = état + route)
        │  sync (gh) toutes les N min + webhook manuel « Sync »
        ▼
aidp gateway (daemon, D:\AIDP\gateway)
  ├─ tickets GH-<n>  ──► dispatcher (slots, priorité, bloqueurs, budget tokens + Pixellab)
  ├─ flow par route  ──► pipelines natifs (claude -p, autonomes)
  │       code : implement → review ⇄ fix → test Unity ⇄ fix → PR ready → merge (ton clic)
  │       art  : brief → génère → anime → review ⇄ fix → import Unity → PR ready → merge
  │       map  : graphe → salles LDtk → review ⇄ fix → test Unity (traversée) → merge
  │       doc  : rédige BIBLE/Docs → review ⇄ fix → merge
  ├─ verrou « unity-editor » (une seule étape Unity à la fois)
  ├─ usage / coût par issue / budget
  └─ bus SSE
        ▼
aidp-studio (front neuf, D:\AIDP\src\aidp-studio, servi par le daemon) : board, issue, inbox, Unity, budget, réglages
```

Principe repris du viewer : **le front affiche, le daemon décide.** Toute règle métier vit dans
`aidp-gateway` ; le front n'appelle que l'API.

---

## 3. Gestion des issues GitHub (état → workflow joué)

### 3.1 Labels

Deux familles, posées et retirées **uniquement par le daemon** (et par toi via le front) :

**Route** (quel flow) — première règle qui matche, comme aujourd'hui dans `/issue-queue` :

| Label | Route / flow | Remarque |
|---|---|---|
| `pixelart`, `art`, `sprite`, `pixel:*`, `asset:*` | `game-art` | existant |
| `map`, `ldtk`, `level`, `salle` | `game-map` | nouveau |
| `doc`, `bible`, `documentation` | `game-doc` | nouveau |
| `bug` | `game-code` avec `track: bug` (test de repro d'abord) | nouveau comportement |
| `dev`, `code`, `feature`, `enhancement`, `tech` | `game-code` | existant |
| aucun | mots-clés du titre et du corps, sinon `aidp:triage` | le daemon ne devine plus `dev` en dernier recours |

**État** (où en est l'issue) :

| Label | Sens | Statut ticket aidp | Posé par |
|---|---|---|---|
| `aidp:triage` | Issue incomplète (pas de critères `CA-n`) | `blocked: needs-refine` | sync |
| `aidp:ready` | Prête, candidate au dispatch | `todo` | toi (front ou GitHub) |
| `aidp:queued` | Retenue, attend un slot, l'éditeur ou le budget | `queued` | dispatcher |
| `aidp:in-progress` | Réalisation en cours | `in-progress` | flow |
| `aidp:review` | Revue avocat du diable en cours | `in-progress` | flow |
| `aidp:fixing` | Correction après revue ou test | `in-progress` | flow |
| `aidp:testing` | Test dans Unity en cours | `in-progress` | flow |
| `aidp:awaiting-merge` | PR prête, attend ton clic | `in-review` | flow |
| `aidp:done` | PR mergée, issue fermée par `Closes #n` | `done` | flow |
| `aidp:blocked` | Besoin d'un humain ; la raison est en commentaire | `blocked: <raison>` | flow, dispatcher |
| `aidp:skip` | Jamais dispatchée | — | toi |

`aidp:need-fix` disparaît : le verdict de revue est une sortie de step, pas un état à part.
Migration : un script unique relabellise l'existant (`aidp:done` + PR ouverte → `aidp:awaiting-merge`).

### 3.2 Ce qu'il faut ajouter à aidp

| Lot | Changement | Fichiers |
|---|---|---|
| G1 | **Source `github` pour les tickets** : `aidp ticket sync --source github` lit `gh issue list --json number,title,body,labels,milestone,state` et miroite chaque issue en `TODO/<repo>/GH-<n>.md` (corps = corps de l'issue). Remplace `jira_sync.rs` en gardant son contrat (créations, annulations, `unrouted`). | `aidp-tickets/src/github_sync.rs` (nouveau), `config.rs`, `aidp-cli` `ticket sync` |
| G2 | **Write-back par labels** : chaque transition de statut (et chaque phase de flow) remplace le label `aidp:*` d'état. Config `github.status_labels` à la place de `jira.status_map`. | `aidp-tickets/src/lifecycle.rs`, action de flow `gh.issue_label` |
| G3 | **Routage par labels vers un flow** : `routing.yaml` gagne `flow:` à côté de `template:` ; le dispatcher démarre ce flow (`POST /api/tickets/:key/flow` le sait déjà). | `aidp-tickets/src/routing.rs`, `dispatcher.rs` |
| G4 | **Bloqueurs GitHub** : « Dépend de #n » / « Blocked by #n » dans le corps et les dépendances natives GitHub (`gh api .../issues/<n>/dependencies/blocked_by`) alimentent `waiting-on:GH-<n>`. | `aidp-tickets/src/blockers.rs` |
| G5 | **Ordre de dequeue** : milestone (la plus proche d'abord), puis label `prio:high|low`, puis numéro. La priorité manuelle du front passe devant. | `aidp-tickets/src/order.rs` |
| G6 | **Triage** : une issue sans `CA-n` passe `aidp:triage` ; bouton « Affiner » dans le front qui lance un pipeline `issue-refine` (skill `github-issue-ticket`) qui réécrit le corps puis remet `aidp:ready`. | template `issue-refine.yaml` |
| G7 | **Actions de flow GitHub** : `gh.issue_label`, `gh.issue_comment` (rapport de chaque étape sur l'issue), `gh.merge` existant, fermeture via `Closes #n`. | `aidp-gateway/src/flows/actions/` |

Exemple de `routing.yaml` :

```yaml
defaults: { track: standard }
rules:
  - { match: { label: pixelart }, service: pixelhood-art }
  - { match: { label: map },      service: pixelhood-map }
  - { match: { label: doc },      service: pixelhood-doc }
  - { match: { label: bug },      service: pixelhood-bug }
  - { match: { label: dev },      service: pixelhood-code }
repos:
  pixelhood-code: { flow: game-code, track: standard }
  pixelhood-bug:  { flow: game-code, track: bug }
  pixelhood-art:  { flow: game-art }
  pixelhood-map:  { flow: game-map }
  pixelhood-doc:  { flow: game-doc }
```

---

## 4. Workflow autonome : Réalise → Review → Test dans Unity

### 4.1 Flow `game-code` (remplace `perso-delivery` + les lanes de `/issue-queue`)

```yaml
id: game-code
version: 1
inputs:
  ticket: { required: true }
settings:
  max_review_rounds: 2
  max_test_rounds: 2
steps:
  - { id: claim,     kind: action,   use: gh.issue_label, with: { state: in-progress }, phase: Préparation }
  - { id: branch,    kind: action,   use: git.branch,     phase: Préparation }
  - { id: implement, kind: pipeline, use: unity-implement, pushes: true, resource: unity-editor, phase: Réalisation }
  - id: review_loop
    kind: loop
    phase: Revue
    max: "{{ settings.max_review_rounds }}"
    until: "steps.review.outputs.verdict == 'done'"
    body:
      - { id: review_label, kind: action, use: gh.issue_label, with: { state: review } }
      - { id: review, kind: pipeline, use: unity-review }            # avocat du diable, lecture seule
      - { id: fix, kind: pipeline, use: unity-fix, pushes: true, resource: unity-editor,
          when: "steps.review.outputs.verdict == 'need-fix'" }
  - id: test_loop
    kind: loop
    phase: Test Unity
    max: "{{ settings.max_test_rounds }}"
    until: "steps.test.outputs.passed == true"
    body:
      - { id: test_label, kind: action, use: gh.issue_label, with: { state: testing } }
      - { id: test, kind: pipeline, use: unity-test, resource: unity-editor }
      - { id: fix_test, kind: pipeline, use: unity-fix, pushes: true, resource: unity-editor,
          with: { report_from: test }, when: "steps.test.outputs.passed == false" }
  - { id: ready,  kind: action, use: gh.pr_ready, phase: Merge }
  - { id: await,  kind: action, use: gh.issue_label, with: { state: awaiting-merge }, phase: Merge }
  - { id: merge_approval, kind: human, phase: Merge, prompt: "Merger la PR ?", choices: [approve] }
  - { id: merge,  kind: action, use: gh.merge, phase: Merge }
  - { id: done,   kind: action, use: gh.issue_label, with: { state: done }, phase: Merge }
  - { id: clean,  kind: action, use: worktree.remove, on_error: warn, phase: Nettoyage }
```

Points clés :

- **Revue avant test**, comme tu le demandes. La revue (lecture seule, sans Unity) tourne pendant que
  l'éditeur sert à une autre issue : c'est là qu'on gagne le parallélisme.
- **Deuxième revue après correction** (jusqu'à 2 tours), au lieu de « fix puis done » sans contrôle
  comme dans `/issue-queue`. Une boucle épuisée finit `escalated` → `aidp:blocked`.
- **Chaque étape commente l'issue** (`gh.issue_comment`) : verdict, rapport, coût de l'étape.
- `track: bug` insère au début de `unity-implement` une étape « écrire le test qui reproduit le bug
  et vérifier qu'il échoue ».
- `resource: unity-editor` est nouveau (voir §5.1).

### 4.2 Pipelines par route

| Pipeline | Stages (modèle) | Issu de |
|---|---|---|
| `unity-implement` | prepare (haiku) → plan (opus) → implement + tests EditMode + compile gate (opus) → report (haiku) | `dev-unity` sans le playtest |
| `unity-review` | prepare (haiku) → revue avocat du diable critère par critère, preuve par critère (opus) → verdict (haiku) | `review-unity` + prompt « avocat du diable » de `/issue-queue` |
| `unity-fix` | fix des points ❌/⚠️ de la revue ou des scénarios KO (sonnet) → compile gate | `fix-review-unity` |
| `unity-test` | compile gate → tests EditMode + PlayMode → scénarios PlaytestDriver par `CA-n` → preuves (captures, GIF) → verdict (sonnet) | `unity-playtester` réécrit (§5) |
| `art-produce` | brief → génère → anime → verify ⇄ correction (opus/sonnet) | `pixelart` |
| `art-review` | avocat du diable art : mesures Aseprite (`lint`, `anim_audit`, `unity_validate`), conformité au brief et à la palette | prompt pixelart de `/issue-queue` |
| `art-import` | import Unity : `.meta` (16 PPU, Point, sans mipmap), sprite placé dans une scène de test, animation jouée, capture | nouveau |
| `map-produce` | `map-world-design` (graphe validé par script) → `map-room-design` → `ldtk` → métriques et rendu PNG | skills map existants |
| `map-review` | solvabilité du graphe, sauts calibrés sur le mouvement du joueur, cohérence BIBLE | nouveau |
| `map-test` | import LDtk compilé, salle chargée, traversée des chemins critiques par PlaytestDriver (téléport + scénarios) | nouveau |
| `doc-write` / `doc-review` | BIBLE et `Docs/` : rédaction, cohérence avec le canon (`04_regle_de_conflit`) | nouveau |
| `issue-refine` | réécrit une issue au format `github-issue-ticket` (CA-n, T-n) | skill existant |

Les flows `game-art`, `game-map`, `game-doc` reprennent la forme de `game-code` (claim → produce →
review ⇄ fix → test → merge gate), `game-doc` sans test Unity, `game-art` avec `art-import` comme test.

---

## 5. Test dans Unity : autonomie sans computer use

### 5.1 Verrou de l'éditeur

- Nouvelle ressource du daemon `unity-editor` par projet (même mécanique que le « one writer per
  workspace » et `pr_workspace_holder`). Une étape `resource: unity-editor` attend le verrou
  (`waiting_external`, raison `resource:unity-editor`) ; le front montre qui le tient.
- **L'éditeur reste ouvert** pendant toute la file. Toute compilation passe par le MCP Unity
  (pas de batchmode tant que l'éditeur tourne) ; le batchmode reste pour une vérification de nuit.
- Le checkout principal est le seul « lane Unity » (une Library). Les revues et la génération d'art
  tournent dans des worktrees sans Unity. Option P7 : un deuxième lane Unity en worktree avec copie
  de la Library.

### 5.2 Compile gate (repris d'AIBridge)

Une étape standard, appelée par `unity-implement`, `unity-fix`, `unity-test` :
`AssetDatabase.Refresh` → `CompilationPipeline.RequestScriptCompilation` → attente de fin → lecture
de la console → JSON `{ ok, errors: [{file, line, message}], warnings: n }`. Zéro erreur ou l'étape
échoue avec la liste. Le contrat est écrit dans `unity-verify.md` et utilisé partout.

### 5.3 PlaytestDriver (package du jeu, éditeur et builds de dev seulement)

Le cœur de l'autonomie : l'agent **joue au jeu par l'Input System**, sans captures d'écran.

| Pièce | Rôle |
|---|---|
| Clavier et manette virtuels | `InputSystem.AddDevice<Keyboard>("AidpKeyboard")`, `QueueStateEvent` frame par frame : `hold`, `press`, `release`, `tap`, `wait`. Le jeu les voit comme de vraies touches. |
| Sondes d'état (`IPlaytestProbe`) | Le code du jeu expose ce qu'un test doit lire : position et vitesse du joueur, au sol, mur, dash dispo, capacités, salle courante, PV. Une sonde = une classe, enregistrée par attribut. |
| Commandes de debug (`IPlaytestCommand`) | Téléport, donner une capacité, charger une salle, `timeScale`, invincibilité : mettre le joueur dans l'état de départ d'un scénario en une ligne (pattern `IAIBridgeHandler` d'AIBridge). |
| Scénarios en données | `Assets/Tests/Playtest/Scenarios/issue-<n>.json` : `setup` (commandes) → `steps` (entrées) → `assert` (expressions sur les sondes, avec tolérance et délai). |
| Runner PlayMode | Chaque scénario devient un test PlayMode (`[UnityTest]` paramétré par fichier). La suite de non-régression grossit d'un scénario par issue. |
| Pont fichier | `.aidp/playtest/<id>.json` → résultat `.aidp/playtest/<id>.result.json`, lu par l'agent ou par le MCP (`Unity_RunCommand`). Moins de tokens qu'une conversation MCP longue. |
| Preuves | Captures `ScreenCapture` aux moments clés et GIF court par scénario (`D:\DevTools\ffmpeg` portable), jointes au commentaire de PR. |

Exemple de scénario (« le dash traverse un trou de 4 tuiles ») :

```json
{
  "id": "issue-214-ca2",
  "scene": "Assets/Scenes/Level_1.unity",
  "setup": [ { "cmd": "teleport", "x": 12, "y": 3 }, { "cmd": "give", "ability": "dash" } ],
  "steps": [ { "hold": "rightArrow", "ms": 300 }, { "press": "x" }, { "wait": 400 } ],
  "assert": [ "player.x > 17.5", "player.grounded == true", "player.hp == 3" ]
}
```

Le playtester (`unity-test`) écrit un scénario par `CA-n`, le lance, lit le résultat, et ne recourt
au computer use que pour un critère de « feeling » marqué `needs-feel-check` sur l'issue, dans une
session Desktop (mode `agentExecutor: external` déjà prévu par le daemon).

### 5.4 Repro par enregistrement

`InputEventTrace` enregistre une partie jouée par toi (bouton « Enregistrer une repro » dans le
front ou raccourci dans l'éditeur) et la sauve en scénario. Une issue `bug` peut ainsi porter sa
repro exacte, rejouée avant et après le fix.

---

## 6. Gestion de la consommation de tokens

Déjà là : fenêtres 5 h / 7 j, verdict de budget du dispatcher avec réserve apprise par template,
`GET /api/usage`, ledger `aidp cost`, anomalies. À ajouter :

| Lot | Ajout | Détail |
|---|---|---|
| T1 | **Coût par issue** | `GET /api/usage?groupBy=ticket` (clé = `origin.ref` des sessions de flow) et `GET /api/tickets/:key/cost` : tokens par flow, step, stage, modèle ; part des fenêtres 5 h / 7 j consommée. |
| T2 | **Pied de rapport** | Chaque commentaire d'étape et le corps de PR finissent par « Coût : 1,2 M tokens (in/out/cache), 4 % de la fenêtre 5 h ». |
| T3 | **Plafond par issue et par route** | `budget.per_ticket: { code: 3M, art: 1.5M, map: 2M, doc: 0.5M }`. Dépassement → le flow passe `waiting_human` (gate « continuer ? ») au lieu de brûler la fenêtre. |
| T4 | **Budget Pixellab** | Snapshot `get_balance` avant/après chaque run art ; générations par issue ; réserve minimale dans le verdict du dispatcher (pas de lancement art sous la réserve) ; plafond par issue (10 générations comme aujourd'hui). |
| T5 | **Politique de modèles** | Table route × stage → modèle, éditable dans le front. Le front montre le coût moyen par stage pour décider les rétrogradations (ex. `report`, `prepare`, `verdict` en haiku ; `review` art en sonnet). |
| T6 | **Planification par fenêtre** | Mode « nuit » : le dispatcher lance les runs lourds (opus) juste après un reset de la fenêtre 5 h ; seuil « stop à X % de la fenêtre 7 j ». |
| T7 | **Économies structurelles** | Sondes PlaytestDriver au lieu de captures (le plus gros gain) ; BIBLE chargée par index (`00_INDEX`) et non en entier ; `rtk` partout ; dispatcher sans session Claude (le daemon remplace le dispatcher de `/issue-queue`, qui relisait tout à chaque réveil). |

---

## 7. Front : aidp-studio (front neuf, décidé le 2026-11-06)

Front neuf, centré jeu, sans reprise d'`aidp-viewer`. Il ne parle qu'au daemon : aucun appel direct
à `gh`, Unity ou Pixellab. **Le front affiche, le daemon décide.**

### 7.1 Architecture

| Élément | Choix |
|---|---|
| Sources | `D:\AIDP\src\aidp-studio` (dépôt git à lui) |
| Stack | Vite, React, TypeScript strict, Tailwind, TanStack Query (cache et invalidation), TanStack Router |
| Données | REST `http://127.0.0.1:3001/api/*` ; temps réel par `GET /api/events` (SSE du bus) et `GET /api/sessions/:id/stream` |
| Temps réel | Un seul `EventSource` global ; chaque type d'événement invalide les requêtes concernées (`flow.step_changed` → `["flow", key]`, `ticket.status` → `["tickets"]`, `gateway.rate_limit` → `["budget"]`…). Reconnexion avec reprise sur le tampon `Bus::recent` du daemon. |
| Service | Le daemon sert le build : `aidp gateway serve --ui-dir D:\AIDP\src\aidp-studio\dist` (route statique ajoutée dans `http.rs`, repli sur `index.html` pour le routeur). Aucun serveur Node en usage normal. |
| Dev | `npm run dev`, proxy Vite `/api` → `127.0.0.1:3001` |
| Sécurité | Écoute sur `127.0.0.1` seulement. Un appel loopback passe sans jeton (règle actuelle du daemon) ; `AIDP_UI_TOKEN` si un jour tu y accèdes depuis le téléphone. |
| Types | `src/api/types.ts` écrit à la main d'après `docs/gateway-api.md` au début ; plus tard générés depuis les structs Rust (`schemars` → JSON Schema → TS). |
| Tests | `npm run typecheck`, `npm run build`, Vitest + Testing Library sur les composants à logique (board, timeline, jauges). Vérification visuelle dans le navigateur intégré de Claude, sans Playwright (il télécharge ses navigateurs sur C:). |
| Disque | `npm_config_cache=D:\DevTools\npm-cache` ; `node_modules` dans le dépôt sur D: ; Node déjà présent réutilisé. |

### 7.2 Disposition

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ aidp-studio   PixelHood ▾      ● daemon   Unity: ouvert, verrou #214   5h ▓▓▓░ 62%  7j ▓░░ 21% │
├──────────┬────────────────────────────────────────────────────────────────────┤
│ Board    │                                                                    │
│ Inbox (3)│                    page courante                                   │
│ Unity    │                                                                    │
│ Budget   │                                                                    │
│ Réglages │                                                                    │
└──────────┴────────────────────────────────────────────────────────────────────┘
```

La barre du haut est toujours visible : état du daemon, de l'éditeur Unity (et qui tient le
verrou), jauges 5 h et 7 j avec le reset, pastille de l'inbox.

### 7.3 Pages, données et actions

| Page | Contenu | Actions | Routes existantes | Routes à ajouter |
|---|---|---|---|---|
| **Board** | Colonnes = états §3.1 (Triage, Prêtes, En file, Réalisation, Revue, Test Unity, À merger, Bloquées, Faites). Carte : `#n`, titre, icône de route, milestone, étape en cours, durée, coût, tours de revue et de test. Filtres : route, milestone, label. | Rendre prête, Lancer maintenant, Monter en priorité, Pause / reprise du flow, Skip, Annuler, Sync GitHub, Pause du dispatcher | `GET /api/tickets`, `GET /api/flows`, `POST /api/tickets/:key/start`, `/cancel`, `/status`, `POST /api/flows/:key/pause`, `/resume`, `POST /api/dispatcher/pause`, `/resume`, `GET /api/queue`, `POST /api/queue/:id/promote` | `POST /api/tickets/sync?source=github` (G1) ; champs `route`, `labels`, `milestone`, `flowSummary`, `cost` dans `GET /api/tickets` |
| **Issue** | En-tête : issue, PR (statut, diff résumé), route, coût total. Timeline du flow par phase : étape, statut, durée, tokens, itération. Panneau session en direct (texte, outils, questions). Onglets Artefacts (plan, revue, rapport de test rendus en Markdown), Preuves (captures, GIF, aperçus de sprites), Hypothèses (décisions prises sans demander), Journal. | Approuver ou rejeter un gate, répondre à une question, relancer ou passer une étape, lancer une action ou un pipeline manuel, envoyer des commentaires de PR à un fix | `GET /api/flows/:key`, `/journal`, `POST /api/flows/:key/steps/:id/{retry,skip,approve,reject}`, `POST /api/flows/:key/manual`, `GET /api/sessions/:id/stream`, `POST /api/sessions/:id/reply`, `GET /api/runs/:id` (assumptions), routes PR detail, `POST` PR comments → run | `GET /api/runs/:id/files` et `GET /api/runs/:id/files/*path` (lecture seule des fichiers du dossier de run : Markdown, PNG, GIF) ; `GET /api/tickets/:key/cost` (T1) |
| **Inbox** | Tout ce qui attend une décision : merges (`merge_approval`), issues bloquées avec la raison, questions de session, dépassements de budget par issue | Merger en un clic (approve du gate), répondre, débloquer (relancer l'étape ou remettre `aidp:ready`) | `GET /api/inbox`, `GET /api/flows?status=`, routes `steps/:id/approve` | — |
| **Unity** | Éditeur (ouvert, PID, projet), verrou `unity-editor` (tenu par quelle issue et quelle étape, depuis quand), dernière compile gate (erreurs avec fichier et ligne), derniers tests EditMode et PlayMode, bibliothèque de scénarios avec leur dernier résultat | Lancer un scénario, lancer la non-régression, enregistrer une repro, libérer un verrou orphelin | — | `GET /api/unity/status`, `POST /api/unity/lock/release`, `GET /api/unity/scenarios`, `POST /api/unity/scenarios/:id/run`, `POST /api/unity/recordings` (P4) |
| **Budget** | Jauges 5 h et 7 j avec compte à rebours, verdict du dispatcher et sa raison, tokens par jour, route, stage et modèle, top des issues les plus chères, crédits Pixellab, anomalies | (F2) plafonds par route, mode nuit, politique de modèles | `GET /api/usage?groupBy=`, `GET /api/usage/rate-limit`, `GET /api/dispatcher/budget` | `groupBy=ticket\|model\|stage` (T1), `GET /api/pixellab/balance` (T4), `GET /api/cost/anomalies` |
| **Réglages** | Routage labels → flow, politique de modèles, slots, seuils de budget, projet et chemins | Édition validée par le daemon avant écriture | `GET /api/catalog/flows`, `GET /api/catalog/templates`, `GET /api/gateway/status`, `GET /api/health` | `GET`/`PUT /api/settings/{routing,budget,models}` (écrivent `TODO/routing.yaml` et `TODO/config.yaml`) |

### 7.4 Lots du front

| Lot | Contenu | Critère de fin |
|---|---|---|
| **F0 Squelette** | Projet Vite, client API typé, `EventSource` global, layout et barre du haut, page « daemon injoignable », route statique `--ui-dir` côté daemon | `typecheck` et `build` verts ; `http://127.0.0.1:3001/` sert l'app ; la barre du haut affiche les jauges réelles |
| **F1 MVP** | Board, Issue (sans Preuves), Inbox avec merge en un clic, Budget en lecture | Une issue réelle se suit de `aidp:ready` à `aidp:done` sans quitter le front ; les 10 PR en attente se mergent depuis l'Inbox |
| **F2 Complet** | Onglet Preuves, page Unity, Réglages, édition du budget et des modèles, galerie art par issue | Chaque page lit et écrit uniquement par l'API du daemon |

F0 peut démarrer dès P1 (il n'a besoin que des routes existantes) ; F1 suit P2 (issues GitHub) ;
F2 suit P4 et P5.

---

## 8. Emplacement sur disque (rien sur C:)

| Élément | Emplacement | Comment |
|---|---|---|
| Sources aidp + skill | `D:\AIDP\src\aidp`, `D:\AIDP\skill` (dépôts git) | `AIDP_SRC`, `AIDP_SKILL_ROOT` |
| Toolchain Rust | `D:\DevTools\rust\rustup`, `D:\DevTools\rust\cargo` | `RUSTUP_HOME`, `CARGO_HOME` posés **avant** `rustup-init` (sinon il installe dans `C:\Users`) |
| Compilateur C (pour la crate `ring`) | `D:\DevTools\winlibs` (MinGW-w64 portable, un zip) | Toolchain `1.95.0-x86_64-pc-windows-gnu` : pas de Visual Studio Build Tools ni de Windows SDK, absents de la machine et qui s'installent sur C: |
| Cibles de build | `D:\AIDP\target` | `CARGO_TARGET_DIR` |
| Binaire | `D:\AIDP\bin\aidp.exe` (déjà là) | PATH utilisateur |
| État du daemon | `D:\AIDP\gateway` | `AIDP_GATEWAY_DIR` (sinon `~/.aidp/gateway` sur C:) |
| Vault | `D:\AIDP\Vault` (déjà là) | `AIDP_VAULT` |
| Front | `D:\AIDP\src\aidp-studio` (build dans `dist/`), cache npm `D:\DevTools\npm-cache` | `npm_config_cache` ; Node existant réutilisé, rien d'installé |
| ffmpeg (GIF) | `D:\DevTools\ffmpeg` (zip portable) | chemin dans la config |
| Agents, skills, commandes du jeu | `<projet>\.claude\` (sur D:) | Claude Code lit les `.claude/` de projet ; les nouveaux fichiers vont là, pas dans `~/.claude` |

Restent sur C:, parce que Claude Code ne lit qu'eux : `~/.claude/settings.json` (le hook
`aidp run summary`), `~/.claude.json` (MCP Unity, Aseprite, Pixellab), `~/.aidp/config.json`
(98 octets, peut pointer vers D:). Ce sont des fichiers de config de quelques Ko, pas des
installations. Le nouveau `setup-gamedev.ps1` remplace `setup-perso.ps1` et ne touche qu'à eux.

---

## 9. Ce qu'on retire

Retrait par suppression dans un fork perso (pas de feature flag) : moins de code, c'est aussi
moins de tokens pour les agents qui feront évoluer aidp.

| Bloc | Taille approx. | Où |
|---|---|---|
| Jira (client, writer, sync, CLI, actions `jira.*`, write-back, release watch) | ~2 500 lignes | `aidp-integrations/src/jira*.rs`, `aidp-tickets/src/jira_sync.rs`, `release_watch.rs`, `aidp-cli` `jira` |
| CI autopilot, GitHub Actions, déploiement (`ci.watch`, probes `ci_checks` et `copilot_review`, `deploy.dev`, hooks CI, `gha-fix`) | ~2 300 lignes | `aidp-integrations/src/ci.rs`, `aidp-cli` `ci`, `flows/probes`, `flows/actions/deploy_dev.rs`, `release_probe.rs` |
| Sonar (+ `sonar-fix`) | ~950 lignes | `sonar.rs`, `aidp-cli` `sonar` |
| Confluence, Teams, Mongo, SQS | ~620 lignes | `aidp-integrations`, `aidp-cli` |
| Providers Codex et Copilot | ~1 800 lignes | `aidp-gateway/src/{codex,copilot}_provider*` |
| Hôte .NET et FFI | ~1 000 lignes | `aidp-ffi`, `dotnet/` |
| Benchmark | ~700 lignes | `aidp-bench`, `aidp-e2e/benchmark` |
| Templates pro | ~45 fichiers | `*-<domaine pro>`, `*-lambda*`, `dev-angular`, `dev-csharp*`, `study-to-jira`, `ticket-create`, `datadog-*`, `gdpr-*`, `sonar-fix`, `gha-fix`, `func-doc`, `tech-doc`… |
| Flows pro | 3 | `ticket-delivery`, `local-delivery`, `test-flow` |

Gardés finalement (décidé pendant P1, le 2026-11-06) :

- **Knowledge graph et notes de connaissance** (`aidp-core/src/{kg,knowledge*}.rs`, `aidp-cli`
  `knowledge`, templates `knowledge-init|read|update`) : tu lances graphify sur le projet Unity.
- **Squads et boîte de PR** (`aidp-integrations/src/squad*`, `aidp-gateway/src/squad_prs.rs`,
  `prs.rs`) : liés à 61 fichiers des modules de PR. L'équipe par défaut est vide ; sans équipe,
  la boîte lit `extra_repos` et les dépôts enregistrés. À simplifier avec le front (F1).

Gardés aussi : `dev-aidp` et `dev-studio` (issu de `dev-aidp-viewer`) pour qu'aidp continue de se
développer lui-même ; les automations et le cron du daemon (sync GitHub périodique, mode nuit),
les worktrees, la santé et le nettoyage, le vault.

---

## 10. Lots et ordre

| Lot | Contenu | Taille | Dépend de |
|---|---|---|---|
| **P0 Socle D:** ✅ | `git init` des sources sur D: ; Rust GNU + MinGW portable sur D: ; `cargo check` et tests ; variables d'env (§8). Voir §12. | S | — |
| **P1 Nettoyage** ✅ | §9 ; corriger les 4 incohérences de l'instantané (§12) ; adapter `dev-aidp` (skills pro absents) pour qu'aidp se développe lui-même ; `setup-gamedev.ps1` côté skill (hook, agents, config) qui remplace `setup-perso.ps1` ; bascule d'`AIDP_SKILL_ROOT` vers `D:\AIDP\skill` ; build release installé dans `D:\AIDP\bin` ; `aidp doctor` profil gamedev ; `validate --all`, tests unitaires et d'intégration, `clippy -D warnings` verts | M | P0 |
| **P2 Issues GitHub** ✅ | G1 à G7 ; migration des labels existants ; `routing.yaml` du jeu | M | P1 |
| **P3 Flows et pipelines** ✅ | `game-code`, `game-art`, `game-map`, `game-doc` ; pipelines §4.2 ; ressource `unity-editor` ; le daemon remplace `/issue-queue` (gardée en secours) | L | P2 |
| **F0 Front squelette** ✅ | §7.4 : projet Vite, client API, SSE, barre du haut, route `--ui-dir` du daemon | S | P1 |
| **F1 Front MVP** ✅ | §7.4 : Board, Issue, Inbox (merge en un clic), Budget lecture seule ; routes daemon `runs/:id/files`, champs de `GET /api/tickets` | M | F0, P2 (en parallèle de P3) |
| **P4 Unity dans la boucle** ✅ (code ; à éprouver dans l'éditeur) | Compile gate ; package PlaytestDriver dans PixelHood (sondes, commandes, scénarios, runner PlayMode, pont fichier, preuves) ; `unity-test` réécrit ; `map-test` | L | P3 |
| **P5 Tokens** ✅ (T1–T6) | T1 à T7 | M | P3 |
| **F2 Front complet** ✅ | §7.4 : Preuves, pages Unity et Réglages, édition des plafonds et de la politique de modèles, galerie art ; routes `unity/*`, `settings/*`, `pixellab/balance` | M | F1, P4, P5 |
| **P7 Options** | Deuxième lane Unity en worktree, enregistrement de repro depuis le front, feel-check Desktop, mode nuit | M | F2 |

Chaque lot devient une issue du dépôt aidp, réalisée avec `dev-aidp` (Rust) ou `dev-studio` (front) :
le harness se construit avec lui-même dès P1.

## 11. Risques

| Risque | Parade |
|---|---|
| MCP Unity (`com.unity.ai.assistant` 2.21 pre-release) instable ou relais coupé | Pont fichier PlaytestDriver indépendant du MCP ; compile gate qui détecte un MCP muet et passe `blocked: unity-mcp` |
| Éditeur unique : la file se sérialise sur Unity | Revue et art hors Unity en parallèle ; lane Unity n° 2 en P7 |
| Build Rust sans MSVC | Toolchain GNU sur D: ; si une dépendance l'exige, `xwin` (CRT et SDK téléchargés dans un dossier de D:) |
| Fenêtres d'usage Claude | Budget existant + plafonds par issue + mode nuit |
| Crédits Pixellab | Budget T4, réserve du dispatcher |
| Autonomie qui merge des régressions | Merge toujours derrière ton clic ; suite PlayMode de non-régression qui grossit à chaque issue |

---

## 12. Avancement

### P0 — Socle D: (fait le 2026-11-06)

**Dépôts git locaux** (branche `main`, un commit d'import chacun, aucun remote) :

| Dépôt | Commit | Contenu |
|---|---|---|
| `D:\AIDP\src\aidp` | `5353945` puis le script de toolchain et ce plan | Sources d'aidp 7.5.19 |
| `D:\AIDP\src\aidp-e2e` | `8fec107` | Suite e2e |
| `D:\AIDP\skill` | `94e222f` | Copie du skill `ai-dev-pipeline-v7` installé |

Pas de dépôt GitHub créé : c'est ta décision. Les sources contiennent du code et des noms liés à ton
employeur ; P1 les retire, la publication GitHub se fait sans l'historique local.

**Toolchain, tout sur D:** — script relançable `D:\AIDP\src\aidp\scripts\setup-gamedev-toolchain.ps1`.

| Élément | Version | Emplacement |
|---|---|---|
| Rust | 1.95.0 `x86_64-pc-windows-gnu`, rustup 1.29.3, toolchain par défaut | `D:\DevTools\rust\{rustup,cargo}` |
| gcc (pour `ring`) | WinLibs 16.2.0 MSVCRT | `D:\DevTools\winlibs\mingw64` |
| cargo-nextest | 0.9.151 | `D:\DevTools\rust\cargo\bin` |
| Builds | — | `D:\AIDP\target` |
| Téléchargements | 286 Mo, supprimables | `D:\DevTools\_downloads` |

Aucun `~/.rustup` ni `~/.cargo` créé. Variables utilisateur posées : `RUSTUP_HOME`, `CARGO_HOME`,
`CARGO_TARGET_DIR`, `AIDP_SRC`, `AIDP_INSTALL_DIR=D:\AIDP\bin`, `AIDP_GATEWAY_DIR=D:\AIDP\gateway`,
`npm_config_cache=D:\DevTools\npm-cache` ; PATH utilisateur : `cargo\bin` et `winlibs\mingw64\bin`
en tête. `AIDP_SKILL_ROOT` est inchangé (toujours la copie de `~/.claude/skills`) : la bascule vers
`D:\AIDP\skill` est en P1, avec le lien qui garde Claude Code et le binaire sur la même copie.

**Résultats**

| Vérification | Résultat |
|---|---|
| `cargo check --workspace --all-targets` | ✅ 203 crates, 1 min 38 |
| `cargo nextest run --workspace` (unitaires) | ✅ 2 151 / 2 151 |
| `cargo fmt --all -- --check` | ✅ |
| `cargo nextest run --workspace -P integration` | ⚠️ 870 / 892, 22 échecs (ci-dessous) |
| `aidp validate --all` (skill de D:) | ✅ `dev-unity`, `review-unity`, `fix-review-unity`, `pixelart`, `dev-exemple` ; ❌ 47 templates pro dont les skills ne sont pas installés (attendu), dont `dev-aidp` et `dev-aidp-viewer` |
| `cargo clippy -D warnings` | non lancé (P1) |

**Les 22 échecs d'intégration** — aucun ne vient de la toolchain GNU ou de D: :

| Groupe | Tests | Cause | Suite |
|---|---|---|---|
| Instantané incohérent | `repo_quality_gates::every_integration_test_file_is_compiled` (`tests/flow_external_actions.rs` rattaché à aucun module), `workflow_emit::golden_tests::it_every_template_compiles_to_its_golden_workflow`, `compose::tests::it_no_doctrine_left_in_descriptions` (fixture `dev-aidp`), `claude_argv_golden_tests::it_headless_stage_session_command_line_is_stable` | Les sources du kit ont été prises sur un arbre en cours de modification : fichiers, fixtures et goldens désynchronisés. Ce sont des comparaisons de texte, indépendantes de la plateforme. | Corriger en P1 |
| Jira / CI | 14 × `ticket_delivery_flow::*`, `it_flow_actions::external_flow_branch_uses_snapshot_type_without_querying_execution_key_in_jira` | Flow `ticket-delivery` (Jira, CI, Sonar, Copilot) | Supprimés avec le flow en P1 |
| Plan tasks natifs | `native_plan_tasks::{on_item_failure_continue_lets_the_merge_see_each_task_state, on_item_failure_pause_stops_the_run_after_the_running_tasks, shared_files_are_written_only_by_the_merge}` | Politique d'échec par tâche : une tâche attendue `Failed` finit `Done`. Le test de scope voisin passe. Probablement la même désynchronisation. | Diagnostiquer en P1 |

### P1 — Nettoyage (fait le 2026-11-06)

Branche `gamedev/p1-cleanup` dans les deux dépôts, rien poussé. Travail fait par des sous-agents en
worktrees séparés, fusionnés en `--no-ff`.

**Volume retiré**

| Dépôt | Depuis l'import | Diff |
|---|---|---|
| `D:\AIDP\src\aidp` | `5353945` → `e2d00cb` | 677 fichiers, +2 460 / −36 167 lignes ; Rust des crates : 195 322 → 165 415 lignes (−15 %) |
| `D:\AIDP\skill` | `94e222f` → `6a0de1e` | 82 fichiers, +1 230 / −9 516 lignes |

**Retiré** : Jira (client, sync, write-back, release watch), CI autopilot et GitHub Actions (verbes
`gh run-*`, `workflow-run`, routes `/api/prs/ci/*`, `probe-ci`, signaux `ci_red`/`sonar_red`,
`CiSignal`), Sonar, Confluence, Teams, Mongo, SQS, déploiement, revue Copilot des PR, providers
Codex et Copilot (restent `claude` et `fake`), hôte .NET et FFI, benchmark, envoi
`/api/prs/comments/send`, automation « Étude puis dev », 91 constantes mortes, module `remedy`,
`.github/`, 45 templates et 3 flows pro. Noms de l'employeur remplacés par des valeurs neutres
(`PIXEL-n`, `cheat-*`, `pixelhood`) ; il en reste dans `docs/proofs/` (empreintes historiques).

**Changé** : flow par défaut `perso-delivery` ; tout flow lancé pour un ticket local fait avancer
son statut ; `probe_reviews` toutes les 10 min sans Jira ; skill réduit à 11 templates
(`dev-unity`, `review-unity`, `fix-review-unity`, `pixelart`, `dev-exemple`, `dev-aidp`,
`dev-studio`, `knowledge-init|read|update`, `review`) ; commande `/pipeline-setup-gamedev-v7` et
`setup-gamedev.ps1` écrits (non lancés).

**Les 4 incohérences de l'instantané (P0)** : corrigées. Goldens régénérés, test orphelin retiré,
fixture `dev-aidp` alignée ; `native_plan_tasks` venait d'un test désynchronisé
(`SEQUENTIAL_CAP = 1`), pas du code.

**Résultats**

| Vérification | Résultat |
|---|---|
| `cargo fmt --all -- --check` | ✅ |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | ✅ 0 avertissement |
| `cargo nextest run --workspace` (unitaires) | ✅ 1 868 / 1 868 |
| `cargo nextest run --workspace -P integration` | ✅ 810 / 810 |
| `cargo test --workspace --doc` | ✅ |
| `aidp validate --all` (`AIDP_SKILL_ROOT=D:\AIDP\skill`) | ✅ 11 / 11 |
| `aidp doctor` | ✅ 17 checks ; avertissements : `core.longpaths` à false, vault sans `.obsidian/` |

Binaire installé : `D:\AIDP\bin\aidp.exe` (16,8 Mo, ancien : 24,9 Mo gardé en
`aidp-7.5.19-kit.exe`).

**Restes** (pour P2/P3, sans urgence) :

- Modèle de tickets : `Source::Jira`, `JiraStatus`, blocs `jira:` des miroirs (~190 mentions) ; à
  remplacer par la source GitHub en P2.
- `OriginKind::Ci` gardé pour les sessions déjà enregistrées ; `ActiveFlowOwnership` n'est plus
  utilisé que par les tests.
- `fixtures/skill/SKILL.md` décrit encore des verbes retirés (copie figée de l'ancien skill).
- `aidp-e2e/tests/templates.rs` réclame encore le template `study`.
- `setup-gamedev.ps1` lancé le 2026-11-06 : `AIDP_SKILL_ROOT=D:\AIDP\skill`, doctor vert. Claude
  Code garde volontairement l'ancienne copie (`~/.claude/skills/ai-dev-pipeline-v7` et
  `~/.claude/commands`, 81 fichiers d'écart) : pas de jonction pour l'instant.

### P2 à P6, F0 à F2 — réalisés du 2026-11-06 au 2026-11-07

Travail parallèle par sous-agents (Sonnet, effort bas), un worktree et une cible de build par agent,
contrats partagés `D:\AIDP\contracts\gamedev-contracts.md` (C1 à C11), fusions `--no-ff` dans
`gamedev/p1-cleanup`.

| Lot | Livré |
|---|---|
| **P2** | Sync `gh issue list` → `TODO/<repo>/GH-<n>.md`, un seul label d'état `aidp:*`, routage `flow:`, bloqueurs, ordre de dequeue, triage `CA-n`, actions `gh.issue_label` / `gh.issue_comment` / `gh.pr_ready`, `POST /api/tickets/sync`, template `issue-refine`. Labels de `Litybe/PixelHood` migrés (11 créés, 5 issues relabellisées). |
| **P3** | Verrou `resource: unity-editor` (FIFO, `waiting_external`, `GET /api/unity/status`, `POST /api/unity/lock/release`) ; sorties `steps.<id>.outputs.*` depuis `result.json` ; gabarits `{{ }}` dans `with` ; pipelines `unity-implement|review|fix|test`, `art-produce|review|fix|import`, `map-produce|review|fix|test`, `doc-write|review|fix` ; recette `unity-verify.md` ; flows `game-code`, `game-art`, `game-map`, `game-doc` ; test e2e de `game-code` (provider `fake`, mock GitHub). |
| **P4** | Package `com.aidp.playtest` (`Litybe/aidp-unity`) : clavier et manette virtuels, sondes et commandes par attribut, scénarios JSON, `CompileGate`, `PlaytestBridge`, `TestGate`, pont fichier sans polling, enregistreur de repro ; sondes et commandes PixelHood (PR Litybe/PixelHood#247, draft). |
| **P5** | Coût par ticket, pied de rapport C7 dans les commentaires, plafonds par route avec gate « continuer ? », budget Pixellab, politique de modèles route × stage, mode nuit, seuil 7 j. |
| **P6** | RAG et mémoire (`aidp-memory`, SQLite FTS5 + graphe graphify, sans embeddings) : CLI `aidp rag` / `aidp memory`, routes `/api/memory` et `/api/rag`, ingestion des `memory-proposals.json` en fin de run, réindexation après merge ; pipelines qui lisent `memory context` et proposent leurs leçons. Essai réel : 1 798 fichiers, 11 871 extraits, recherche ~85 ms. |
| **F0–F2** | `aidp-studio` (`Litybe/aidp-studio`) servi par `aidp gateway serve --ui-dir` : Board, Issue (timeline, session, artefacts, preuves), Inbox, Unity, Budget, Réglages (`/api/settings/routing|budget|models`), Mémoire, Art. JS initial 102 Ko gzip, 0 polling, invalidations SSE regroupées, tas ~7 Mo. |

**Vérifications** (aidp `895f19e`) : fmt, clippy `-D warnings`, 2 169 tests unitaires, 858/858
d'intégration, doc-tests ; `validate --all` 27/27, `flow validate` des 5 flows ; front 111 tests ;
jeu compilé en batchmode sans erreur. Binaire installé dans `D:\AIDP\bin` (`aidp-p1.exe` gardé),
`setup-gamedev.ps1` et doctor verts. Daemon démarré sur le vault réel, **dispatcher en pause**,
projet PixelHood enregistré, 3 issues synchronisées.

**Publication** : dépôts privés `Litybe/aidp`, `Litybe/aidp-skill` (instantanés sans l'historique
local, via `D:\AIDP\sync-github.sh`), `Litybe/aidp-studio`, `Litybe/aidp-unity`.

**Restes** : P7 (deuxième lane Unity, repro depuis le front, mode nuit avancé) ; T7 ; serveur
`aidp mcp memory` ; rattrapage d'ingestion des runs finis daemon éteint ; embeddings (reportés) ;
`cargo hakari verify` ; sondes PlaytestDriver à éprouver dans l'éditeur ; label `aidp:need-fix` à
supprimer à la main ; les issues sans `aidp:ready` (`blocked: not-ready`) tombent dans la colonne
« Bloquées » du Board.
