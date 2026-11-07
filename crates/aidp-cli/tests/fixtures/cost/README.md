# Agent JSONL Fixture Template

## Overview

The `agent-a1.jsonl.tmpl` file is a template for an agent transcript fixture used by the cost E2E test suite. It cannot be a plain static fixture because `scan_run_agents` (in `rust/crates/aidp-core/src/tokens.rs`) unconditionally confirms every candidate JSONL against the workspace root via the `cwd` field, and attribution requires matching the run id from the JSONL header against the expected run id — both of which are only known at test runtime.

## Placeholder Contract

Two placeholders must be substituted before the template is used:

- `__CWD__`: JSON-escaped absolute path of the test's temporary workspace root
- `__RUN_ID__`: The run id returned by `aidp run init`

These placeholders are substituted by the test harness as it materializes the fixture into a temporary directory. Both values must match what the `aidp` binary will resolve at runtime.

## File Structure

The template contains exactly two JSON lines (JSONL format):

**Line 1 (Prompt/Attribution Line):**
- Contains `cwd` and `message` fields
- `message.content` is a plain string (never a content block list) carrying the AIDP_RUN and AIDP_STAGE headers
- These headers are parsed by `parse_agent_file` in `tokens.rs` to attribute the transcript to a run/stage

**Line 2 (Usage Line):**
- Contains `message.id`, `message.model`, and `message.usage` nested under the `message` key
- Usage fields must be nested under `message`, never at the top level (see the shape rules below)
- The usage is aggregated by message id to compute final token counts

## Shape Rules (Critical for Silent-Zero Prevention)

- `usage` and `model` MUST be nested under `message`, never at the top level
- `message.content` MUST be a plain string, never a list of content blocks
- `message.id` must be present and unique per assistant line (for deduplication)
- The `AIDP_RUN=` and `AIDP_STAGE=` lines must be on the FIRST line of the file (line 0)
- `cwd` field confirms the workspace root at parse time

## Token Arithmetic

This fixture encodes the following token values:

- **input_tokens**: 100
- **output_tokens**: 200
- **cache_creation_input_tokens**: 5000
- **cache_read_input_tokens**: 9000

### Cost Calculations

- **total_tokens**: 100 + 200 = **300** (cache is excluded by design)
- **billable_total**: 100 + 200 + 5000 + 9000 = **14300** (includes cache)

These values are load-bearing assertions in the cost E2E test suite — changing them will break the test expectations.
