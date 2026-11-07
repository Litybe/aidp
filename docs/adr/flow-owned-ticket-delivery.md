# ADR: Flow-owned ticket delivery

Status: accepted

## Context

A ticket could be handled by a dispatcher session or by a flow run. Two owners on one ticket push to the same branch and race on its status.

## Decision

1. The daemon owns ticket delivery through the existing flow registry. `dispatcher.launch` (`session` or `flow`, default `session`) selects what the dispatcher starts.
2. The default delivery definition covers the whole path: preparation, development, review, merge and cleanup. It is `perso-delivery` since the gamedev profile (it was `ticket-delivery`), and any flow run started for a ticket moves that ticket's status.
3. A ticket with an active flow is never handled by a session. The dispatcher skips it with the reason `owned-by-flow`.
4. The flow state is the single source of ownership. The dispatcher reads it through a port and keeps no copy of its own.
5. `POST /api/tickets/:key/flow` starts a delivery manually and ignores the dispatcher pause.
6. The flow writes the ticket status under the actor `flow`: `in-progress` at start, `in-review` when the PR is ready, `done` on merge, `blocked` on escalation.

## Consequences

- One owner per ticket, so no two writers push to the same branch.
- Restart recovery is the flow engine's, not the dispatcher's.
- The default stays `session`, so existing setups do not change.

## Alternatives rejected

- Letting a session and the flow both act on a PR: they race on pushes and replies.
- A separate ownership marker on the ticket: it can drift from the flow state.
