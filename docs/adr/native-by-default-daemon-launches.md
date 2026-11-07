# ADR: Native by default for daemon launches

Status: accepted

## Context

Pipelines launched by the gateway daemon ran through a Claude session that drove `workflow.js`. The native executor can run the same pipeline without that session, and native runs carry autonomy, assumptions and a typed plan. Daemon launches should use it.

## Decision

1. Every pipeline launched by the daemon creates a native run (`mode: "native"`): viewer template launch, `aidp gateway start --template`, ticket dispatcher and Start, automations, presets and the flow `pipeline` step.
2. Autonomy defaults to true for the dispatcher and the flow pipeline step, to false for viewer launches, and is opt-in (`autonomous: true`) for presets and automations.
3. Add a `RunLauncher` seam in `aidp-gateway` that picks native or workflow per launch. Do not branch inside `SessionManager::start`: `NativeRunManager` starts its stage sessions through `SessionManager`, so a native branch there would make a stage session start another run.
4. `engine: "workflow"` (legacy alias `v7`) on a launch, preset or automation keeps the previous path.
5. `/run-pipeline-v7` and `aidp workflow emit` keep workflow mode.
6. `autopilot.run_id` and `autopilot.session_id` on a ticket carry the native run id.

## Consequences

- Unattended launches no longer depend on a live Claude session, and their assumptions are visible in the API.
- Two paths stay supported, so a regression in one has a documented fallback.
- A caller reading `session_id` as a Claude session id must read the run id for native launches.

## Alternatives rejected

- A native branch in `SessionManager::start`: it is re-entered by stage sessions of the native run itself.
- A global switch: it cannot give one preset or automation the old path while the rest move on.
