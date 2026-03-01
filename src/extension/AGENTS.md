# Extension Agent Guide

Thin primitive layer — Chrome API wrappers only, no orchestration logic.

## Architecture

The extension exposes ~16 primitives with `p:` prefix (e.g. `p:tab-query`, `p:group-update`, `p:screenshot-tile`). All command orchestration lives in the Rust host (`rust/crates/host/src/host_impl/orchestrate/`).

## Files

- `background.ts`: Service worker — native messaging port, primitive dispatch table, input validation/clamping.
- `lib/content.ts`: Content-script functions injected via `p:execute-script`.
- `lib/screenshot.ts`: Screenshot capture and OffscreenCanvas tiling for `p:screenshot-tile`.

## Constraints

- **No orchestration here.** Multi-step logic belongs in the Rust host.
- **Primitives are flat.** Params must not use nested wrapper objects.
- **Validate at the boundary.** Enum fields and numeric ranges are validated/clamped in `background.ts` before calling Chrome APIs.
- New CLI commands should add orchestration in `rust/crates/host/`, not new handlers here.
