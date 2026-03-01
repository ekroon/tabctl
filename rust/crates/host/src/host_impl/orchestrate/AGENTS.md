# orchestrate Module Guide

Command orchestrations — each file sequences extension primitives for a single CLI request.

## Pattern

Every orchestration implements the `Orchestration` trait (`mod.rs`):
- `start()` → first `OrchStep::SendPrimitive`
- `step(response)` → advance state machine until `Complete` or `Error`

New commands: add a file here, implement `Orchestration`, wire into `orchestration_for()` in `mod.rs`, and add CLI routing in `tabctl/src/cli/route.rs`.

## Module map

**Core infrastructure:**
- `mod.rs`: `Orchestration` trait, `OrchStep` enum, `orchestration_for()` factory.
- `resolve.rs`: snapshot helpers (find group by ID/title, find window, find tab location).
- `scope.rs`: tab scoping by `--window`/`--group`/`--tab`/`--all` params.

**Tab operations:**
- `list.rs`: `list` and `group-list` — snapshot queries.
- `focus.rs`: `focus` — activate a tab.
- `refresh.rs`: `refresh` — reload tabs.
- `open.rs`: `open` — open URLs with group reuse and dedup.
- `close.rs`: `close` — close tabs with undo payload.
- `move_tab.rs`: `move-tab` — cross-window tab moves with anchor resolution.

**Group operations:**
- `group_update.rs`: `group-update` — rename/recolor groups.
- `group_assign.rs`: `group-assign` — assign tabs to groups.
- `group_ungroup.rs`: `group-ungroup` — remove tabs from groups.
- `group_gather.rs`: `group-gather` — gather tabs into a group.
- `move_group.rs`: `move-group` — cross-window group moves.

**Window operations:**
- `archive.rs`: `archive` — archive windows into a single window with undo.
- `merge_window.rs`: `merge-window` — merge windows together.
- `undo.rs`: `undo` — replay undo payloads to restore state.

**Analysis & capture:**
- `analyze.rs`: `analyze` — stale/duplicate detection.
- `inspect.rs`: `inspect` — execute signals (page-meta, selectors) on tabs.
- `report.rs`: `report` — generate tab reports with descriptions.
- `screenshot.rs`: `screenshot` — capture and tile screenshots with waitFor support.
