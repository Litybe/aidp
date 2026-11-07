# ADR: Native autonomy

Status: accepted

## Context

A native run stops when an agent returns `blocked` or asks a question. Runs launched by the ticket workflow or the dispatcher have nobody to answer, so they stall.

## Decision

1. Add an immutable `autonomous` flag to the native run, fixed at launch. It defaults to true for ticket workflow and dispatcher launches; a viewer launch lets the user choose.
2. In an autonomous run, pre-approve every gate and start every stage prompt with a versioned autonomy preamble (v1). The agent never asks, takes the recommended option or else the most prudent and reversible one, and records it in `assumptions[]` with the choice, the reason and the rejected alternative.
3. Retry once a reply that is `blocked` or carries `ask_user_questions[]`, answering with the recommended option (the first when none is recommended). Journal the answer as an assumption with `auto_answered: true`. A second block fails the stage with reason `blocked-twice`.
4. Aggregate assumptions in `run-state.json` and expose `autonomous` and `assumptions` on `GET /api/runs/:id`.
5. Give sessions an `autonomous` profile: no `AskUserQuestion`, permission policy `allow` with a deny list, never ask. Free autonomous sessions share it.
6. Let `pr-comments` report `escalations[]` instead of blocking.
7. Leave workflow mode (`workflow.js`) unchanged.

## Consequences

- Unattended runs finish, and a human reviews the assumptions afterwards.
- A wrong default can be taken silently, so every one is recorded and visible in the API.
- Preamble changes are versioned, so a prompt change is traceable.

## Alternatives rejected

- Prompt-only autonomy: an instruction in the prompt does not stop a session from calling `AskUserQuestion` or waiting on a permission prompt, so runs still stall.
- Session-only autonomy: removing the tools and prompts gives the agent no way to say what it chose, and a `blocked` reply would still end the run with nothing recorded.
