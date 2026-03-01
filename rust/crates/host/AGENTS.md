# host Crate Agent Guide

## Module map
- `src/lib.rs`: declarative crate root forwarding to implementation.
- `src/host_impl.rs`: host runtime/protocol implementation + unit tests.
- `src/host_impl/state.rs`: state machine driving CLI requests through orchestration.
- `src/host_impl/orchestrate/`: command orchestrations — each sequences extension primitives per CLI request.
- `src/host_impl/orchestrate/mod.rs`: `Orchestration` trait, `OrchStep` enum, `orchestration_for()` factory.
- `src/host_impl/orchestrate/resolve.rs`: snapshot resolution helpers (group by ID/title, window ID).
- `src/host_impl/orchestrate/scope.rs`: tab scoping by window/group/tab/all.

## Constraints
- Keep crate root thin and forward-only.
- Preserve request/response protocol behavior and undo semantics.
- Keep transport behavior stable across unix/windows/tcp paths.
- Extension primitives use `p:` prefix; params must be flat (never wrapped in nested objects like `createData`).
- New CLI commands should add orchestration here, never new handlers in the extension.
