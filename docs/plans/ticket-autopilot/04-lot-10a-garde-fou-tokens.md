# Lot 10a — Garde-fou tokens : budget par fenêtre, réservation, attente du reset

Règle couverte : R15. Dépend du lot 9a (le dispatcher appelle le verdict à chaque tick).

## Objectif

Le dispatcher ne démarre un développement que s'il reste assez de fenêtre 5 h **et** 7 j pour le
mener au bout, en tenant compte des sessions déjà lancées dont la consommation n'est pas encore
visible ; quand ce n'est pas le cas, il passe en attente jusqu'au reset annoncé par le CLI `claude`,
l'affiche, puis reprend seul. Une session tuée par la limite d'usage ne compte pas comme un échec
du ticket. Le verdict est explicable par `aidp ticket budget`.

## Périmètre

Dans le lot :

- `aidp_tickets::budget` : types `RateSnapshot`, `Verdict`, `BudgetStats` et la fonction pure
  `verdict()` ;
- apprentissage de la réserve par template (`.state/budget-stats.json`) ;
- mode `waiting` du dispatcher : persistance de `wait_until`, réveil à l'heure du reset,
  événements `dispatcher.waiting` / `dispatcher.resumed` ;
- détection d'une fin de session due à la limite d'usage → `rate_limited: true` pour
  `lifecycle::on_ended` ;
- `aidp ticket budget`, `GET /api/dispatcher/budget`, section Budget de `_TODO.md` ;
- docs : `docs/tickets.md` (budget), `docs/gateway-api.md`, note `usage-guardrail` du knowledge du
  viewer à rafraîchir au prochain `knowledge-update`.

Hors lot : modifier le seuil des automations (`utilization_block` reste tel quel, le comportement
existant des automations ne change pas) ; toute utilisation de l'API facturée.

## Conception

### Entrées

Le démon possède déjà tout : `SessionManager::rate_limit()` (dernier instantané, réamorcé au
démarrage depuis `rate-limit.jsonl` seulement si la fenêtre 5 h n'est pas expirée),
`rate_limit_history()`, et par session `usage` (tokens) et `started_at`. Le crate `aidp-tickets`
ne connaît pas ces types : le dispatcher les mappe vers :

```rust
pub struct Window { pub utilization: f64, pub resets_at_s: i64 }
pub struct RateSnapshot { pub status: String, pub is_using_overage: bool, pub five_hour: Option<Window>, pub seven_day: Option<Window>, pub observed_at_ms: i64 }
```

### Le verdict, pur

```rust
pub enum Verdict {
  Allow        { max_launches: usize, headroom_5h: f64, headroom_7d: f64, reserve_5h: f64 },
  Conservative { max_launches: usize /* 1 */, why: &'static str },
  Wait         { until_ms: i64, window: WindowKind, utilization: f64, limit: f64 },
  Paused       { why: String },
}
pub fn verdict(snap: Option<&RateSnapshot>, running_dispatcher: usize, unreflected: usize, stats: &BudgetStats, cfg: &BudgetCfg, now_ms: i64) -> Verdict
```

Règles, dans l'ordre :

1. `snap` absent, ou sa fenêtre 5 h expirée (`resets_at_s * 1000 <= now_ms`) → selon
   `unknown_policy` : `conservative` → `Conservative { max_launches: 1 }` (une seule session à la
   fois tant qu'aucun instantané frais n'est arrivé ; la première session en produit un dès son
   premier tour) ; `allow` → traiter comme utilisation 0 ; `block` → `Paused`.
2. `status == "rejected"` ou `is_using_overage` → `Wait` jusqu'au reset 5 h (ou `Paused` si pas
   de fenêtre).
3. Réserve : `reserve_5h = stats.reserve_for(template_mix)` si `learn` et ≥ 5 échantillons, sinon
   `cfg.reserve_per_run_5h` ; idem 7 j.
4. Projection : `u5 = five_hour.utilization + unreflected × reserve_5h` où `unreflected` = sessions
   du dispatcher démarrées **après** `observed_at_ms` (leur consommation n'est pas encore dans
   l'instantané). Même chose pour 7 j.
5. `n5 = floor((max_5h − u5) / reserve_5h)`, `n7 = floor((max_7d − u7) / reserve_7d)`, bornés à
   0. `max_launches = min(n5, n7)`.
6. `max_launches == 0` : si `u5 ≥ max_5h` ou `n5 == 0` → `Wait` sur la fenêtre 5 h
   (`until = five_hour.resets_at_s × 1000 + 60 000`) ; sinon `Wait` sur la fenêtre 7 j. La fenêtre
   la plus contraignante gagne.
7. Sinon `Allow`.

Une réserve inférieure à la marge restante mais insuffisante pour un run entier produit donc une
attente : c'est le comportement demandé (ne pas lancer un développement qu'on ne pourra pas finir).

### Apprentissage de la réserve

`.state/budget-stats.json` :

```json
{ "templates": { "dev-csharp": { "samples": 7, "p50_delta_5h": 0.06, "p75_delta_5h": 0.09, "p75_delta_7d": 0.015 } },
  "updated_at": "…" }
```

À chaque `session.ended` d'une session du dispatcher : `Δ5h` = utilisation du premier instantané
observé **après** la fin moins celle du dernier instantané observé **avant** le début, divisée par
le nombre moyen de sessions du dispatcher concurrentes pendant le run (approximation assumée :
la fenêtre est partagée). Rejeter l'échantillon si un reset est tombé entre les deux
instantanés (`resets_at` différent). Recalcul des percentiles sur les 30 derniers échantillons.
`reserve_for(mix)` = max des p75 des templates que le plan s'apprête à lancer.

