# Local Tickets

`ticket` names three different things in this repo. Confuse them and you edit, or wait on, the
wrong thing.

1. **`pipeline::Placement::Ticket` / `ticket_root()`** — the cross-run folder a v7 pipeline shares
   across every run of the same feature (`<vault>/Pipelines/<repo>/<feature>/`), used to persist
   research (discovery/study) stage output. Unrelated to this document; nothing in this lot renames
   it.
2. **`FeatureNote.tickets`** — the list of ticket keys in a knowledge note's frontmatter (`tickets:
   [LOC-12, LOC-15]`), used to find a feature note by ticket id. Unrelated to this document.
3. **A local ticket** — what this document is about: a Markdown note under
   `<vault>/TODO/<service>/<KEY>.md`, created and transitioned only by `aidp ticket …`.
   `aidp ticket` starts no session and runs no daemon. Where this document says "ticket" from here
   on, it means this third sense.

## Scope of this lot

`aidp ticket` and the `TODO/` folder it owns: the model, the note format, the verbs, the status
machine, the dequeue order, `config.yaml`, `routing.yaml`'s `rules:` and `repos:` sections, the
resident dispatcher ([The dispatcher](#the-dispatcher)), and its token-budget guard
([Budget](#budget)). What remains is the viewer's Tickets page — see
[Out of scope](#out-of-scope).

## The `TODO/` folder

```
<vault>/TODO/
  _TODO.md                     generated: board (todo/in-progress/in-review/blocked/done, per-service counts, dispatcher placeholder)
  config.yaml                  hand-edited: slots, thresholds, budget, local prefix
  routing.yaml                 hand-edited: rules: (ticket → service) + repos: (service → template/track)
  unrouted/                    tickets whose service could not be inferred
  <service>/                   one folder per service
    _<service>.md               generated: service index
    LOC-12.md                   local ticket (source: local)
    archive/                    done/cancelled tickets past archive_after_days
  .state/                      hidden from Obsidian, machine only
    counter.json                next local number
    ticket-events.jsonl         append-only transition journal
```

Two rules to leave with:

- **A note whose name starts with `_` is generated.** `aidp` rewrites `_TODO.md` and every
  `_<service>.md` on each command that touches a ticket. Never hand-edit one; never rely on hand
  edits surviving.
- **A status is only ever written by `aidp`.** Never by hand in Obsidian, never by an agent session.
  A session that needs a ticket state change asks a human, or waits for a future lot's dispatcher —
  it does not open the note and change `status:` itself.

## Note format

### Frontmatter

```yaml
---
aidp: ticket
key: LOC-12
source: local                # local, or github for a mirror of an issue (then a `github:` block, `route`, `flow`, `milestone`)
type: Story                  # Story | Task | Bug | Spike
summary: "Add the dash to the player controller"
status: todo                 # see Status model below
priority: High                # Highest | High | Medium | Low | Lowest
priority_rank: 2               # 1..5, derived
service: pixelhood
template: dev-unity            # resolved from routing.yaml, overridable
track: standard
labels: [player]
components: []
epic: LOC-10
created: 2026-09-10T08:12:00Z
updated: 2026-09-10T09:40:00Z
autopilot:
  adopted: false               # true when an existing PR was adopted instead of launching a session
  attempts: 0
  queued_at: null
  started_at: null
  session_id: null
  run_id: null
  branch: null
  pr: null                     # { url, number, state }
  blocked_reason: null
tags: [aidp/ticket, aidp/ticket/todo, service/pixelhood, source/local]
---
```

### Body

```markdown
# LOC-12 — Add the dash to the player controller

## 🎯 Objectif
…
## 🔧 Impacts techniques
…
## ✅ Critères d'acceptation
- …

## Notes locales
(never touched by aidp)
```

The frontmatter is rewritten on every save, but **unknown keys are preserved** — a field this lot
does not know about round-trips unchanged. **`## Notes locales` survives byte-for-byte**: `aidp`
never touches it. Notes are UTF-8 without a BOM, with `\n` line endings; Windows
must not introduce `\r\n`.

`priority_rank` and `tags` are **derived and stored**, not read as input. Editing `priority:` by
hand in Obsidian changes nothing else until the next `aidp ticket` command recomputes and rewrites
`priority_rank` — until then it is stale.

`key:` and `service:` are used as filesystem path components, so each must be a single safe path component:
not empty or whitespace-only; not starting with `_` or `.` (which also rules out `.`
and `..`); containing no `/`, `\` or `:`; containing no control character; and containing none of
`[`, `]`, `|`, `#` or a backtick, since these values are interpolated unescaped into the
`[[wikilink]]` and the Markdown table row that the generated `_TODO.md` and `_<service>.md` notes
build — one of these characters would let the value break out of the link or the table row. The
value must also not be a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`,
`LPT1`–`LPT9`, case-insensitively, with or without an extension), which would otherwise fail much
later with a raw OS error. The store refuses a note whose `key:` or `service:` breaks any of
this — whether the value comes from the command line or is read back from a hand-edited
frontmatter — and skips it from every listing with one line on stderr rather than crashing
anything. This is what stops a hand-edited note from making `aidp ticket` write outside the vault;
there is still no charset allowlist beyond the characters above, so a key of the usual
`ABC-123` shape keeps working. The rule is enforced on read as well as on write: a
note already on disk whose `key:`/`service:` carries one of these refused characters stops
appearing in listings until its frontmatter is fixed.

The body template follows vision §5.1: French, level-2 headings (`## 🎯 Objectif`), three
sections — Objective, Technical impacts, Acceptance criteria.

## Verbs

