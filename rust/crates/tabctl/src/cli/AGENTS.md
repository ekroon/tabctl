# tabctl CLI Subtree Agent Guide

## Current structure
- `mod.rs`: top-layer API surface (`pub use impls::run`).
- `impls.rs`: aggregator that wires and re-exports the split domain modules.
- Domain modules: `api.rs`, `commands.rs`, `route.rs`, `setup.rs`, `local.rs`, `output.rs`, `transport.rs`.

## Maintenance direction
- Keep `mod.rs` declarative and stable for callers.
- Keep domain logic in the split modules; avoid moving logic back into top-layer files.
