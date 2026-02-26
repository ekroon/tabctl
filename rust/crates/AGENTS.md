# Rust Crates Agent Guide

## Where to edit
- CLI flags/command routing: `tabctl` crate.
- Host request handling/runtime transport: `host` crate.
- Shared contract types/config/profile registry: `shared` crate.

## Progressive disclosure
- Prefer API-first top files (`mod.rs`/`lib.rs`) with forwarding.
- Keep heavy logic in deeper implementation modules.
