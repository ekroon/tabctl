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
- `close.rs`: `close` — preview unless confirmed and not dry-run; plan scoped removals and report protection skips.
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
- `undo.rs` and `undo/`: normalize historical/write-ahead payloads, reconcile current browser IDs, restore placement/groups, and remove only recorded created tabs.

Durable recovery is captured centrally at the host primitive boundary in `transaction.rs`, not inferred from orchestration success. Explicit empty tab selections must never broaden scope.
Multi-scope plans implement `mutation_scope()` with all planned tab sources and destination windows before their first mutation. Transaction preflight rejects mixed private/regular scopes without touching the browser or persisting private data.

**Analysis & capture:**
- `analyze.rs`: `analyze` — stale/duplicate detection.
- `inspect.rs`: `inspect` — execute signals (page-meta, selectors) on tabs.
- `report.rs`: `report` — generate tab reports with descriptions.
- `screenshot.rs`: `screenshot` — capture and tile screenshots with waitFor support.
