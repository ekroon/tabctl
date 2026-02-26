# Rust Workspace Agent Guide

## Workspace map
- `crates/tabctl`: CLI binary entrypoint and command routing surface.
- `crates/host`: Native messaging host runtime.
- `crates/shared`: Shared protocol/config types used by tabctl + host.

## Data flow
CLI (`tabctl`) -> local socket/pipe/tcp -> host -> browser extension native messaging.

## Local constraints
- Keep top-level Rust modules declarative and forward to deeper files.
- Preserve CLI/protocol behavior unless explicitly changing behavior.
- Keep undo and scope-first safety semantics intact for mutating operations.
