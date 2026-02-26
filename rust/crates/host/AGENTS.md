# host Crate Agent Guide

## Module map
- `src/lib.rs`: declarative crate root forwarding to implementation.
- `src/host_impl.rs`: current host runtime/protocol implementation leaf.

## Constraints
- Keep crate root thin and forward-only.
- Preserve request/response protocol behavior and undo semantics.
- Keep transport behavior stable across unix/windows/tcp paths.
