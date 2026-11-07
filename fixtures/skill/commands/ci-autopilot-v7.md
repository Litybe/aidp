---
description: Inspect and control the ai-dev-pipeline-v7 CI autopilot — what CI is unhappy about, which pipeline it would run, and whether it is allowed to run it unprompted.
argument-hint: "[status | run | ack | off | on | reset | config]"
---

Control the **CI autopilot**: the `aidp ci` layer that turns a red build, a red SonarQube gate or
unresolved PR review comments into an automatic pipeline run.

`$ARGUMENTS` selects the action. Default (no argument): `status`.

## status — what is CI unhappy about right now
```bash
aidp ci status --refresh --format json-pretty
```
Report, in plain language: the PR in flight, the failing checks, the Sonar gate, the unresolved review
threads, and for each signal the pipeline that would run. If `skipped` is set, say why (no PR, draft,
ignored branch, PR already merged) — that is not a failure.

If `errors.sonar` is set, mention it once: the `sonar` signal is degraded, the other two are fine.
The fix is `aidp sonar config --host <url> --save` plus `SONAR_TOKEN` in the environment.

## run — do now what the autopilot would do
Invoke the `ci-autopilot` skill and follow it. Use this when the hooks are off, or when you want the
work started without waiting for the next event.

## ack — stop the current signals from firing
```bash
aidp ci ack --all --note "handled manually"
```
Use after fixing something by hand, or when the user declines. Signals re-fire on a new head sha, so
this silences the *current* state, not the feature.

## off / on — the master switch, per repo
```bash
aidp ci config --enable false --save     # off
aidp ci config --enable true  --save     # on
```
Writes `.aidp/ci-autopilot.json`. To keep the reporting but stop the automatic starts, use
`--auto-dispatch false` instead: signals still surface, and Claude proposes rather than acts.

## reset — forget every dispatch record
```bash
aidp ci reset
```
Everything currently wrong will be reported again on the next probe. Useful after changing the config
or when debugging why a signal is not firing.

## config — show the effective configuration
```bash
aidp ci config --format json-pretty
```
Shows the master switch, the per-kind pipeline mapping (`gha`→`gha-fix`, `sonar`→`sonar-fix`,
`comments`→`pr-comments`), the cache TTLs, the ignored branches and the notification cap, plus the
paths of the config and state files.

## Notes
- Set `AIDP_SKILL_ROOT` to the v7 skill root for every call so v7 templates are discovered.
- Run these from inside the project repo — the autopilot is per-repo, and `--workspace-root <path>`
  is the way to target another one.
- The hooks that make this automatic live in `hooks/ci-events.example.json` and are installed by
  `/pipeline-setup-v7`. Without them, everything above still works on demand.
