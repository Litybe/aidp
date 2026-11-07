# ADR: Generic dev flow templates

Status: accepted

## Context

Each stack had its own `dev-*` template, and none could fix a branch after a DEV test run without opening a second PR. The stack is known before the run starts, so one template can pick the right test phase.

## Decision

1. Add two flat sibling templates, `dev-flow` and `correction-after-test-dev`, with a `stack` enum input. Neither composes the other.
2. Select the per-stack test composite with `when:` on `inputs.stack`: `dev-tests-csharp-split`, `dev-tests-jest`, `dev-tests-vitest` or `dev-tests-cargo`. The emitter prunes the unused stages at compile time.
3. Add `aidp project stack [PATH]`, a typed detector that returns `{path, stack}`. The `stack: auto` detector at run init stays unchanged.
4. Refuse `stack: unknown` at run init. The driver offers the detected value through AskUserQuestion and escalates on `unknown`.
5. Add `aidp gh pr-comment`, so `correction-after-test-dev` comments the existing PR instead of opening one.

## Consequences

- One template covers every supported stack; adding a stack means one composite and one `when:` branch in each template.
- The two templates share most stages, so a change to one must be mirrored in the other.
- Two detectors coexist: `stack: auto` for run init, `project stack` for the driver.

## Alternatives rejected

- One template per stack: repeats the same stages in every file.
- Composing `correction-after-test-dev` from `dev-flow`: `policy:` does not survive `uses:`, and a PR-less variant would need extra switches.
- Extending `stack: auto` with the new values: changes the behaviour of existing runs.