| Verb | What it does | Flags |
|---|---|---|
| `new` | Creates a local ticket. Body defaults to the vision §5.1 template (see [Note format](#note-format)) when none is given; prints `{key, path, warnings}` (see [Index warnings](#index-warnings)). | `--service <repo>` `--summary "<text>"` `--type story\|task\|bug\|spike` `--priority highest\|high\|medium\|low\|lowest` `--body-file <md>` \| `--body "<md>"` \| `--stdin` `--labels a,b` `--template <id>` `--track quick\|standard\|deep` `--epic <KEY>` |
| `list` | Lists tickets in dequeue order; `text` is one line per ticket. | `--status s1,s2` `--service X` `--source github\|local` `--reason <blocked_reason>` `--archived` `--format text\|json-compact\|json-pretty` |
| `show` | Prints frontmatter + body; `json` returns the Ticket plus body. | `<KEY>` `--format` |
| `render` | Prints the body exactly as it will be passed as `prompt` to a future dispatched run — a way to check it now. | `<KEY>` |
| `sync` | Mirrors the open issues of `github.repos` into `TODO/<repo>/GH-<n>.md`, then writes the state labels back; prints the report. See [GitHub issues as tickets](#github-issues-as-tickets). | `--source github` `--repo owner/name` `--limit N` `--dry-run` |
| `label` | Replaces the single `aidp:*` state label of a mirror's issue. | `<KEY> <STATE>` |
| `comment` | Posts a comment on a mirror's issue. | `<KEY>` `--body-file F` \| `--body T` `--cost-footer` |
| `adopt` | Attaches an existing PR to a `todo` or `queued` ticket and moves it to `in-review` without launching a session; actor `human`, `autopilot.adopted: true`. Reaches the daemon. | `<KEY> --pr <url\|number>` `--url` |
| `edit` | Changes frontmatter fields only; `--service` delegates to `route`. | `<KEY>` `--priority` `--service` `--template` `--track` `--summary` `--labels` `--epic` |
| `status` | Manual transition, actor `human`, journalled. Refuses an illegal transition explicitly. | `<KEY> <status>` `--reason "<text>"` `--force` |
| `route` | Assigns or reassigns the service (and optionally the template). | `<KEY> --service <repo>` `--template <id>` `--from-unrouted` |
| `cancel` | Cancels a ticket. | `<KEY>` `--reason` |
| `log` | Reads the transition journal, one ticket or all. | `[<KEY>]` `--limit 50` `--format` |
| `reindex` | Regenerates `_TODO.md` and every `_<service>.md`, sweeps `archive/`; prints `{archived, warnings}` (see [Index warnings](#index-warnings)). | — |
| `start` | Pushes a ticket to the front (`manual_priority`) and launches it as soon as its project is free; reaches the daemon. | `<KEY>` `--force` `--service` `--template` `--track` `--url` |
| `plan` | Shows what the dispatcher would launch. Answers locally with an empty running set, so its `ProjectBusy`/`NoSlot` verdicts are optimistic compared to the daemon's; `aidp gateway dispatcher status` is the daemon-aware view. | `--format` |
| `dispatch` | Asks the daemon to tick now, or to show its plan. | `--dry-run` `--url` |
| `budget` | Explains the token-budget verdict the daemon computed on its last tick: the 5h and 7d windows (utilization and reset), the thresholds in force, the reserve per run (learned or configured), the number of running sessions not yet reflected in the snapshot, and the resulting verdict — see [Budget](#budget). Reaches the daemon, like `start` and `dispatch`. | `--url` `--format` |

`--reason` on `list` filters to tickets whose `autopilot.blocked_reason` matches — for example
`aidp ticket list --status blocked --reason needs-routing` lists exactly the unrouted tickets.
`--from-unrouted` on `route` re-evaluates every note under `TODO/unrouted/` in one batch instead of
naming a single `<KEY>`, so it takes no `<KEY>` argument; each note that now matches a rule in
`routing.yaml` is moved to its resolved service and returned to `todo`.

#### Index warnings

`new` and `reindex` always print a `warnings` array; it is `[]` normally. A non-empty `warnings`
means an index note (`_TODO.md` or a `_<service>.md`) could not be written. Each entry names the
file, and stderr carries the same error. The exit code stays 0: the ticket itself was saved.

### Flag validation

`--type`, `--priority`, `--source`, `--track` and the `<STATUS>` argument of `status` are checked by clap at parse time and fail with exit 2. `--priority` still accepts `P1`..`P5` and `1`..`5` as aliases of `highest`..`lowest`. `route` takes either `<KEY> --service <repo>` or `--from-unrouted`; clap refuses both together. `start --service`, `--template` and `--track` are forwarded to the daemon and applied.

### Examples

Create a local ticket:

```bash
aidp ticket new --service pixelhood --summary "Dash" --type story --priority medium
# {"key": "LOC-1", "path": ".../TODO/pixelhood/LOC-1.md", "warnings": []}
```

List in dequeue order:

```bash
aidp ticket list --format text
# LOC-1       todo    High    pixelhood              Dash
# LOC-2       todo    Medium  release-tools          Write the changelog script [no-template]
```

Columns are `key`, `status`, `priority`, `service`, `summary`, tab-separated (shown above with
aligned spaces for readability). `[no-template]` is appended when the ticket has no `template`
resolved.

Block one with a reason:

```bash
aidp ticket status LOC-1 blocked --reason "attente specs"
# journals the transition, rewrites _TODO.md and _pixelhood.md
```

## Status model

| Status | Meaning | Who writes it |
|---|---|---|
| `todo` | Candidate for dequeue. | `new`, retry, dispatcher |
| `queued` | Held by the dispatcher's plan, waiting on a slot, a free project, or budget. | dispatcher |
| `in-progress` | A session is running (`autopilot.session_id`). | dispatcher |
| `in-review` | The session ended and a PR exists — done for the autopilot; the slot is freed. | dispatcher |
| `done` | The PR merged, or an adopted PR was already merged. | dispatcher |
| `blocked` | Needs a human. `blocked_reason` ∈ `needs-routing`, `failed`, `gate:<stage>`, `no-pr`, `pr-closed`, `cancelled-by-user`. | dispatcher |
| `cancelled` | Withdrawn. | human |

**`failed` is not a status.** It is one of the `autopilot.blocked_reason` values. A retry is the
transition `in-progress → todo` with `autopilot.attempts` incremented — never a `failed` status of
its own.

Legal transitions, from the vision's state machine: `todo → queued → in-progress`; `in-progress →
in-review` (PR detected) or back to `todo` (retry, `attempts < max`) or to `blocked: failed`
(`attempts` exhausted); `in-review → done` (PR merged) or `blocked: pr-closed`; any status →
`cancelled` at any time via `aidp ticket cancel` or the human `status` verb. A manual `status` call
that would leave `in-progress` is refused without `--force`.

`dispatcher` is now a real actor, journalled on every transition the daemon makes. On a session
event or a merge probe it applies exactly one of:

| Trigger | Transition |
|---|---|
| session ended `Done`, a PR is found | `in-progress → in-review` |
| session ended `Done`, no PR is found | `in-progress → blocked: no-pr` |
| session ended `Failed`, `attempts < max_attempts` | `in-progress → todo` (attempt consumed) |
| session ended `Failed`, `attempts` at the ceiling | `in-progress → blocked: failed` |
| session ended `Failed`, rate-limited | `in-progress → todo` (attempt **not** consumed) |
| session ended `Cancelled` | `in-progress → blocked: cancelled-by-user` |
| daemon restart, session orphaned, a PR exists | `in-progress → in-review` |
| daemon restart, session orphaned, no PR | `in-progress → todo` |
| an unapproved gate | `in-progress → blocked: gate:<stage>` — the session keeps running and keeps its slot |
| the gate is answered | `blocked: gate:<stage> → in-progress` |
| `gh pr view` reports the PR merged | `in-review → done` |
| `gh pr view` reports the PR closed unmerged | `in-review → blocked: pr-closed` |
| launch-time PR lookup finds an open PR | `todo\|queued → in-review` (actor `dispatcher`, `autopilot.adopted: true`; no session started) |
| launch-time PR lookup finds a merged PR | `todo\|queued → done` (actor `dispatcher`, `autopilot.adopted: true`) |
| `aidp ticket adopt <KEY> --pr <url\|number>` | `todo\|queued → in-review` (actor `human`, `adopt`) |

The launch-time PR lookup runs once per candidate, right before launch, never on every tick: it
looks for a PR on `feat/<KEY>`, then falls back to a `gh pr list` search by key in the title. A PR
closed without merging is ignored.

Every transition is journalled to `.state/ticket-events.jsonl` as `{ts, key, from, to, actor, reason,
session_id?}`. Four actors are written today: `flow` (see below), `human` (`aidp ticket status`, `aidp ticket cancel`), `cli`
(every other verb that changes status — `new`, `route`, `edit` when it moves service), and
`dispatcher` (every transition in the table above, plus `todo → queued → in-progress` on launch).
The actor `sync` is the GitHub sync.

`log`'s `--limit` is served by a bounded read from the end of the journal, so it stays cheap as the
file grows. The file itself is never rotated nor truncated: it keeps its exact path,
`.state/ticket-events.jsonl`, and its one-JSON-object-per-line format.

### The `flow` actor

When `dispatcher.launch` is `flow`, or a delivery is started with `POST /api/tickets/:key/flow`, the flow engine writes the ticket status under the actor `flow`, journalled like every other actor (reason `flow-progress`). Every flow run started for the ticket does so, whatever its flow id. It sets:

| Moment | Transition |
|---|---|
| Start of the run | `todo` or `queued` to `in-progress` |
| The step `ready` is done (PR marked ready) | `in-progress` to `in-review` |
| The step `merge` is done (PR merged) | `in-review` to `done` |
| A step escalates | any active status to `blocked` |

The ticket frontmatter carries `autopilot.flow: <flow id>` (`perso-delivery` by default) for the life of the run. A Bug ticket works on `fix/<KEY>`; every other ticket on `feat/<KEY>`. A ticket with an active flow is never launched as a session. `dispatcher.launch: flow` in `config.yaml` (default `session`) makes the dispatcher start the default flow (`perso-delivery`) instead of a session. See [flows.md](flows.md#ticket-delivery-definition).

## Dequeue order

Sort key: `(manual, milestone, priority, source, issue number, created, key)`, all ascending.

1. A ticket with `autopilot.manual_priority == Some(true)` (`aidp ticket start`, or the front's
   promote) sorts before every other.
2. Then the nearest milestone: a ticket in a milestone before one without; the earlier due date
   first (an undated milestone after the dated ones), then the milestone title.
3. Then the priority; the labels `prio:high` and `prio:low` of a GitHub mirror set it.
4. Then a GitHub mirror before a local ticket, then the issue number, then FIFO on `created`,
   then the key as the final tie-break.

## `config.yaml`

`<vault>/TODO/config.yaml`, not secret, hand-edited, read on every command:

```yaml
dispatcher:
  slots: 3          # dev lane: flows in worktrees running at once
  test_slots: 1     # test lane: test flows at once per project (main checkout, Unity editor)
  tick_seconds: 60
  permission_mode: acceptEdits
  auto_approve_gates: [pr]       # "*" for all; other gates block the ticket (gate:<stage>)
  max_attempts: 2
  gate_timeout_hours: 12
  paused: false
  launch: session                # session | flow: how a queued ticket is launched
  max_worktrees_per_project: 3   # live worktrees per project (see Worktrees and the cap)
  worktree_copy:                 # untracked files copied into each new worktree
    - .claude/settings.local.json
    - ".aidp/*.json"
budget:
  max_utilization_5h: 0.80
  max_utilization_7d: 0.90
  reserve_per_run_5h: 0.10       # estimated cost of one dev run on the 5h window
  reserve_per_run_7d: 0.02
  unknown_policy: conservative   # no snapshot yet: run one session at a time
  learn: true                    # refine the reserve from observed deltas
  rate_limit_patterns:           # substrings that mark a session killed by the usage limit
    - "rate limit"
    - "usage limit"
    - "hit your limit"
    - "429"
    - "rate_limit"
local:
  key_prefix: LOC
  archive_after_days: 30         # done/cancelled moved to <service>/archive/
doctor:                          # the pipeline doctor, see doctor.md
  enabled: true
  stuck_minutes: 10              # a running step with no live run for this long is stuck
  max_attempts_per_ticket: 2     # investigations per ticket over 24 h
  # harness_ticket: GH-1          # issue under which bugs shared by several tickets are raised
  max_budget_raise: 0            # times the doctor may lift a ticket's token ceiling per 24 h (0: never)
  prune_worktree_hours: 24       # hours a terminal or blocked flow keeps its worktree once the cap is full
  # fallback_model: sonnet        # model a run resumes with after the provider stayed overloaded
```

`doctor` is read by the daemon's doctor on every pass ([doctor.md](doctor.md)). `local.*` is read by every verb (key allocation, the archive sweep). `dispatcher` is read by the
resident dispatcher on every tick ([The dispatcher](#the-dispatcher)), and so is `budget`, to compute
the token-budget verdict (see [Budget](#budget)). A leftover `jira:` section is ignored. The `github:` section is described in [GitHub issues as tickets](#github-issues-as-tickets). No secret
lives in the vault.

## `routing.yaml`

`<vault>/TODO/routing.yaml` has three sections:

```yaml
defaults:
  track: standard

rules:                                   # evaluated in order, first match wins
  - match: { component: pixelhood }
    service: pixelhood
  - match: { label: art }
    service: pixelhood-art
  - match: { key_prefix: LOC, summary_regex: "(?i)\\b(map|room)\\b" }
    service: pixelhood-map
  - match: { epic: LOC-10 }
    service: pixelhood

repos:
  pixelhood:      { template: dev-unity }
  pixelhood-art:  { template: pixelart, track: standard }
  pixelhood-map:  { template: dev-unity }
  aidp:           { template: dev-aidp }
```

**`defaults:`** currently holds one key, `track` — the fallback used when a matched rule's `repos:`
entry sets none.

**`rules:`** is a sequence evaluated in order; the first rule whose `match:` holds wins, and later
rules are never consulted. Each rule's `match:` may combine any of eight criteria, all ANDed together:

| Criterion | Matches when |
|---|---|
| `component` | one of the ticket's `components` equals the value |
| `label` | one of the ticket's `labels` equals the value, ignoring case; a trailing `*` is a prefix (`pixel:*`) |
| `labels` | a list: any of them matches a label, same rules as `label` |
| `keywords` | a list: any keyword appears as a whole word, ignoring case, in the summary or the body |
| `epic` | the ticket's parent epic key equals the value |
| `key_prefix` | the ticket key starts with `<value>-` |
| `summary_regex` | the summary matches the regex |
| `issuetype` | the ticket type equals the value, case-insensitively |

A `match:` with no criteria at all never matches anything — it is skipped rather than swallowing
every ticket. In the example above, a ticket in component `pixelhood` routes to `pixelhood`
regardless of the rules below it; a ticket with no component but labelled `art` falls to the second
rule; a `LOC-*` ticket whose summary mentions "map" or "room" falls to the third; and a ticket under
epic `LOC-10` falls to the fourth.

**`repos:`** resolves the winning rule's `service` to a `template`, a `track`, and for a game
backlog a `flow` and a `route` family. `new` and `route` also read `template` directly when
neither flag is passed explicitly.

| `repos:` key | Effect |
|---|---|
| `template` | the pipeline a session runs |
| `flow` | the delivery flow the dispatcher starts for the ticket instead of a session (`POST /api/tickets/:key/flow` starts the same one); a ticket with a `flow` needs no `template` |
| `track` | the run depth: `quick`, `standard`, `deep`, or `bug` (the failing repro test first) |
| `route` | the family the mirror shows (`code`, `art`, `map`, `doc`; the entry name when absent), carried by the API as `route` |

In `rules:`, `route:` is a synonym of `service:`. For a GitHub mirror the rule names the route: the
mirror stays filed under its repository's service. An issue with no label that matches any
rule falls to the `keywords` rules; if none matches, it is `unrouted` and waits on `aidp:triage`.
A complete example for PixelHood is [examples/pixelhood-routing.yaml](examples/pixelhood-routing.yaml).

### Tolerance contract

Routing tolerates a broken `rules:` entry rather than losing routing altogether — this is a
deliberate design decision, not an oversight:

- A rule that is malformed (wrong shape, e.g. a `match:` that is a bare string instead of a map), one
  that names a criterion `routing.yaml` doesn't recognize, or one whose `summary_regex` fails to
  compile is reported on stderr and **dropped individually**. The rest of `rules:` — and `repos:` in
  particular — is never lost along with it. Only a whole-file YAML syntax error falls back to an
  empty `Routing` (no `rules:`, no `repos:`).
- A rule whose `service` has no entry in `repos:` still routes: the ticket gets that `service`,
  `template: null`, and a warning naming the service is printed on stderr.
- A ticket created for a service with no `repos:` entry at all still gets `template: null`, and
  `list` flags it (`[no-template]`) — the dispatcher skips it as `NoTemplate` on every tick, the same
  way it skips an unrouted ticket as `NeedsRouting`.

## GitHub issues as tickets

The backlog of a game lives in GitHub issues. `aidp ticket sync --source github` (and the daemon,
every `github.sync_minutes`) mirrors every open issue of the repositories named in
`github.repos` into `TODO/<service>/GH-<n>.md`. The note body is the issue title and body; the
section `## Notes locales` is yours and survives every update.

```yaml
# <vault>/TODO/config.yaml
github:
  repos:                         # a bare "owner/name", or a mapping
    - Litybe/PixelHood           # service = the repository name (PixelHood), key prefix GH
    # - { repo: Litybe/Other, service: other, key_prefix: OT }
  sync_minutes: 10               # daemon rhythm; 0 = only the manual sync
  limit: 200                     # open issues read per repository and sync (gh issue list --limit)
  native_dependencies: true      # also read GitHub's blocked_by dependencies (one call per issue)
  ready_required: true           # a complete issue waits for aidp:ready (blocked: not-ready)
  label_prefix: "aidp:"          # state label family: one label at a time on an issue
  status_labels:                 # ticket status (or blocked:<reason>) -> state label
    todo: "aidp:ready"
    queued: "aidp:queued"
    in-progress: "aidp:in-progress"
    in-review: "aidp:awaiting-merge"
    done: "aidp:done"
    blocked: "aidp:blocked"
    "blocked:needs-refine": "aidp:triage"
    "blocked:needs-routing": "aidp:triage"
    "blocked:skip": "aidp:skip"
  phase_labels: ["aidp:review", "aidp:fixing", "aidp:testing"]   # set by a flow while in-progress
```

Reads and writes go through the `gh` CLI (`gh issue list --json number,title,body,labels,milestone,state,url,createdAt,updatedAt`, `gh api` for the rest); a missing `gh` login is reported per repository in the sync report.

**Sync outcome.** One report is printed (and kept in `TODO/.state/github-sync.json`):

| Field | Meaning |
|---|---|
| `created` / `updated` / `unchanged` | mirrors written, rewritten (fully while not started, only the metadata once started), or already current |
| `cancelled` | a mirror whose issue was closed without being delivered becomes `cancelled`; one in `in-review` becomes `done` |
| `unrouted` | issues no rule of `routing.yaml` matched: filed under `unrouted/`, `blocked: needs-routing`, label `aidp:triage` |
| `triaged` | complete issues without a `CA-n` criterion: `blocked: needs-refine`, label `aidp:triage` |
| `labels_pushed` | state labels written back |
| `labels_seen` | state labels met on GitHub, to tune `status_labels` |
| `errors` | per-issue problems; the run goes on |

`--dry-run` computes the report and writes nothing, neither a note nor a label.

**State labels (`aidp:*`).** A ticket status maps to exactly one label through `github.status_labels`
(a `blocked:<reason>` key wins over the bare `blocked`); every status change replaces the label,
so at most one `aidp:*` label sits on an issue. A flow sets the phase labels itself with the
action `gh.issue_label`. The label is also an intent: when you change it on GitHub since the last
sync, the ticket follows (`aidp:ready` makes it `todo`, `aidp:skip` blocks it for good, any other
state hands it over: `blocked: label:aidp:done`, so an issue worked outside aidp is never
dispatched again). The label the sync wrote itself never overrides the local status.

| Label | Ticket status |
|---|---|
| `aidp:triage` | `blocked: needs-refine` (no `CA-n`) or `blocked: needs-routing` |
| `aidp:ready` | `todo` |
| `aidp:queued` | `queued` |
| `aidp:in-progress`, `aidp:review`, `aidp:fixing`, `aidp:testing` | `in-progress` |
| `aidp:awaiting-merge` | `in-review` |
| `aidp:done` | `done` |
| `aidp:blocked` | `blocked` |
| `aidp:skip` | `blocked: skip` (never dispatched) |

**Refining.** An issue whose body has no `CA-n` acceptance criterion parks on `aidp:triage`
(`blocked: needs-refine`). The pipeline template `issue-refine` (skill `github-issue-ticket`)
rewrites the body, publishes it and puts `aidp:ready` back; the next sync makes the ticket `todo`.

**Blockers.** A sentence « Dépend de #n » / « Dépend du #n », « Depends on #n », « Blocked by #n »,
« Bloquée par #n » in the body, and GitHub's native dependencies
(`gh api repos/<o>/<r>/issues/<n>/dependencies/blocked_by`), both become
`github.blocked_by`; the ticket then waits with `waiting-on:GH-<n>[,...]` until every blocker is
closed. A reference may name another repository (`owner/name#n`) when it is configured.

**Priority and milestone.** `prio:high` and `prio:low` labels set the priority (`High`, `Low`,
`Medium` otherwise); the milestone (title, due date) is mirrored in the frontmatter.

Verbs for the flows (the actions `gh.issue_label` and `gh.issue_comment` call them):

| Verb | What it does |
|---|---|
| `label <KEY> <STATE>` | Replaces the issue's state label by `aidp:<STATE>` (or the full label name); prints `{key, label, changed, removed}`. |
| `comment <KEY> --body-file F` \| `--body T` `[--cost-footer]` | Posts a comment on the issue. `--cost-footer` appends the ticket's cost line when the usage source provides it. |
| `sync --source github` `[--repo O/N]` `[--limit N]` `[--dry-run]` | The sync above. Local, no daemon needed; `POST /api/tickets/sync?source=github` is the daemon's twin. |

`aidp gh labels-migrate --repo O/N [--apply]` (script `scripts/migrate-github-labels.ps1`) brings an
existing repository to this vocabulary. It is a dry run unless `--apply`: it creates the missing
labels (the eleven `aidp:*`, `code`, `art`, `map`, `doc`, `prio:high`, `prio:low`), turns
`aidp:need-fix` into `aidp:fixing`, and `aidp:done` on an open issue closed by an open PR into
`aidp:awaiting-merge`.

## Vault resolution

`aidp ticket` follows the same `--vault` / `AIDP_VAULT` precedence as every other `aidp` command, but
it **fails explicitly when no vault resolves** — it does not fall back to `.aidp/` the way the
knowledge commands do. A ticket store with no vault would have nowhere durable to live; refusing
outright is preferred to silently writing beside the repo.

`TODO/` is created by `aidp ticket` itself, the first time it runs against a vault — not by `aidp
vault init`, which only creates `Pipelines/` and `Knowledge/`. Two creation paths for two sibling
roots under the vault, and that split is deliberate: `vault init` sets up the pipeline substrate;
`TODO/` only exists once someone actually uses the ticket store.

## `--force`

`--force` on `aidp ticket status` (and the equivalent `POST /api/tickets/:key/status` with
`"force": true`) waives only the one guard refusal that blocks a human-initiated transition away
from `in-progress` without it. It does not touch, cancel, or otherwise affect any running session:
a `--force`-transitioned ticket whose session is still actually running keeps running, orphaned
from the ticket's new status, until that session itself ends — at which point the dispatcher's own
session-ended handling applies as normal.

`aidp ticket start --force` (and `POST /api/tickets/:key/start` with `"force": true`) sets
`autopilot.manual_priority`, which reorders the dequeue queue (see [Dequeue order](#dequeue-order)
above) so the ticket is considered first. Outside a budget wait this is the whole effect: the
ticket still waits its turn behind the normal slot ceiling, at the front of the line once a slot
frees. While the dispatcher is in `waiting` mode, `--force` takes a dedicated fast path instead
that skips both the slot ceiling and the token-budget guard for that one ticket — the launch is
journalled `forced-over-budget` (see [Budget](#budget)) and every other queued ticket keeps
waiting. Either way the one-project-one-session rule is never bypassed: a forced ticket whose
project already has a running session is still skipped (reported as `queue-race`), because both
paths only ever submit to `SessionManager::start`, never admit directly.

## Forced move

`POST /api/tickets/:key/force {"target": "<column>", "reason": "..."}` (the board's « Forcer vers… »)
unblocks a ticket by hand: it moves the ticket to a board column whatever its status, skipping the
steps between, and leaves it valid there so its path goes on. No transition guard applies; the move
is journalled with actor `human` and reason `forced:<target>` (followed by the given reason).

| Target | Status | Phase | Label | Flow |
|---|---|---|---|---|
| `ready` | `todo` | `dev` | `aidp:ready` | none: the dispatcher starts the dev lane from scratch |
| `review` | `in-progress` | `dev` | `aidp:review` | delivery flow, entry `review` |
| `to-test` | `in-progress` | `to-test` | `aidp:to-test` | none: the test lane takes it |
| `testing` | `in-progress` | `testing` | `aidp:testing` | test flow (else delivery flow), entry `testing` |
| `merge` | `in-progress` | `testing` | `aidp:awaiting-merge` | test flow (else delivery flow), entry `merge` |
| `blocked` | `blocked` (`blocked_reason: forced`) | kept | `aidp:blocked` | none |
| `done` | `done` | kept | `aidp:done` | none; the issue and the PR are left as they are |

In order: the target's flow is resolved first (the route's `flow:` and `test_flow:` in
`routing.yaml`, the first whose definition declares the entry; none answers `409` and nothing
changes), the active flow run of the ticket is aborted (its session is cancelled, worktree and PR
kept), the note is written, the label is set on the issue of a GitHub mirror (a failure is left to
the next write-back), then the flow restarts from the entry: the top-level steps before it are
skipped with reason `forced`, except the ones the entry keeps, and the outputs the entry assumes
(`test.passed: true` for `merge`) make the later conditions hold. See
[flows.md](flows.md#entries-forced-start).

## The dispatcher

### Two lanes (dev and test)

Since the decision of 2026-11-08 a flow-driven game ticket has a **phase**, `autopilot.phase`: `dev` (default),
`to-test`, `testing`. `GET /api/tickets` shows it as `phase`, with `branch`.

- Dev lane: `Todo`/`Queued` tickets, their route's `flow:` in a worktree, at most `dispatcher.slots` (default 3) at once
  (dispatcher sessions plus running dev flows). The dev flow ends at `to-test`: label `aidp:to-test`, phase `to-test`,
  status still `in-progress`, worktree removed.
- Test lane: tickets in phase `to-test` (status `in-progress`, or `todo`/`queued` when a human put the label
  `aidp:to-test` on the issue), launched with the route's `test_flow:` (`routing.yaml`) in the main checkout, at most
  `dispatcher.test_slots` (default 1) per project, oldest first, a manual start (`POST /api/tickets/:key/start`) ahead.
  Refusal reason `no-test-slot`. The phase becomes `testing` when the test flow starts. A test flow never uses a dev slot.
- Pause, budget and night mode apply to both lanes. `dispatcher.json` `slots` carries `used`/`max` (dev) and
  `testUsed`/`testMax` (test).
- A manual or forced start, and any flow launch, is no longer short-circuited by the adoption of an open PR: a ticket in
  the dev lane stays there until it is `to-test`.
- `aidp:to-test` is a phase label (`github.phase_labels`): the sync keeps it on an `in-progress` ticket, and reads it on
  an issue as "send to the test lane".

`aidp gateway serve` runs a dispatcher loop that ticks every `dispatcher.tick_seconds` and on
session events (start, end, gate question, gate answer). The timer tick also probes the PRs of
`in-review` tickets for a merge (`gh pr view`) at most every 10 minutes. Each tick launches at most `dispatcher.slots` sessions, counting
only the sessions the dispatcher itself started — a UI session on the same project occupies that
project and blocks the dispatcher from touching it, but consumes none of the dispatcher's slots. The
dispatcher never writes `queue.json` and never reimplements the one-session-per-project rule: it
submits every launch to `SessionManager::start`'s verdict, and if it loses the race for a project it
cancels its own queue entry immediately and returns the ticket to `todo`. It writes its state to
`TODO/.state/dispatcher.json` — slots used, mode, the last plan's launches and skips — which
`_TODO.md` renders in its Dispatcher section. The gate `pr` is pre-approved for every session the
dispatcher starts, so an autonomous run never stalls on its own PR; any other gate blocks the ticket
as `blocked: gate:<stage>` while the session keeps running and keeps its slot.

A ticket routed to a template that declares no `prompt` input is skipped as
`template-without-demand-mode`. That skip is expected behavior, not a bug.

A launch the dispatcher submits but that fails to start is recorded as `reason: "start-failed"` in
`dispatcher.json`'s `lastPlan.skipped`, with an optional `detail` string carrying the launch error —
see [`GET /api/dispatcher`](gateway-api.md#dispatcher-endpoints--lot-9a) for the full field.

A ticket whose frontmatter lists unfinished blockers (`github.blocked_by`) is
skipped as `waiting-on:<KEY>[,<KEY>...]`. It stays `todo`, and the skip writes no journal line. A
manual start bypasses it. If the blockers form a cycle, the ticket is launched anyway with
`detail: "blocker cycle"`.

### Launch mode

`dispatcher.launch` selects what the dispatcher starts for a ticket: `session` (default) or `flow`, which runs the default flow (`perso-delivery`). A ticket whose flow is active is skipped as `owned-by-flow` in `lastPlan.skipped`. With `flow` set and no flow engine attached, every ticket is skipped as `flow-unavailable`. The branch recorded for a flow-delivered ticket is `fix/<KEY>` for a Bug and `feat/<KEY>` for any other type. A manual start through `POST /api/tickets/:key/flow` ignores the dispatcher pause.

### Native runs

The dispatcher and `aidp ticket start` launch a native run (`mode: "native"`, autonomous by default). `autopilot.run_id` and `autopilot.session_id` both carry the native run id. Status transitions follow the run: `in-progress` while it runs, `review` when it ends with a PR, `blocked: gate:<stage>` on a gate that is not pre-approved, `failed` when it fails. `engine: "workflow"` on the launch restores the session-driven path, where `session_id` is the Claude session id. See [native-execution.md](native-execution.md).

## Worktrees and the cap

Every launch runs in `<vault>/Pipelines/<repo>/<KEY>/.worktree` on `feat/<KEY>`, not in the project
checkout (details in [gateway-api.md](gateway-api.md#worktrees--lot-17a)). The unit of exclusion is
the workspace, so a UI session on the checkout no longer blocks the dispatcher.

`dispatcher.max_worktrees_per_project` (default `3`) bounds the live worktrees of a project: tickets
that are `in-progress`, `in-review` or `blocked` and own one. `dispatcher.worktree_copy` lists the
untracked files copied into a new worktree.

Skip reasons in `lastPlan.skipped`:

| Reason | Meaning |
|---|---|
| `worktree-cap` | The project already holds the maximum. `detail` lists the keys that hold them. |
| `branch-checked-out-elsewhere` | `feat/<KEY>` is checked out in another worktree, your checkout for instance. Nothing is created. |

A ticket that already owns its worktree is never held back by the cap. A manual start
(`aidp ticket start`, `POST /api/tickets/:key/start`) is never capped: it launches with the warning
`worktree-cap: 3 already open (A, B, C)`, returned in the response and recorded in `dispatcher.json`.

### Worktree lifecycle

- **PR merged** (probe or `pr.merged`): unsaved work is committed with `--no-verify` as
  `wip(<KEY>): sauvegarde avant suppression` and pinned under `refs/aidp-wip/<KEY>-<sha12>` in the
  main repository, nothing is pushed. Then the worktree and `feat/<KEY>` are removed.
- **Ticket cancelled**: same save, then removal.
- **`blocked: pr-closed`**: the worktree stays, since the PR can be reopened.
- **Removal failure** (a Windows lock from an IDE, say): 3 attempts, then `worktree.remove_failed`
  and a line in `GET /api/health`. The next sweep retries.
- **Sweep**: at daemon start and on `POST /api/health/cleanup`, each `.worktree` whose ticket is
  `done`, `cancelled` or unknown is saved then removed. `aidp gateway worktrees list|prune` shows
  and runs the same sweep.

## Budget

Before it launches anything, the dispatcher asks `aidp_tickets::budget::verdict` whether the
account's rolling usage windows have room left — `verdict` is the only place this decision is made,
and it is pure: no filesystem, no network, no clock read (the caller passes `now_ms`).

Two windows are tracked, each with its own ceiling from `config.yaml`'s `budget:` block:

| Window | Ceiling key | Default |
|---|---|---|
| 5 hours | `max_utilization_5h` | `0.80` |
| 7 days | `max_utilization_7d` | `0.90` |

`reserve_per_run_5h` / `reserve_per_run_7d` (defaults `0.10` / `0.02`) is the estimated utilization
one more dispatcher run costs on each window — used until enough history exists to learn a better
figure (see below). "Unreflected sessions" are dispatcher sessions started after the usage
snapshot's `observedAt` timestamp: the snapshot cannot yet show their consumption, so each one is
projected onto the window at the reserve rate before deciding how much room is actually left.

`verdict` applies these rules in order:

1. No usable snapshot — none at all, or its 5h window already expired — branches on
   `unknown_policy`: `block` refuses every launch outright; `conservative` (the default) allows at
   most one dispatcher session in total, so a freshly started daemon launches exactly one and waits
   for that session's first real snapshot before considering a second; `allow` treats both windows
   as `0.0` utilization instead.
2. A fresh snapshot reporting `status: rejected` or already drawing on overage waits for the 5h
   reset (or pauses, if no 5h window is known at all).
3. The reserve per run for each window is decided: the configured `reserve_per_run_5h` /
   `reserve_per_run_7d`, raised to the learned p75 of any template in the dispatch mix that has
   enough samples, when learning is on (see [Learning the reserve](#learning-the-reserve)).
4. Each still-unreflected launch's projected cost is added on top of the observed utilization for
   both windows (a missing 7d window counts as `0.0`).
5. How many more launches fit under each ceiling is computed as `floor((ceiling − projected) /
   reserve)`, clamped to `0`; a non-positive reserve is treated as unconstrained rather than divided
   by. The smaller of the two window counts is `max_launches`.
6. If `max_launches` is `0`, the dispatcher waits on whichever window is the tighter constraint —
   the 5h window if it is already at or over its ceiling, the 7d window otherwise — or pauses if
   that window's reset instant is not actually known.
7. Otherwise, `max_launches` more sessions may launch this tick.

The verdict is one of four:

| Verdict | Meaning | The dispatcher |
|---|---|---|
| `allow` | Room for `max_launches` more sessions right now | launches up to that many candidates this tick |
| `conservative` | No usable snapshot yet | launches at most one session, waiting for real data before a second |
| `wait` | One window is out of room | stops launching and enters waiting mode until that window resets |
| `paused` | Nothing to launch, and no instant to resume at | launches nothing this tick; a later tick with fresher data may recompute |

### Waiting mode

A `wait` verdict sets `mode: waiting` and `wait_until` (the window's reported reset instant plus a
one-minute margin) in `.state/dispatcher.json`, alongside the rest of the tick state. While waiting:

- The tick loop sleeps `min(tick_seconds, wait_until − now)` instead of the full tick interval, so
  it wakes up at (or just after) the reset rather than idling out an extra full tick.
- `dispatcher.waiting {until, window, utilization, limit}` is emitted exactly once, on entry into
  waiting mode — not on every tick spent waiting. `dispatcher.resumed {"reason": "budget"}` is
  emitted once, on exit.
- No ticket launches while waiting, including in reaction to a `session.ended` event that would
  otherwise free a slot and trigger an immediate replay. Sessions already running are untouched.
- `aidp ticket start --force` still launches even in this state — see [`--force`](#--force) — and
  the launch is journalled `forced-over-budget`.

### Rate-limited session ends

A session end is classified `rate_limited: true` — rather than a real failure — when any of three
signals holds: the last usage snapshot observed during the session reported `status: rejected`; the
session's last text or error event matches one of `budget.rate_limit_patterns`
(`config.yaml`, see above); or the session exited non-zero while the 5h window's utilization was
already at or above its ceiling. `lifecycle::on_ended` then returns the ticket to `todo` with
`autopilot.attempts` unchanged and `rate-limited` in `.state/ticket-events.jsonl` — a session killed
by the usage limit is never charged as a failed attempt.

### Learning the reserve

Each dispatcher session end that brackets a clean before/after pair of usage snapshots — no window
reset in between — is recorded as one sample per template at
`<vault>/TODO/.state/budget-stats.json`:

```json
{
  "templates": {
    "dev-unity": {
      "samples": 7,
      "p50_delta_5h": 0.06,
      "p75_delta_5h": 0.09,
      "p75_delta_7d": 0.015,
      "deltas_5h": [0.04, 0.05, 0.09, "…"],
      "deltas_7d": [0.01, 0.012, 0.015, "…"]
    }
  },
  "updated_at": "2026-09-14T12:00:00Z"
}
```

A sample is the (5h delta, 7d delta) observed between the two bracketing snapshots, divided by the
average number of dispatcher sessions running concurrently during the run — the assumed
approximation is that concurrent sessions share the window equally, so the observed jump is spread
across them rather than charged in full to one template. A sample is rejected outright, never
recorded, when a window reset fell between its two snapshots. Percentiles are recomputed over the
last 30 samples per template on every accepted sample. `reserve_for` — what `verdict`'s rule 3 uses
— starts from the configured `reserve_per_run_5h` / `reserve_per_run_7d` and raises each to the p75
of any template in the dispatch mix that has at least 5 recorded samples, independently per window;
it returns the configured values unchanged when learning is off or the mix is empty.

### Exposure

The verdict is explainable through four surfaces: `aidp ticket budget`, `GET
/api/dispatcher/budget`, the `budget` object nested inside `GET /api/dispatcher`, and a Budget table
in `_TODO.md`.

### Two boundaries this lot does not move

The automations' own usage guard (`utilization_block`, threshold `0.9` on the 5h window by default,
overridable via `AIDP_MAX_UTILIZATION`, run marked `skipped`) is a **separate mechanism on a
separate code path** — same underlying usage data, unrelated call site — and this lot leaves it
untouched.

And structurally: this project never works around a rate limit by switching to a billed API key —
the guard above is the whole answer to running out of window, not a stopgap until one is bought.

## Out of scope

- The viewer's Tickets page.
