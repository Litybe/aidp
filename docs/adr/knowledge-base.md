# Knowledge base: hybrid retrieval, knowledge notes, guaranteed injection, post-run learning

## Status

Accepted. Extends `docs/memory.md`; does not replace the memory store.

## Context

`aidp-memory` gives stages a BM25 search and short memories (600 chars). Four gaps limit pipelines:
retrieval is lexical only (no synonyms, no semantics); injection relies on the C10 recipe copied in
templates, which an agent may skip, and runs once per run; domain knowledge (Unity, C#, pixel art,
pixel animation, metroidvania maps, game design) lives in skills outside the corpus; after a run only
review/fix/test stages propose memories, and the `run`, `ticket` and `knowledge` sources are not
reindexed.

## Decision

1. **Knowledge notes.** Long, structured markdown notes, distinct from memories:
   - global: `<vault>/Knowledge/_global/<domain>/<slug>.md`;
   - per game: `<vault>/Knowledge/<repo>/notes/<domain>/<slug>.md`.
   - `Domain` enum: `game-design`, `lore`, `mechanics`, `rooms`, `unity`, `csharp`, `pixel-art`,
     `pixel-animation`, `level-design`, `tooling`.
   - Frontmatter: `type: note`, `domain`, `tags: []`, `agents: []` (agent types that must see it;
     empty means all), `status: active|proposed|superseded`, `superseded_by`, `importance` (0..1),
     `updated`, `source`.
   - Indexed as source `knowledge`; `_global` is indexed under the `_global` project and searched
     with every project. `domain`, `agents`, `status`, `importance` go into chunk `meta`.
     `proposed` and `superseded` notes are not retrieved.
   - `<vault>/Knowledge/_inbox/` holds proposed notes; never indexed.
2. **Glossary.** `<vault>/Knowledge/_glossary.md` (YAML block of synonym groups, FR/EN, e.g.
   `[coyote time, coyote, grace jump, saut de grâce]`). Query terms are expanded with their group
   before BM25.
3. **Hybrid retrieval.** `Embedder` trait with a `model2vec-rs` implementation (static embeddings,
   pure Rust, no network at runtime; model `potion-multilingual-128M` in
   `<AIDP_GATEWAY_DIR>/memory/models/<name>/`). Vectors are stored per chunk (`chunk_vectors`,
   f32 LE blob, model id). `HybridRetriever` fuses BM25 and brute-force cosine by RRF (k = 60),
   then applies source weights, `importance` and the existing MMR and packing. No model on disk:
   BM25 only, with one `tracing::warn!`.
4. **Guaranteed injection, three layers.**
   - Compiler: `aidp-core` declares a `ContextProvider` trait; `aidp-memory` implements it. When a
     pipeline enables `settings.knowledge`, every agent stage prompt gets a `## Knowledge` block
     built at compile time from the stage's own query (stage id, description, agent type, issue
     text), filtered by the agent's domains. Replaces the copied C10 text.
   - Hook: `aidp memory hook subagent-start` reads the SubagentStart JSON on stdin, builds a context
     for `agent_type` (role → domains table), prints `additionalContext` under 9 000 characters.
     Silent when the prompt already carries the compiler block (`AIDP_RUN` set).
   - CLI: `aidp rag search` stays the on-demand deeper lookup. An MCP server brings no extra
     guarantee and stays deferred.
5. **Post-run learning.**
   - `run.ended` reindexes `run`, `ticket`, `knowledge` for the project and `_global`; the daemon
     catches up runs that ended while it was down.
   - Stages may write `knowledge-proposals.json`: note upserts `{ domain, slug, title, body, tags,
     agents, importance, action: add|update|supersede, target? }`. Ingest reconciles: cosine ≥ 0.92
     against an existing note → merge evidence; 0.75–0.92 → `_inbox` for review; below → add.
     Domains `lore`, `game-design`, `rooms` always go to `_inbox`.
   - Memory dedup uses cosine when vectors exist, Jaccard otherwise.
   - Skill templates end each flow with a `learn` stage gated on "something changed".

## Consequences

New dependency `model2vec-rs` (hakari regenerated, `cargo tree -i tokio` stays empty). Retrieval
quality is measured with `aidp rag eval`; cases are added for every domain. Templates drop the copied
C10 text once they enable `settings.knowledge` (skill raises `min_aidp_version`).

## As built

- CLI: `aidp memory inbox list|approve <name>|reject <name>` (reject moves to `_inbox/_rejected/`,
  nothing is deleted); `aidp memory ingest-knowledge <file|dir>` ingests a `knowledge-proposals.json`;
  `aidp memory model status` reports the embedding model. All need a vault.
- An update or supersede that rewrites a note first copies the previous version to
  `<vault>/Knowledge/_history/` (never indexed).
- Catch-up: at start the daemon indexes the runs that ended while it was down, found through the
  `native-run.json` of each run.
- Protected domains `lore`, `game-design`, `rooms`: any add, update or supersede goes to the inbox.
- Domain narrowing: a specific role (pixel, unity) keeps its domain table; a generic agent
  (`analyzer`, `planner`, `reviewer`, `executor`, unknown) reads the domains of its route (`code`,
  `art`, `map`, `doc`, `tooling`), else every domain. `Route::domains` holds the route table; the
  non-existent map agents are gone from the role table.
- Embedding: `model2vec-rs` with the `wasm` feature, so the build needs no C++ toolchain or
  libstdc++.
