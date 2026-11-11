# RAG and project memory

`aidp-memory` gives pipeline stages two things: a lexical search over the project corpus (`aidp rag`)
and a store of durable lessons (`aidp memory`). One SQLite database holds both:
`<AIDP_GATEWAY_DIR>/memory/memory.db` (WAL, one writer connection, versioned migrations in
`crates/aidp-memory/migrations/`, `PRAGMA user_version` = number of applied migrations). Search is
BM25 over FTS5 plus an expansion through the graphify code graph. When a static-embedding model is
installed (see [Embedding model](#embedding-model)), a brute-force cosine search over stored vectors
is fused with it (hybrid retrieval); without a model, search stays BM25 only.

## Model

A chunk is `{ id, project, source, path, line_start, line_end, title, text, hash, mtime, meta }`.
The `project` column keys every row, so one database serves all projects. A linked git worktree
counts as its main checkout. The project key is the registered name in the daemon (`projects.json`
under the gateway directory, matched on `path` or `testCheckout`), else the folder name of the main
checkout.

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

A query longer than 24 terms keeps its most telling ones: terms of its first line count 3 more,
identifiers (inner capital, underscore) 2 more, and each repetition 1 more. A mirrored ticket is
searched as its `summary` then its body; its frontmatter is dropped.

Hybrid retrieval, used automatically when the model is installed (`--lexical` forces BM25):

- BM25 and graph candidates are ranked as below, and the vector side ranks every stored vector of
  the same project (and `--source` filter) by cosine similarity with the query (at least 60
  candidates, `5 x k`; 400 with `--path`; cosine under 0.05 is dropped).
- The two ranked lists are fused by reciprocal rank fusion, `1 / (60 + rank)` per list; a hit found
  by both is `via: hybrid`, a hit found by vectors only is `via: vector`.
- The source weights below then multiply the fused score, followed by MMR and packing.

1. BM25 candidates (at least 60, `5 x k`; 400 with a `--path` filter), each score multiplied by its
   source weight: `memory` 1.5, `bible` 1.2, `doc` and `code` 1, `knowledge` 0.9, `ticket` 0.7,
   `run` 0.4, and 0.5 more for a session journal (`/_journal/`).
2. Graph expansion when `<project>/graphify-out/graph.json` exists: symbols found in the top code
   chunks, and symbols named like a query term, pull in their `calls`, `inherits`, `implements` and
   `references` neighbours (both directions). A neighbour scores half of its seed (`via: graph`);
   at most 30 are added.
3. MMR (`lambda` 0.7, word-set Jaccard) keeps `k` results without near duplicates.
4. Packing: hits are kept in order while their Markdown rendering fits the budget (tokens are
   characters / 4). A hit that does not fit is skipped so a smaller one can enter; if the first hit
   alone is too big it is cut. Each excerpt carries its citation `path:L<start>-L<end>`.

## Embedding model

Vectors come from a [Model2Vec](https://github.com/MinishLab/model2vec) static-embedding model: the
tokens of a text are looked up in an embedding matrix, mean-pooled and L2-normalised, in pure Rust
(`model2vec-rs`, loaded from a local folder, no network, no `tokio`). The default model is
`potion-multilingual-128M`, read from `<AIDP_GATEWAY_DIR>/memory/models/potion-multilingual-128M/`,
which must hold `model.safetensors`, `tokenizer.json` and `config.json`. Code never downloads it;
install it once by hand:

```
git clone https://huggingface.co/minishlab/potion-multilingual-128M <AIDP_GATEWAY_DIR>/memory/models/potion-multilingual-128M
```

(`git lfs` must be installed, the weights are an LFS file.) `aidp memory model status` prints the
folder, whether it is complete, the vector size and the number of chunks embedded. Without the
model, every command logs one warning and uses BM25 only.

Vectors live in `chunk_vectors (chunk_id, model, dim, vector)`: one row per chunk, the vector an
f32 little-endian blob, deleted with its chunk. Memories are indexed as `memory` chunks, so they
have vectors too; no separate memory table is needed. The text embedded is `title
text`, cut to 2 000
characters. `rag index` embeds, by batches of 64 and in the indexing transaction, every chunk of the
project that has no vector or whose vector comes from another model, so a model change is applied by
the next `rag index`. A memory rewritten by `memory add/edit/...` has no vector until the next
`rag index`; BM25 still finds it meanwhile.

## CLI

All verbs take `--project <key-or-path>` (default: the current directory) and honour the global
`--format`: `md` (alias of `text`, the default) or `json`.

```
aidp rag index  [--project P] [--source bible,doc,code,run,ticket,knowledge] [--full] [--embed]
aidp rag search "<query>" [--project P] [--source ...] [--path <glob>] [--k 20] [--budget 1500] [--lexical]
aidp rag purge --key K
aidp rag eval --cases <yaml> [--project P] [--k 5] [--lexical]
aidp memory recall "<question>" [--project P] [--route R] [--budget 1500] [--lexical]
aidp memory context [--project P] [--issue GH-214] [--route code|art|map|doc] [--stage id] [--budget 2000] [--lexical]
aidp memory model status
aidp memory add --kind K --title T --text X [--route R] [--path G]... [--tag T]... [--ref issue:GH-12]...
                [--status proposed|active] [--confidence 0.7] [--supersedes M-3]
aidp memory list [--status S] [--kind K] [--q words]
aidp memory show|approve|archive M-12
aidp memory edit M-12 [--title ...] [--text ...] [--kind ...] [--route ...] [--path ...]... [--tag ...]... [--confidence ...]
aidp memory merge M-12 --into M-7
aidp memory gc
aidp memory ingest <folder-or-file>
```

`rag index` prints `{ indexed, updated, removed, unchanged, chunks, embedded, durationMs, skippedSources }`.
It embeds automatically when the model is installed; `--embed` makes a missing model an error (exit 7).
`memory model status` prints `{ model, path, present, dim, embeddedChunks, staleChunks, totalChunks }`
(`staleChunks`: vectors of another model, re-embedded by the next `rag index`).
`run`, `ticket` and `knowledge` need a vault (`--vault`, `AIDP_VAULT`, or auto-detected); without
one they are listed in `skippedSources`. `rag search --format json` prints
`[{ id, source, path, lines, citation, title, score, via, text }]`.

`rag purge` deletes the chunks and file rows of an orphan key, then compacts the database, and prints
`{ chunks, files, compacted }`. `compacted` is false when the database was busy. A registered project
or a key holding memories is refused (exit 4); a key with no index is exit 3.

`rag eval` reads a YAML list of `{ query | issue, expect: [path prefix], sources }`. A `query` case
searches the index; an `issue` case builds the `memory context` block of that ticket (with its route,
without counting uses) and ranks its memories (`memory/M-n`) then its excerpts. It prints
`{ cases, k, recall, mrr, misses }`. The PixelHood cases live in `<vault>/Knowledge/PixelHood/_eval/`.

`memory recall` is `memory context` for a free question.

`memory context` prints the block a stage injects: the active memories of the project and
`_global` whose scope matches (route equal, unset, or `tooling`; without `--route`, the route of the
ticket; path globs matched against `Assets/...` or `Docs/...` paths found in the issue text,
or, without such paths, a text match), best first, using at most 40 % of the budget; then the best
excerpts for the issue text under the rest. With a query, a memory that shares no word with it is
left out unless one of its path globs matches; the best memory is shortened rather than skipped when
it alone exceeds the memory budget. A proposal without route takes the route of its first issue's
ticket, and the daemon runs `memory gc` for the project after each run. The issue text is read from the ticket mirror
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
- Automatic activation: a proposal from `memory ingest` or a finished run is active at once when
  `confidence >= 0.7`. Any other proposal needs `evidence >= 2`, references to at least two
  different issues, and `confidence >= 0.8`, or a human approval.
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

Agents also write undocumented shapes, so a proposal holding `type`, `name`, `description`, `body`,
`lesson`, `fact`, `why`, `how_to_apply` or `evidence` is folded first. Title comes from `title`,
`description`, then `name`; text from `text`, `body`, `lesson`, then `fact`, with `why`,
`how_to_apply` and `evidence` appended as lines; both are shortened to their limits. `type` gives the
kind when it is a kind (`user` is `preference`, `project` and `reference` are `fact`, anything else
`lesson`). Issue and run references are added from `GH-<n>` and run id segments of the file path.

A file is refused as a whole: it creates nothing. The daemon records a file as ingested only when it
was accepted, so a refused file is tried again if its run ends again.

## Classification and vault notes

A memory has a project (or `_global`, read by every project), a route (`code`, `art`, `map`, `doc`,
`tooling` or none), a kind (`decision`, `convention`, `lesson`, `gotcha`, `preference`, `fact`,
`incident`), path globs, tags and references.

Each memory is one Markdown note, the source of truth for what a human edits:

```
<vault>/Knowledge/<repo>/_memories/<route or general>/<kind>/M-12-<slug>.md
<vault>/Knowledge/<repo>/_memories/_archive/M-3-<slug>.md     archived and superseded
```

The frontmatter holds `id`, `kind`, `status`, `route`, `paths`, `tags`, `refs` (`type:id`),
`confidence`, `evidence`, `supersedes`, `created`, `updated`; the body is `# title` then the text.
`_memories` is never indexed as `knowledge`.

`aidp memory sync` compares each note to the hash recorded at its last write:

| Note | Effect |
|---|---|
| edited | applied to the database; references are unioned; `evidence` and dates stay the database's |
| no `id` | creates an active memory (`kind` defaults to `fact`) and receives an id |
| unknown `id` | recreates the memory at this id: the database can be rebuilt from the notes |
| deleted | the memory is archived, unless the notes folder is empty or missing |
| unreadable or invalid | reported and left untouched |

Every writing `aidp memory` verb syncs after it and `memory context` syncs before it. The daemon
syncs after a run's ingestion and after each change through `/api/memory`. A sync failure is logged
and never blocks. `memory sync` prints `{ exported, imported, created, archived, rejected }` and
exits 4 when a note was refused.

## Claude Code sessions

`aidp memory hook <session-start|prompt|stop|pre-compact|session-end>` reads the hook JSON on stdin.
The project comes from its `cwd`; a linked worktree counts as its main checkout. It always exits 0
and prints nothing on failure, inside a pipeline agent (`AIDP_RUN`) or in an extraction.

| Event | Effect |
|---|---|
| `session-start` | injects the titles of the active memories (project and `_global`), preferences and conventions first, 500 tokens at most |
| `prompt` | injects the 3 active memories sharing the most words with the prompt, at least 2 words, 400 tokens at most; injected memories count as used |
| `stop` | rewrites the session journal; starts an extraction every 5 new prompts |
| `pre-compact`, `session-end` | rewrites the journal; starts an extraction when a session of 2 prompts or more has new prompts |

A journal is `<vault>/Knowledge/<repo>/_journal/YYYY-MM/YYYY-MM-DD-<session>.md`: the prompts, edited
files, commits and last reply read from the transcript, without tool output. It is written for a
session in a git repository with at least two prompts, an edit or a commit, never for a pipeline
agent. Journals are indexed as `knowledge`.

`aidp memory distill --journal P --session ID` runs detached: it gives the journal to `claude -p
--model haiku` with no tools, ingests the answer as proposals with a `session` reference, and appends
its outcome to `<gateway>/memory/distill.log`. `<gateway>/memory/distilled.json` records the prompt
count already extracted per session. Proposals follow the rules above: active at `confidence >= 0.7`.
Injected blocks say they are reference data, not instructions.

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
`HybridRetriever::new(&store, Some(project_root), Box<dyn Embedder>)` implementing it too,
`Indexer::new(&store).with_embedder(&embedder)`, the `Embedder` trait (`model_id`, `dim`, `embed`) with
`Model2VecEmbedder::load(dir)`, `load_default_embedder(gateway_dir)` (`None` plus one warning when the model is
absent) and `model_status`,
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
