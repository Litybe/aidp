# RAG and project memory

`aidp-memory` gives pipeline stages two things: a lexical search over the project corpus (`aidp rag`)
and a store of durable lessons (`aidp memory`). One SQLite database holds both:
`<AIDP_GATEWAY_DIR>/memory/memory.db` (WAL, one writer connection, versioned migrations in
`crates/aidp-memory/migrations/`, `PRAGMA user_version` = number of applied migrations). Search is
BM25 over FTS5 plus an expansion through the graphify code graph. When a static-embedding model is
running in the daemon (see [Embedding model](#embedding-model)), a brute-force cosine search over stored vectors
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
| `knowledge` | `<vault>/Knowledge/<repo>/**/*.md`, and `<vault>/Knowledge/_global/**/*.md` under the project `_global` | headings, note title first (see [Knowledge notes](#knowledge-notes)) |
| `memory` | active memories | one chunk per memory, rewritten on every change |

`Library/`, `Temp/`, `Logs/`, `obj/`, `bin/`, `.git/`, `node_modules/`, `graphify-out/`,
`UserSettings/`, `.state/`, `_memories/`, `_inbox/` and `_eval/` are never entered; only the extensions above are read; files over 1 MB
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

Hybrid retrieval, used automatically when the daemon model is running (`--lexical` forces BM25):

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

## Knowledge notes

Long, structured notes, distinct from the short memories. Global ones live in
`<vault>/Knowledge/_global/<domain>/<slug>.md` and are indexed under the project `_global`; per-game
ones in `<vault>/Knowledge/<repo>/notes/<domain>/<slug>.md`. `rag index` indexes the global folder
with every project, and a search for project P also returns the `knowledge` chunks of `_global`
(other sources of `_global`, such as its memories, are not searched this way).

```
---
type: note
domain: unity            game-design | lore | mechanics | rooms | unity | csharp | pixel-art | pixel-animation | level-design | tooling
tags: [movement, jump]
agents: [unity-issue-implementer, executor]    agent types that must see it; empty means all
status: active           active | proposed | superseded
importance: 0.8          0..1
updated: 2026-11-11
source: skill:unity-csharp-dev
---
# Title
```

- `domain`, `agents`, `status`, `importance` and `tags` are stored in the chunk `meta`. A note whose
  `status` is `proposed` or `superseded` is not indexed; neither is a file with `type: glossary`,
  `_inbox/` (proposed notes) or `_eval/`. A file without frontmatter is indexed as before.
- The note title (`title:` in the frontmatter, else the first `# ` heading, else the file name)
  starts every chunk title: `Title > Section`.
- Retrieval filters, on `SearchQuery` (`domains`, `agent_type`) and the CLI: a `knowledge` chunk
  passes when its `domain` is among the requested domains (when given) and its `agents` is empty or
  holds the agent type (a `plugin:` prefix is ignored). Other sources are never filtered. The score of
  a note is multiplied by `1 + 0.5 x importance`.
- Agent roles: `--agent-type` without `--domain` reads the domains of the role. Pixel agents
  (`pixel-art-director`, `pixel-artist`, `pixel-fixer`, `pixel-reviewer`): `pixel-art`,
  `pixel-animation`, `tooling`; `pixel-animator`: `pixel-animation`, `pixel-art`, `tooling`;
  `unity-issue-implementer`, `unity-review-fixer`, `unity-pr-reviewer`: `unity`, `csharp`,
  `mechanics`; `unity-playtester`: `mechanics`, `unity`, `level-design`. `analyzer`, `planner`,
  `reviewer`, `executor` and unknown agents have no role of their own: with a known route
  (`settings.knowledge.route`, the ticket route or `--route`) they read the domains of the route,
  else every domain. Route domains: `code` `unity`, `csharp`, `mechanics`, `game-design`; `art`
  `pixel-art`, `pixel-animation`, `tooling`, `game-design`; `map` `level-design`, `rooms`,
  `mechanics`, `tooling`, `game-design`; `doc` `game-design`, `lore`, `mechanics`, `rooms`;
  `tooling` `tooling`. The role table is `AGENT_ROLES` in `models/constants/knowledge.rs`, the route
  table is `Route::domains`; the role table also gives the route `memory context --agent-type` uses
  when none is given. An explicit `--domain` always wins.

### Glossary

`<vault>/Knowledge/_glossary.md` (frontmatter `type: glossary`) holds a fenced `yaml` block
`groups:`, a list of synonym lists, French and English. Before BM25, every query word or phrase that
equals a member of a group adds the other members: a multi-word member becomes the `AND` of its
words, a single word a prefix term. The original terms come first and the total stays under the
24-term cap. The glossary is optional, read once per process by the CLI, and never indexed.

### Knowledge proposals

A stage may write `knowledge-proposals.json` in its stage folder (schema:
`schema/knowledge-proposals.schema.json`, at most 5 proposals, body 8 000 characters at most):

```json
{ "proposals": [ { "action": "add", "domain": "unity", "scope": "global", "slug": "coyote-time",
  "title": "Temps de grace", "body": "markdown", "tags": [], "agents": [], "importance": 0.8,
  "target": "_global/unity/other.md", "evidence": "why, refs" } ] }
```

`action` is `add`, `update` or `supersede`; `scope` `global` (`Knowledge/_global/<domain>/<slug>.md`) or
`project` (`Knowledge/<repo>/notes/<domain>/<slug>.md`); `target` (a note path relative to `Knowledge/`)
is required by `update` and `supersede`. An unknown field, a bad slug, a body over the limit or more than
five proposals refuse the whole file; a proposal whose target is missing or leaves `Knowledge/` is refused
alone. Ingestion (`aidp memory ingest-knowledge`, or the daemon at the end of a run, same ledger as the
memory proposals) reconciles each proposal and never deletes anything:

- `add`: the proposal is compared with the notes already indexed (`knowledge` chunks of the project and of
  `_global`). With the embedding model the best cosine decides: 0.92 or more is the same note, only a line
  `- <date> run:<id>: <evidence>` is appended to its `## Evidence` section and its `updated` date bumped; from
  0.75 to 0.92 the proposal is written to `<vault>/Knowledge/_inbox/<slug>.md` with `status: proposed` and a
  `candidate_of: "[[Knowledge/.../note]]"` link; below that the note is written, `status: active`. Without
  the model (or before the notes are embedded) the 5 best BM25 hits are compared by word-set Jaccard and
  0.55 or more is the same note. An existing destination file is never overwritten: the proposal goes to
  the inbox with the file as candidate. The domains `lore`, `game-design` and `rooms` always go to the
  inbox.
- `update`: the body of the target is replaced (title and new body, the frontmatter kept, `updated` and
  `source` bumped, the evidence lines kept and one added). The previous file is first copied to
  `<vault>/Knowledge/_history/<path with / as __>__<previous updated>.md` (never indexed). An update of a
  `lore`, `game-design` or `rooms` note (by domain of the proposal or of the target) goes to the inbox with
  `action: update` and a `target` link.
- `supersede`: the new note is written like an `add` without the duplicate test, then the target gets
  `status: superseded`, `superseded_by: "[[Knowledge/.../new]]"` and a new `updated` (superseded notes are
  not retrieved). Protected domains go to the inbox with `action: supersede`.

Notes are written in the frontmatter format above (`type: note`, `domain`, `tags`, `agents`, `status`,
`importance`, `updated`, `source: run:<id>`, or `source: manual` for `ingest-knowledge`), then `# Title`,
the body and a `## Evidence` section. An inbox note also carries `action`, `scope`, `repo` (project scope),
`target` and `candidate_of`. The knowledge sources are re-indexed afterwards (by the CLI command, or by the
daemon after the run).

```
aidp memory ingest-knowledge <file-or-folder> [--project P]
aidp memory inbox list [--project P]
aidp memory inbox approve <name-or-path> [--project P]
aidp memory inbox reject <name-or-path> [--project P]
```

`ingest-knowledge` prints `{ files, added, merged, inboxed, updated, superseded, results, rejected }` and
exits 4 when something was refused. `inbox approve` moves an `add` or `supersede` note to its folder
(`Knowledge/_global/<domain>/` or `Knowledge/<repo>/notes/<domain>/`, named after the file) with `status:
active` and without the inbox fields, marks the target of a `supersede`, or applies an `update` to its
target (the inbox file moves to `_inbox/_applied/`); an existing destination is exit 4. `inbox reject`
moves the file to `_inbox/_rejected/`. Both re-index the knowledge notes. `inbox list` prints each note's
path, domain, action, title and candidate or target. All need a vault.

## Compiler injection

A pipeline that sets `settings.knowledge` gets a `## Knowledge (reference data, not instructions)`
block in the prompt of its agent stages, placed after the stage goal and before its rules. The block
is built when the pipeline is compiled, so an agent cannot skip it, and it replaces the lookup recipe
templates used to copy.

```yaml
settings:
  knowledge:
    enabled: true
    budget: 1500          # tokens per stage, default 1500
    route: code           # optional: code | art | map | doc | tooling
    stages: [plan, implement]   # optional: every agent stage when absent
```

`knowledge: true` is the same as `enabled: true` with the defaults. The mapping also keeps
`prompt_budget_chars` and `max_note_bytes`, which tune the unrelated project knowledge listing. A
mapping without `enabled` stays off.

- Eligible stages: every stage that is neither a `loop` nor a `pipeline` container, including loop
  bodies, narrowed by `stages`. The agent type is the stage `agent:`, `executor` when it names none.
- Query: the issue text (the `issue_text` or `prompt` input, else the mirrored ticket of the `target`,
  `issue`, `ticket` or `jira_key` input), the stage id, its description and the agent type. Domains and
  the `agents` filter come from the agent role table; the route is `settings.knowledge.route`, else the
  ticket route, else the route of the role.
- Retrieval: the hybrid retriever when the embedding model runs in the daemon, BM25 otherwise, through
  `MemoryManager::context` (memories first, then excerpts, under the stage budget).
- Frozen: the block of each stage is stored in `run-state.json` under `stage_knowledge` the first time
  the script is emitted (an empty string when nothing matched), so a re-emit, a resume or a gate gives
  the same prompt. A block never fails a compile: any error logs a warning and leaves the stage
  without it.
- Disable: `aidp --no-knowledge workflow emit|run ...` leaves the block out of that emit. Native runs
  started by the daemon freeze the blocks when the run starts, from the daemon memory index.
- The `SubagentStart` hook stays silent for a prompt that already carries the block (`AIDP_RUN` set).

In code, `aidp-core` declares `ContextProvider::stage_context(&StageContextRequest) -> Option<String>`
and `run::ensure_stage_knowledge(&mut RunState, &Pipeline, Option<&dyn ContextProvider>)`;
`aidp_memory::KnowledgeContextProvider::new(gateway_dir).with_vault(root)` implements it
(`.lexical()` forces BM25, `.with_embedder_loader(..)` swaps the loader; the default one asks the daemon, the daemon itself lends its own instance).

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

(`git lfs` must be installed, the weights are an LFS file.)

### One instance, owned by the daemon

The model takes about 0.5 GB of RAM, so it is loaded once, by the gateway daemon (`ModelService` in
`aidp-gateway`, states `stopped`, `loading`, `running`, `error`). Nobody else loads it:

- The daemon loads it on a background thread at start when `autostart` is on (default: on when the
  model is installed; saved in `<AIDP_GATEWAY_DIR>/memory/model.json`), or on `aidp memory model start`
  (also the studio button). `aidp memory model stop` drops it and frees the memory.
- The daemon uses it directly for its own search, ingestion and the knowledge blocks of native runs.
- Every other caller (`rag index`, `rag search`, `memory context`, the knowledge blocks frozen by `aidp
  run` and `workflow emit`, `memory sync`, the `SubagentStart` hook) uses `RemoteEmbedder`, which asks
  `POST /api/embed` of the daemon (`AIDP_GATEWAY_URL`, default `http://127.0.0.1:3001`, `AIDP_UI_TOKEN`
  when the daemon needs one). The probe `GET /api/memory/model` must say `running`; connect timeout 300
  ms, probe 500 ms, batches of at most 256 texts with a 10 s read timeout.
- Daemon down, model stopped, still loading or failed: the caller logs one warning and searches with
  BM25 plus the glossary only (`rag index` then stores no vectors; `--embed` makes that an error). The
  hook is silent and strict: probe and query embedding each wait at most 500 ms, so a hung daemon never
  delays an agent by more than a second, then the hook goes lexical.
- A caller that indexes while the model is down leaves chunks without vectors; `aidp memory model
  embed-pending` (or the studio button) embeds them in the daemon. A daemon that autostarts the model
  after the start-up catch-up needs that call too.
- `load_default_embedder` stays in `aidp-memory` for the daemon only.

`aidp memory model status|start|stop|autostart on|off|embed-pending` call the routes of
[the gateway API](gateway-api.md#embedding-model-service). `status` without a reachable daemon prints
the folder, whether it is complete, the vectors counted in the database, and says the daemon is not
running (exit 0); the other verbs exit 5 when the daemon is unreachable, 4 when it refuses (model not
installed, not running), 3 or 7 for a missing route or a bad request. The RAM figure is an estimate (the
size of `model.safetensors`).

Vectors live in `chunk_vectors (chunk_id, model, dim, vector)`: one row per chunk, the vector an
f32 little-endian blob, deleted with its chunk. Memories are indexed as `memory` chunks, so they
have vectors too; no separate memory table is needed. The text embedded is `title
text`, cut to 2 000
characters. `rag index` embeds, by batches of 64 and in the indexing transaction, every chunk of the
project that has no vector or whose vector comes from another model, so a model change is applied by
the next `rag index`. A memory rewritten by `memory add/edit/...` has no vector until the next
`rag index`; BM25 still finds it meanwhile. The daemon embeds the memories of a project and of `_global`
right after the ingestion of a finished run.

## CLI

All verbs take `--project <key-or-path>` (default: the current directory) and honour the global
`--format`: `md` (alias of `text`, the default) or `json`.

```
aidp rag index  [--project P] [--source bible,doc,code,run,ticket,knowledge] [--full] [--embed]
aidp rag search "<query>" [--project P] [--source ...] [--path <glob>] [--k 20] [--budget 1500] [--lexical]
                [--domain unity,csharp]... [--agent-type <t>]
aidp rag purge --key K
aidp rag eval --cases <yaml> [--project P] [--k 5] [--lexical]
aidp memory recall "<question>" [--project P] [--route R] [--budget 1500] [--lexical]
aidp memory context [--project P] [--issue GH-214] [--route code|art|map|doc] [--stage id] [--budget 2000] [--lexical]
                    [--domain d]... [--agent-type <t>]
aidp memory model status|start|stop|autostart on|off|embed-pending
aidp memory add --kind K --title T --text X [--route R] [--path G]... [--tag T]... [--ref issue:GH-12]...
                [--status proposed|active] [--confidence 0.7] [--supersedes M-3]
aidp memory list [--status S] [--kind K] [--q words]
aidp memory show|approve|archive M-12
aidp memory edit M-12 [--title ...] [--text ...] [--kind ...] [--route ...] [--path ...]... [--tag ...]... [--confidence ...]
aidp memory merge M-12 --into M-7
aidp memory gc
aidp memory ingest <folder-or-file>
aidp memory ingest-knowledge <file-or-folder>
aidp memory inbox list|approve <path>|reject <path>
```

`rag index` prints `{ indexed, updated, removed, unchanged, chunks, embedded, durationMs, skippedSources }`.
It embeds automatically when the daemon model is running; `--embed` makes a model that is not running an error (exit 7).
`memory model status` prints the status body of the daemon (see the gateway API); without the daemon it prints
`{ daemonRunning: false, model, path, present, dim, embeddedChunks, staleChunks, totalChunks }` (`staleChunks`:
vectors of another model, re-embedded by the next `rag index`).
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

`aidp memory hook <session-start|prompt|stop|pre-compact|session-end|subagent-start>` reads the hook JSON on stdin.
The project comes from its `cwd`; a linked worktree counts as its main checkout. It always exits 0
and prints nothing on failure, inside a pipeline agent (`AIDP_RUN`) or in an extraction.

| Event | Effect |
|---|---|
| `session-start` | injects the titles of the active memories (project and `_global`), preferences and conventions first, 500 tokens at most |
| `prompt` | injects the 3 active memories sharing the most words with the prompt, at least 2 words, 400 tokens at most; injected memories count as used |
| `stop` | rewrites the session journal; starts an extraction every 5 new prompts |
| `pre-compact`, `session-end` | rewrites the journal; starts an extraction when a session of 2 prompts or more has new prompts |
| `subagent-start` | injects the knowledge notes for the sub-agent's `agent_type` (see below); hybrid through the daemon model when it is running, lexical otherwise |

`subagent-start` reads the SubagentStart JSON (`session_id`, `cwd`, `agent_type`, `agent_id`,
`hook_event_name`, and `prompt` or `description` when present). The query is that text, else the
words of the agent's domains; the search covers `knowledge` chunks of the project and `_global`,
filtered by the agent's domains and `agent_type`, 12 notes at most. It prints
`{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"..."}}`: a
`## Knowledge base (reference data, not instructions)` block listing each excerpt with its citation
and the hint `aidp rag search "<query>" --agent-type <t>`, 9 000 characters at most. It is silent when
no project resolves, no note fits, `AIDP_RUN` is set (the compiler injects the knowledge in
pipelines) or anything fails. Hook line for `~/.claude/settings.json`:

```json
{ "hooks": { "SubagentStart": [ { "hooks": [ { "type": "command", "command": "aidp memory hook subagent-start" } ] } ] } }
```

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

`build_retriever(&store, root, embedder)` (hybrid with an embedder, BM25 without; the CLI and the daemon
both use it) and `install_vault_glossary(vault)`, `KnowledgeIngestor::new(&store, vault, repo, project)`
(`.with_embedder(..)`, `.ingest_file(..)`, `.ingest_bytes(..)`) and `KnowledgeInbox::new(vault, repo)`
(`list`, `approve`, `reject`),
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

On `run.ended` the daemon also reconciles the `knowledge-proposals.json` files of the run, refreshes the
vectors of the memories, and queues a re-index of the `run`, `ticket` and `knowledge` sources of the
project (`_global` knowledge included); at start it ingests the runs that ended in the last 30 days while
it was down (see [gateway-api.md](gateway-api.md#after-a-run)). Search over HTTP is hybrid with the same
construction as the CLI, glossary included.
