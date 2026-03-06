---
name: tabctl
description: Manage and analyze Edge tabs and groups with tabctl. Use GraphQL-first workflows via `tabctl query` for browser reads and mutations, and keep mutations explicit and reversible.
---

# Tab Control

Use tabctl to inspect and analyze tabs safely, then perform targeted GraphQL mutations only when requested.

## Public browser API

Browser state is now exposed through GraphQL only:
- `tabctl query '<GRAPHQL>'`
- `tabctl schema`
- `tabctl ping`
- `tabctl history`

Do not use removed legacy browser subcommands like `list`, `open`, `close`, `inspect`, `report`, `screenshot`, `archive`, or `undo`.

## Discovery

```bash
tabctl schema
tabctl help
tabctl help query
```

## Read examples

```bash
# Windows, tabs, and groups
 tabctl query '{ windows { windowId focused groups { groupId title } tabs { tabId title url groupTitle } } }'

# Paginated tabs
 tabctl query '{ tabs(limit: 20, offset: 0) { total hasMore items { tabId title url } } }'

# Analyze duplicate/stale tabs
 tabctl query '{ analyze(windowId: 123, staleDays: 30) { totalTabs duplicateTabs staleTabs } }'

# Inspect page metadata
 tabctl query 'query { inspectTabs(tabIds: [456], signals: ["page-meta"]) { entries { tabId signals { name valueJson } } } }'

# Build reports
 tabctl query '{ reportTabs(windowId: 123) { entries { tabId title url description } } }'

# Capture screenshots
 tabctl query 'query { captureScreenshots(tabIds: [456], mode: "viewport") { entries { tabId tiles { index width height } } } }'
```

## Mutation examples

```bash
# Open tabs in a new grouped window
 tabctl query 'mutation { openTabs(urls: ["https://example.com"], group: "Research", newWindow: true) { windowId groupId tabs { tabId url } } }'

# Group and window changes
 tabctl query 'mutation { updateGroup(groupId: 99, title: "Inbox", color: "blue") { groupId title color } }'
 tabctl query 'mutation { assignToGroup(tabIds: [456], groupTitle: "Inbox") { groupId title } }'
 tabctl query 'mutation { ungroupTabs(tabIds: [456]) { tabId groupId } }'
 tabctl query 'mutation { moveGroup(groupId: 99, newWindow: true) { movedToWindowId movedTabs } }'
 tabctl query 'mutation { mergeWindows(fromWindowId: 200, toWindowId: 123, closeSource: true, confirm: true) { movedTabs sourceClosed } }'

# Tab changes
 tabctl query 'mutation { focusTab(tabId: 456) { success } }'
 tabctl query 'mutation { refreshTabs(tabIds: [456]) { refreshedTabs } }'
 tabctl query 'mutation { moveTab(tabIds: [456], windowId: 123, index: 0) { movedTabs } }'

# Destructive actions with undo support
 tabctl query 'mutation { closeTabs(tabIds: [456], confirm: true) { txid closedTabs } }'
 tabctl query 'mutation { archiveTabs(windowId: 123) { txid archivedTabs } }'
 tabctl query 'mutation { deduplicateTabs(windowId: 123, confirm: true) { txid closedTabs } }'
 tabctl query 'mutation { undoAction(latest: true) { txid summary } }'

# Extension reload
 tabctl query 'mutation { reloadExtension { reloading } }'
```

## Safety

- Prefer read-only GraphQL queries unless the user explicitly asks for changes.
- Keep mutations scoped and reversible; surface `txid` values when available.
- Never run destructive workflows against the user's full browsing session without explicit targets or confirmation.
- Respect policy: protected tabs are excluded from query and mutation results.
- Use screenshots only when visual context is necessary.

## Convenience commands

```bash
tabctl ping
tabctl history --limit 10
```

Use `ping` for connectivity/version health and `history` to find undo transactions.

## Troubleshooting

- `tabctl ping --json` is the canonical runtime health check.
- `tabctl doctor --fix` repairs local profile wiring and sync drift.
- `TABCTL_AUTO_SYNC_MODE=release-like tabctl query '{ tabs { total } }'` forces runtime sync behavior during local release-like testing.
