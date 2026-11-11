---
type: note
domain: unity
tags: [movement, jump]
agents: [unity-issue-implementer, executor]
status: active
importance: 0.9
updated: 2026-11-11
source: skill:unity-csharp-dev
---

# Temps de grace au saut

## Principe

Le coyote window laisse sauter quelques frames apres avoir quitte le bord d'une plateforme.
Implementer avec un timer reinitialise au sol et decompte en l'air.
