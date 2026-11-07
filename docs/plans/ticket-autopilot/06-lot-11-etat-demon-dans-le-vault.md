# Lot 11 — État du démon dans le vault (optionnel)

Règle concernée : R12 dans sa lecture la plus large. Les lots 7a à 10a mettent **toutes** les
données du système de tickets dans le vault (`TODO/`). Reste l'état préexistant du démon
(`~/.aidp/gateway/` : `projects.json`, `automations.json`, `automation-runs.jsonl`,
`queue.json`, `rate-limit.jsonl`, `sessions/*.json` + `*.events.jsonl`), qui
n'est pas dans le vault aujourd'hui. Ce lot le déplace. Indépendant des autres.

## Objectif

Quand un vault est configuré, le démon range son état sous `<vault>/.gateway/` par défaut ; une
commande de migration déplace l'existant ; `AIDP_GATEWAY_DIR` garde la priorité.

## Conception

- Précédence du répertoire d'état : `--state-dir` → `AIDP_GATEWAY_DIR` → `<vault>/.gateway/` si
  un vault se résout → `~/.aidp/gateway/` (comportement actuel, conservé sans vault).
- Le dossier commence par `.` : Obsidian l'ignore, aucun index `_*.md` n'y est généré, et les
  JSONL de sessions ne polluent pas la recherche du vault.
- `aidp gateway migrate-state [--from <dir>] [--to <dir>] [--dry-run]` : démon arrêté exigé
  (`GET /api/health` doit échouer), copie fichier par fichier avec vérification de taille, écrit un
  `MIGRATED-FROM.txt` dans la destination et un `MIGRATED-TO.txt` dans la source ; ne supprime
  rien (l'utilisateur supprime la source à la main).
- `aidp doctor` : nouvelle vérification « répertoire d'état du démon » avec `fix` prêt à l'emploi.
- `aidp-viewer` : `docker-compose.yml` monte le vault (déjà nécessaire pour les artefacts) ; le
  volume `config/gateway` devient inutile ; `docs/ORCHESTRATOR.md` et `Knowledge/aidp-viewer/data`
  à mettre à jour (les fichiers `config/gateway/*` présents dans l'arbre de travail du viewer
  ressemblent à de l'état réel et ne devraient plus être versionnés).

## Étapes

1. Résolution du répertoire d'état avec la nouvelle précédence ; test unitaire de la précédence.
2. `migrate-state` + tests sur répertoires temporaires (copie, refus si démon vivant, `--dry-run`).
3. `doctor`.
4. Docs (`gateway-api.md` §Decisions, `operations`), viewer (compose, ORCHESTRATOR).

## Risques

- **Synchronisation du vault** (Obsidian Sync, OneDrive) : des JSONL réécrits en continu peuvent
  saturer un outil de synchro ; documenter l'exclusion de `.gateway/` ou laisser
  `AIDP_GATEWAY_DIR` hors vault dans ce cas. C'est la raison pour laquelle ce lot est optionnel.
- **Chemins Windows/WSL** : le vault est référencé en `C:/…` ; le démon compilé en musl dans
  l'image Docker le voit via un montage. Vérifier `docker-compose.yml`.

## Taille

**S**.
