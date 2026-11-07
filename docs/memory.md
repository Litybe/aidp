# RAG and project memory

`aidp-memory` gives pipeline stages two things: a lexical search over the project corpus (`aidp rag`)
and a store of durable lessons (`aidp memory`). One SQLite database holds both:
`<AIDP_GATEWAY_DIR>/memory/memory.db` (WAL, one writer connection, versioned migrations in
`crates/aidp-memory/migrations/`, `PRAGMA user_version` = number of applied migrations). Search is
BM25 over FTS5 plus an expansion through the graphify code graph. There are no embeddings in v1; the
`Retriever` trait is the seam where a vector index can be added without touching the CLI.

## Model

A chunk is `{ id, project, source, path, line_start, line_end, title, text, hash, mtime, meta }`.
The `project` column keys every row, so one database serves all projects. The project key is its
registered name in the daemon (`projects.json` under the gateway directory), else the folder name.

| Source | Files | Cut |
|---|---|---|
| `bible` | `<project>/.claude/BIBLE/**/*.md` | by heading, title is the heading path (`A > B`) |
| `doc` | `<project>/Docs/**/*.md`, `README.md` | same |
| `code` | `<project>/Assets/**/*.cs` | one chunk per method; a type keeps its fields and properties in `Type` / `Type (members)` chunks |
| `run` | `<vault>/Pipelines/<repo>/**` (`md`, `json`, `txt`, `yaml`) | headings, else by size |
| `ticket` | `<vault>/TODO/<repo>/*.md` | headings |
| `knowledge` | `<vault>/Knowledge/<repo>/**/*.md` | headings |
| `memory` | active memories | one chunk per memory, rewritten on every change |

`Library/`, `Temp/`, `Logs/`, `obj/`, `bin/`, `.git/`, `node_modules/`, `graphify-out/`,
`UserSettings/` and `.state/` are never entered; only the extensions above are read; files over 1 MB
are skipped. Markdown chunks hold at most 1 200 characters with a 150-character overlap; C# chunks at
most 1 800. The C# cutter is a brace and signature scanner (no syntax tree): it skips comments and
strings, keeps doc comments and attributes with their member, and names nested members `Outer.Inner.Method`.
Code chunks also store an `idents` column: camelCase and snake_case identifiers split into lowercase
parts plus the joined form, so `apply damage` finds `ApplyDamage`.

Indexing is incremental. A file is skipped when its size and mtime match the stored row; when only
the mtime changed, its SHA-256 decides. Changed files are re-chunked, vanished files lose their
chunks, all in one transaction per source. Files are read one at a time.

## Search

`unicode61 remove_diacritics 2` tokenizer with prefix indexes (2 to 4). A query is folded (accents,
case), split on camelCase, stripped of French and English stopwords and plural marks, and sent as an
`OR` of prefix terms. Column weights: title 5, identifiers 2, path 1.5, text 1.

1. BM25 candidates (at least 60, `5 x k`; 400 with a `--path` filter).
2. Graph expansion when `<project>/graphify-out/graph.json` exists: symbols found in the top code
   chunks, and symbols named like a query term, pull in their `calls`, `inherits`, `implements` and
   `references` neighbours (both directions). A neighbour scores half of its seed (`via: graph`);
   at most 30 are added.
3. MMR (`lambda` 0.7, word-set Jaccard) keeps `k` results without near duplicates.
4. Packing: hits are kept in order while their Markdown rendering fits the budget (tokens are
   characters / 4). A hit that does not fit is skipped so a smaller one can enter; if the first hit
   alone is too big it is cut. Each excerpt carries its citation `path:L<start>-L<end>`.

## CLI

All verbs take `--project <key-or-path>` (default: the current directory) and honour the global
`--format`: `md` (alias of `text`, the default) or `json`.

```
aidp rag index  [--project P] [--source bible,doc,code,run,ticket,knowledge] [--full]
aidp rag search "<query>" [--project P] [--source ...] [--path <glob>] [--k 20] [--budget 1500]
aidp memory context [--project P] [--issue GH-214] [--route code|art|map|doc] [--stage id] [--budget 2000]
aidp memory add --kind K --title T --text X [--route R] [--path G]... [--tag T]... [--ref issue:GH-12]...
                [--status proposed|active] [--confidence 0.7] [--supersedes M-3]
aidp memory list [--status S] [--kind K] [--q words]
aidp memory show|approve|archive M-12
aidp memory edit M-12 [--title ...] [--text ...] [--kind ...] [--route ...] [--path ...]... [--tag ...]... [--confidence ...]
aidp memory merge M-12 --into M-7
aidp memory gc
aidp memory ingest <folder-or-file>
```

