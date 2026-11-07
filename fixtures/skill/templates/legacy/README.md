# templates/legacy/

Archive de templates retires du parc actif. Un template n'est jamais efface : quand il n'a plus sa
place parmi les pipelines decouvrables, il est deplace ici tel quel (`git mv`, contenu inchange —
une archive est un instantane, pas une migration).

Ce dossier est exclu de la decouverte (`pipeline::ARCHIVE_DIR` / `is_archived` dans
`crates/aidp-core/src/pipeline.rs` du depot aidp) : `aidp pipeline list` ne le voit pas, et son `id:` ne peut
donc pas entrer en collision avec un template actif. Il reste toutefois chargeable par chemin
explicite, par exemple :

```
aidp validate templates/legacy/dev-tests-csharp.yaml
```

## Arrivees du lot T

| Fichier | Regle | Raison |
|---|---|---|
| `dev-tests-csharp.yaml` | R-ARC-1 | Orphelin : zero reference `uses:` dans tout le parc de templates. Supplante par `dev-tests-csharp-split`, qui livre la meme phase de tests C# en deux stages composables (`tests_unit` ‖ `tests_reqnroll`) au lieu du fan-out par label de `dev-tests-csharp`. |
