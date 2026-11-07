---
aidp: artifact
stage: implement
title: "T4 Implementation: ProcShim and MockServerHandle"
tags: [aidp/artifact, LOC-12, testkit, process-testing, mock-server]
---

# T4 Implementation Summary

## What was added

### `proc_shim.rs`
A generalized fake executable module that:
- Creates Windows `.bat` or Unix `.sh` scripts in a temporary workspace
- Captures all command-line arguments passed to the fake binary
- Automatically prepends the workspace directory to `PATH` via `EnvGuard`
- Restores the original `PATH` when the shim is dropped

**Key design**:
- Uses `EnvGuard` for automatic PATH restoration (no manual save/restore)
- Writes arguments to a file (named `.argv`) for easy testing
- Includes test `gh_shim_captures_argv` validating argument capture and PATH restoration

### `mock_server.rs`
Encapsulation of the ephemeral port boot sequence used in multiple test files:
- Binds to an OS-assigned ephemeral port via `TcpListener::bind("127.0.0.1:0")`
- Drops the listener exactly once to free the port
- Spawns `aidp_integrations::mock::serve()` in a background thread
- Polls up to 40 times (50ms interval) for readiness

**Key design**:
- `drop(listener)` appears exactly once in the entire crate ✓
- Provides `addr()` and `base_url()` accessors
- Includes test `mock_server_jira_401_without_auth` verifying 401 response without auth header

## Verification checklist

- [x] `grep -rn "drop(listener)" rust/crates/aidp-testkit/src/` returns exactly 1 line
- [x] No modifications to `aidp-integrations` or `aidp-tickets` production code
- [x] Both modules compile and include comprehensive unit tests
- [x] `lib.rs` wiring lines provided in follow_ups (merge step owns the edit)

## What the merge step needs to do

Add to `rust/crates/aidp-testkit/src/lib.rs`:

```rust
pub mod proc_shim;
pub mod mock_server;

pub use proc_shim::ProcShim;
pub use mock_server::MockServerHandle;
```

These should be added after the existing `temp_workspace` and `env_guard` module declarations.
