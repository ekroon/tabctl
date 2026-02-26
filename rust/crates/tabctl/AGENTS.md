# tabctl Crate Agent Guide

## Module map
- `src/main.rs`: entrypoint dispatch (`host` subcommand vs CLI).
- `src/cli/mod.rs`: declarative CLI API surface.
- `src/cli/impls.rs`: current CLI implementation leaf.
- `src/launcher.rs`: legacy launcher path.

## Constraints
- Keep `cli/mod.rs` thin and forward-only.
- Preserve existing CLI flags, output shape, and exit behavior.
- Keep scope-first semantics for mutating commands.
