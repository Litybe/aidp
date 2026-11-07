# Flow definitions

This directory holds ticket workflow definitions read by the flow engine
(`aidp flow ...`). They are validated against `schema/flow.schema.json`.

They are not v7 pipelines: they carry no `stages:` and must not be run with
`aidp run`. `aidp pipeline list` must not discover them.

## Definitions

- `test-flow.yaml`: a minimal flow used by tests.


## Executors

The same definition runs with either executor. By default the gateway executes every action and probe itself. A host that launches the flow with `policy.executor: "external"` executes the non-agent steps and answers them as jobs, while the flow keeps its order, loops and gates. See [External executor](../../../../docs/flows.md#external-executor).
