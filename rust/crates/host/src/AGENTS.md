# host src Subtree Agent Guide

## Current structure
- `lib.rs`: top-level forwarding surface.
- `host_impl.rs`: module map + forwarding shell (`run()`).
- `host_impl/runtime.rs`: platform runtime wiring (unix/windows/tcp bootstrap).
- `host_impl/dispatch.rs`: client/native IO dispatch loop.
- `host_impl/protocol.rs`: protocol framing + response helpers.
- `host_impl/state.rs`: request/response state machine + action routing.
- `host_impl/undo.rs`: undo-log persistence + retention helpers.

## Refactor direction
- Keep top-level API stable (`pub fn run()`).
- Preserve protocol response shapes and undo semantics across module moves.
