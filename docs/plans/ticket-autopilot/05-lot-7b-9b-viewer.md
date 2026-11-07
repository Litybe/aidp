# Lot 7b/9b — Viewer (optionnel) : page Tickets, carte Dispatcher, création de ticket

Non demandé explicitement : la demande est centrée sur le CLI (R9). Ce lot rend visible dans
`aidp-viewer` ce que les lots 7a à 10a exposent déjà par HTTP, sans y ajouter de règle (principe du
viewer : afficher, ne pas décider). Dépend du lot 9a ; le budget (10a) s'affiche s'il est présent.

## Objectif

Une page **Tickets** montre le tableau par statut et par service dans l'ordre de dépilage, l'état
du dispatcher (slots, mode, attente de reset, dernier plan et raisons de skip), et permet les
mêmes actions que le CLI : créer un ticket local, démarrer maintenant, annuler, router, forcer un
statut. La barre latérale porte un raccourci et un compteur « en attente / en cours / en revue ».

## Périmètre

- `src/shared/types.ts` : `Ticket`, `TicketStatus`, `TicketSource`, `DispatcherState`,
  `BudgetVerdict`, `SkipReason` ; `OriginKind` gagne `dispatcher` ; `Trigger`/`Action` inchangés.
- `src/server/routes/gateway.ts` : deux lignes de pass-through, `/tickets(/*)` et
  `/dispatcher(/*)`, comme pour `/usage` et `/queue` au lot 4b (sans elles la requête tombe sur le
  catch-all SPA).
- `src/client/pages/Tickets.tsx` : colonnes `À faire`, `En file`, `En cours`, `En revue`,
  `Bloqués`, filtres service/source, recherche, tiroir de détail (frontmatter + corps rendu par
  `MarkdownRenderer`, liens vers la session, le run vault et la PR), actions.
- `src/client/components/DispatcherCard.tsx` : slots `n/5`, mode (`normal`, `en attente jusqu'à
  HH:MM`, `en pause`), boutons pause/reprise/tick, jauges 5 h / 7 j réutilisant celles de la barre
  latérale, verdict en clair.
- Formulaire « Nouveau ticket » : service (liste des projets du démon), résumé, type, priorité,
  template (pré-rempli depuis `routing.yaml` via la réponse du démon), corps pré-rempli avec le
  gabarit Objectif / Impacts / Critères.
- `useLiveReload` sur les préfixes `ticket.`, `dispatcher.`, `tickets.` ; centre de notifications :
  `ticket.reviewed` (« PR ouverte pour <KEY> ») et `dispatcher.waiting`.
- Palette de commandes (lot 6b) : « Tickets », « Nouveau ticket », « Lancer un ticket… ».
- `docs/ORCHESTRATOR.md` : ligne Control UI et état du chantier.

Hors lot : édition du corps d'un ticket (Obsidian reste l'éditeur), toute logique d'ordre ou de
budget côté TypeScript.

## Règles à respecter

- Le viewer n'ordonne jamais les tickets lui-même : il affiche l'ordre renvoyé par
  `GET /api/tickets`.
- Aucun seuil de budget recodé côté client (même règle que la note `usage-guardrail`).
- Aucune action mutante déclenchée au montage ou au survol (règle 2 de `ORCHESTRATOR.md`).
- Les listes sont plafonnées côté client ; les compteurs viennent de `GET /api/dispatcher`.
- `origin.kind === 'dispatcher'` s'affiche avec un badge dédié dans Sessions et le tiroir de
  session renvoie vers le ticket (`origin.ref`).

## Étapes

1. Types partagés et pass-through Express ; `npm run check` vert.
2. `api.ts` : `tickets.list/get/create/start/cancel/route/status/sync`, `dispatcher.get/plan/pause/resume/tick/budget`.
3. Page Tickets (lecture seule d'abord), puis actions, puis formulaire.
4. `DispatcherCard` + intégration barre latérale et page Tickets.
5. Notifications et palette.
6. Tests Vitest sur l'adaptateur `Ticket → colonnes` et sur le rendu conditionnel du mode
   `waiting` (les seuls morceaux avec un peu de logique).
7. `docs/ORCHESTRATOR.md`.

## Critères d'acceptation

- Le scénario de recette du lot 9a est suivi en direct dans la page Tickets sans rechargement.
- « Démarrer maintenant » sur un ticket produit le même effet que `aidp ticket start`.
- Un démon antérieur au lot 9a (404 sur `/api/tickets`) laisse la page afficher un état vide
  expliqué et désactive les actions avec infobulle, jamais masquées (règle des lots précédents).

## Taille

**M**, un run `dev-aidp-viewer` en `track: standard`.
