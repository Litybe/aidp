---
aidp: knowledge-feature
feature: Limites de depot
slug: limites-de-depot
aliases: [deposit limits, plafonds de depot]
area: paiements
status: active
tickets: [PIXEL-1234]
mon_champ_perso: 42
updated: 2026-01-10
files: [src/Deposit/Limits.cs, src/Deposit/LimitsRepository.cs]
tags: [aidp/knowledge-feature]
---

# Limites de depot

## Ce que ca fait

Un joueur peut fixer un plafond de depot journalier, hebdomadaire ou mensuel
sur son compte. Le plafond choisi s'applique aux nouveaux depots des sa
validation ; une baisse prend effet immediatement, une hausse est retardee
de sept jours pour rester conforme aux exigences de jeu responsable.

## Regles metier

- Le plafond par defaut est fixe par `Limits.DefaultDailyCapCents` dans
  `src/Deposit/Limits.cs`.
- Une hausse de plafond est mise en file d'attente et appliquee sept jours
  plus tard par le job planifie, jamais immediatement.
- Une baisse de plafond s'applique des le prochain depot tente, sans delai.
- Le plafond mensuel ne peut jamais depasser dix fois le plafond journalier.

## Code map

- `src/Deposit/Limits.cs` — calcul et validation des plafonds, application
  du delai de hausse de sept jours.
- `src/Deposit/LimitsRepository.cs` — persistance des plafonds actifs et de
  la file d'attente des hausses en cours.