### Attente et reprise dans le dispatcher

- `Verdict::Wait` → `state.mode = waiting`, `wait_until`, événement `dispatcher.waiting
  {until, window, utilization, limit}` (émis une fois par entrée en attente, pas à chaque tick),
  `_TODO.md` : « En attente du reset (fenêtre 5 h, 87 % ≥ 80 %) jusqu'à 14:35 ».
- Pendant l'attente : aucun lancement, y compris sur `session.ended` ; les sessions en cours
  continuent ; `aidp ticket start --force` reste possible et journalisé `forced-over-budget`.
- La boucle dort `min(tick_seconds, wait_until − now)`. À l'échéance : nouveau verdict ; si toujours
  `Wait` (instantané pas encore rafraîchi), reprogrammer ; un instantané postérieur au reset qui
  montre une utilisation basse fait sortir de l'attente → `dispatcher.resumed`, tick immédiat.
- `Conservative` : `max_launches = 1`, raison affichée « utilisation inconnue jusqu'au premier
  instantané ».

### Fin de session due à la limite

`rate_limited = true` si l'un de : dernier instantané `status == "rejected"` observé pendant la
session ; `lastText`/dernier événement d'erreur de la session contenant l'un des motifs
(`rate limit`, `usage limit`, `hit your limit`, `429`, `rate_limit`) ; `exit_code != 0` **et**
utilisation 5 h ≥ `max_5h` à la fin. Le ticket revient en `todo` sans tentative consommée et le
dispatcher passe en `Wait` (§7.2 de la vision). Les motifs vivent dans `config.yaml`
(`budget.rate_limit_patterns`) pour suivre les changements de libellé du CLI.

### Exposition

```
aidp ticket budget [--format]        → verdict, fenêtres (utilisation, reset en heure locale), réserve, sessions non reflétées, stats
GET /api/dispatcher/budget           → même objet ; inclus aussi dans GET /api/dispatcher
```

`_TODO.md` : tableau « Budget » (fenêtre, utilisation, seuil, reset, verdict).

## Étapes

1. `budget.rs` : types, `verdict()`, `BudgetStats` (charge, met à jour, percentiles) ; tests.
2. Mapping `RateLimitInfo → RateSnapshot` et calcul de `unreflected` dans le dispatcher ;
   remplacement du stub du lot 9a.
3. Mode `waiting` : persistance, sommeil borné, événements, sortie d'attente.
4. Détection `rate_limited` ; branchement dans `on_ended` ; motifs configurables.
5. Apprentissage : capture des instantanés avant/après, échantillons, recalcul.
6. `aidp ticket budget`, route HTTP, section Budget de `_TODO.md`.
7. Docs et `cargo test --workspace`.

## Tests

- `verdict()` : pas d'instantané → Conservative ; instantané expiré → Conservative ; `rejected` →
  Wait 5 h ; 5 h à 0,75 avec réserve 0,10 et seuil 0,80 → Wait (pas assez pour un run) ; 5 h à
  0,50 → Allow max 3 ; 7 j à 0,89 seuil 0,90 réserve 0,02 → Wait 7 j même si 5 h est basse ;
  `unreflected = 2` réduit `max_launches` de 2 ; `until` = reset + 60 s ; horloge injectée.
- `BudgetStats` : percentiles sur ≤ 30 échantillons ; échantillon rejeté quand un reset est tombé ;
  `reserve_for` prend le max du mix.
- Détection `rate_limited` : chaque motif ; faux positif évité quand utilisation basse et texte
  neutre.
- Dispatcher : `dispatcher.waiting` émis une fois ; aucun `start()` pendant l'attente même sur
  `session.ended` ; reprise au reset.

## Critères d'acceptation

- Fenêtre 5 h à 85 % : `aidp ticket budget` répond `Wait` avec l'heure de reset ; `_TODO.md`
  l'affiche ; aucun ticket ne démarre ; un ticket démarre dans la minute qui suit un instantané
  post-reset.
- Démon fraîchement démarré sans historique vivant : une seule session démarre ; la deuxième
  attend le premier `rate_limit_event`.
- Une session interrompue par la limite d'usage laisse son ticket en `todo` avec `attempts`
  inchangé et le journal porte `rate-limited`.
- Le comportement des automations existantes (seuil 0,9 sur 5 h, run `skipped`) est inchangé.

## Risques et points d'attention

- **Sémantique de la fenêtre** : le CLI remonte `utilization` et `resetsAt` ; on suppose un reset
  franc à `resetsAt`. Si la fenêtre glisse plutôt qu'elle ne se réinitialise, l'attente est
  simplement plus longue que nécessaire, jamais plus courte.
- **Réserve par défaut** : 0,10 est un a priori ; l'apprentissage corrige. Le garder visible dans
  `aidp ticket budget` pour que l'utilisateur l'ajuste.
- **Instantané partagé** : plusieurs sessions concurrentes brouillent l'attribution ; la division
  par la concurrence moyenne est assumée et documentée comme approximation.
- **Ne jamais contourner par une clé API** : décision structurante du projet, rappelée dans la doc.

## Dépendances et taille

Dépend du lot 9a. Taille **M**.