`rag index` prints `{ indexed, updated, removed, unchanged, chunks, durationMs, skippedSources }`.
`run`, `ticket` and `knowledge` need a vault (`--vault`, `AIDP_VAULT`, or auto-detected); without
one they are listed in `skippedSources`. `rag search --format json` prints
`[{ id, source, path, lines, citation, title, score, via, text }]`.

`memory context` prints the block a stage injects: the active memories whose scope matches (route
equal or unset; path globs matched against `Assets/...` or `Docs/...` paths found in the issue text,
or, without such paths, a text match), best first, using at most 40 % of the budget; then the best
excerpts for the issue text under the rest. The issue text is read from the ticket mirror
`<vault>/TODO/<repo>/GH-<n>.md`. Every injected memory gets `uses + 1` and a new `lastUsedAt`.

Exit codes follow `exit-codes.md`: 0 success, 3 unknown memory or project path, 4 refused transition
(approving an archived memory, merging into a dead one) or `memory ingest` with at least one refused
file, 7 invalid input (unknown kind or status, text over 600 characters, bad glob, bad id), 1 for
database or I/O failures.

## Life cycle

```
proposed --approve / auto--> active --60 days unused (gc)--> stale --approve--> active
   any --archive--> archived          active --newer decision--> superseded
```

- A proposal close to an existing live memory (title and text Jaccard similarity >= 0.55 among the
  BM25 candidates) does not create a duplicate: `evidence += 1`, references are unioned, confidence
  becomes the maximum.
- Automatic activation: `evidence >= 2`, references to at least two different issues, and
  `confidence >= 0.8`. Otherwise a human approves.
- `supersedes`: when the new memory becomes active, the old one becomes `superseded`.
- `merge A --into B`: evidence adds up, references, paths and tags are unioned, `A` is superseded.
- `gc` marks `stale` every active memory whose latest of `lastUsedAt` and `updatedAt` is older than
  60 days. Nothing is ever deleted.
- Limits: title 120 characters, text 600, confidence in 0..1.

Pipelines write at most 5 proposals in `memory-proposals.json` in their stage folder:

```json
{ "proposals": [ { "kind": "lesson", "title": "...", "text": "...",
  "scope": { "route": "code", "paths": ["Assets/**"], "tags": [] },
  "refs": [ { "type": "issue", "id": "GH-214" } ], "confidence": 0.8 } ] }
```

`memory ingest` validates strictly (unknown fields, kinds, lengths, more than 5 proposals refuse the
whole file; other files still go through) and creates `proposed` memories through the same
de-duplication.

## Cost

Memory use is a few hundred bytes of SQLite per memory. The index of a Unity project is dominated by
its C# and Markdown: roughly one chunk per method or section; the database is about the size of the
indexed text again (FTS5 external content with prefix indexes). Indexing streams files one at a time,
so memory stays flat; an unchanged re-index only stats files. A query is one FTS5 statement plus a
few point lookups, and loads `graph.json` once per process (a few MB). `memory context` and
`rag search` add roughly `budget` tokens to a stage prompt, never more.

## Rust API

`MemoryStore::open_in(gateway_dir)`, `Indexer::new(&store).index(&ProjectRef, &IndexOptions)`,
`Bm25Retriever::new(&store, Some(project_root))` implementing `Retriever::search(&SearchQuery)`,
`pack(hits, budget)` and `render_markdown(&packed)`, `MemoryManager::new(&store)` with `add`, `get`,
`list`, `edit`, `approve`, `archive`, `merge`, `gc`, `context(&ContextRequest, &dyn Retriever)` and
`ingest`, `resolve_project(arg, gateway_dir, cwd)`. The crate reads no environment variable and
parses no arguments; the caller passes the gateway directory.

## Daemon integration

The gateway opens the store once (`<state dir>/memory/memory.db`, one writer connection shared by
all requests) and exposes it under `/api/memory` and `/api/rag`, see
[gateway-api.md](gateway-api.md#project-memory-and-rag). It also ingests the
`memory-proposals.json` files of a finished native run (once per file and content, with the issue,
pull request and run references completed) and, after a successful `gh.merge`, re-indexes the
`code`, `doc` and `bible` sources on a low-cost background thread, one pass at a time. The thread
does not lower its OS priority: the pass is incremental (stat only for unchanged files) and
streams one file at a time. Ingestion and re-indexing failures are logged and never block a run or
a flow.
