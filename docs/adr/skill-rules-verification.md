# ADR: Skill rules verified by code, conditional skills resolved at compile

Status: accepted

## Context

Skills state rules such as "no `//` comments" or "no ticket keys", and the review relied on a model to notice a breach. Breaches slipped through and cost review rounds. Separately, `graphify-context` was ordered on every stage even when the run had no knowledge graph, and some stages carried skills with no rule applicable to their deliverable.

## Decision

1. Mechanical skill rules are checked by deterministic Rust over the added lines of the branch diff. `aidp skill-rules check` holds a per-skill catalogue (today `rust-dev-aidp`: no `//` line comment other than `///` and `//!`, no ticket key, both on added `.rs` lines) and writes a `findings.json` of `blocking` findings with file and line.
2. The command exits 0 whether or not violations exist. The `review` synthesis stage and dev-aidp's `rereview` merge its output as axis `skill_rules` with `aidp findings merge`, which derives `has_blocking_findings`.
3. Conditional skills are resolved once, at compile time. `aidp workflow emit` drops `graphify-context` from every stage when the run has no knowledge graph, and warns once per stage.
4. A skill declared on a stage must carry at least one rule applicable to that stage's deliverable. Procedure skills on their own stage are exempt. A guard test enforces a denylist.

## Consequences

- A rule in the catalogue is enforced the same way on every run, with no model judgement.
- Only added lines are scanned, so existing code that predates a rule is never flagged.
- A rule that needs understanding, not matching, stays with the reviewer.
- The prompt, options and expected `skills_loaded` always agree, since the skill is removed from all three together.
- Adding a rule means adding a catalogue entry and a test; the review stage needs no change.

## Alternatives rejected

- Leaving the rules to the reviewing model: inconsistent, and costs a round per miss.
- Deciding at run time whether a skill applies: the stage would load or skip it by guess.
- A non-zero exit on violations: it would stop the synthesis stage before it could merge the findings.
