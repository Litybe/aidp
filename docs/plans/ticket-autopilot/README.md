# Plans — tickets locaux et traitement automatique des tickets

Ces plans décrivent comment doter `aidp` d'un **système de tickets local** rangé dans le vault
Obsidian et d'un **dépileur automatique** qui transforme les tickets à faire en sessions `claude`
pilotées par le démon `aidp gateway serve`, cinq projets en parallèle au plus, en respectant le
budget de l'abonnement. Le lot 8a (ingestion Jira) a été retiré.
L'état actuel est décrit dans [`docs/tickets.md`](../../tickets.md).

Lire d'abord [00-vision-et-architecture.md](00-vision-et-architecture.md) : il fixe le modèle de
données, les machines à états, les décisions par défaut et le vocabulaire que chaque lot réutilise.

## Les lots

| Ordre | Lot | Côté | Livrable | Taille |
|---|---|---|---|---|
| 1 | [Lot 7a — Jira local](01-lot-7a-jira-local.md) | CLI `aidp` (nouveau crate `aidp-tickets`) | `aidp ticket new/list/show/edit/status/route/log/reindex`, dossier `<vault>/TODO/<service>/`, notes `_TODO.md` et `_<service>.md` | M |
| 3 | [Lot 9a — Dispatcher](03-lot-9a-dispatcher.md) | démon `aidp gateway` | dépilage priorisé, 5 slots, cycle de vie ticket ↔ session ↔ run ↔ PR, `aidp ticket start` manuel, gates auto-approuvées, routes HTTP et verbes CLI | L |
| 4 | [Lot 10a — Garde-fou tokens](04-lot-10a-garde-fou-tokens.md) | démon `aidp gateway` | verdict de budget sur les fenêtres 5 h / 7 j, réservation par run, attente du reset, reprise | M |
| 5 | [Lot 7b/9b — Viewer (optionnel)](05-lot-7b-9b-viewer.md) | `aidp-viewer` | page Tickets, carte Dispatcher, formulaire de création | M |
| 6 | [Lot 11 — État du démon dans le vault (optionnel)](06-lot-11-etat-demon-dans-le-vault.md) | démon | `AIDP_GATEWAY_DIR` par défaut sous le vault, migration | S |

Dépendances : 7a → 9a → 10a. Le viewer vient après 9a. Le lot 11 est indépendant.

Convention de nommage : comme pour la passerelle, un lot **a** vit dans ce dépôt (Rust), un lot
**b** dans `aidp-viewer` (TypeScript). Les numéros continuent la série existante (le lot 6 est le
dernier livré).

## Ce que les plans ne refont pas

Ils s'appuient sur le knowledge du vault (`Knowledge/ai-dev-pipeline-v7/`, `Knowledge/aidp-viewer/`)
et sur `docs/gateway-api.md`. Les règles déjà en place ne sont pas recopiées : une session par
projet, file `queue.json`, `utilization_block`, appariement session → run, `changes::pr`, notes
`_*.md` régénérées, familles de templates `dev-*`. Chaque plan nomme ce qu'il réutilise et ce qu'il
modifie.
