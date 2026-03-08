# tabctl CLI

## Quick start
```bash
mise use -g github:ekroon/tabctl   # or: cargo install --path rust/crates/tabctl

tabctl setup --browser edge --extension-id <extension-id>
tabctl ping

tabctl query '{ tabs { total items { tabId title url } } }'
tabctl schema
```

Runtime architecture: a single Rust `tabctl` binary serves as both the CLI and the native messaging host. Browser tab reads and mutations now go through GraphQL only.

## Supported commands

### Browser-facing commands
- `tabctl query <QUERY>` — execute a GraphQL query or mutation against the current browser snapshot and host actions
- `tabctl schema` — print the GraphQL SDL
- `tabctl ping` — convenience health/version check
- `tabctl history [--limit <n>]` — convenience undo-history query

### Local/admin commands
- `tabctl help [command]`
- `tabctl version`
- `tabctl setup ...`
- `tabctl upgrade`
- `tabctl doctor [--fix]`
- `tabctl policy [--init]`
- `tabctl profile-list`
- `tabctl profile-show`
- `tabctl profile-switch <name>`
- `tabctl profile-remove <name>`
- `tabctl extension-fetch ...`

## Global flags
- `--help` / `-h`
- `--version` / `-v`
- `--json`
- `--pretty`
- `--no-pretty`
- `--progress`
- `--profile <name>`

## GraphQL workflow

Use `tabctl schema` when you need field discovery, then use `tabctl query` for both reads and mutations.

### Read examples
```bash
# All windows with grouped tabs
 tabctl query '{ windows { windowId focused groups { groupId title } tabs { tabId title url groupTitle } } }'

# Paginated tabs in one window
 tabctl query '{ tabs(windowId: 123, limit: 20, offset: 0) { total hasMore items { tabId title url } } }'

# Ungrouped tabs ordered by recency
 tabctl query '{ tabs(ungrouped: true, orderBy: LAST_ACCESSED_DESC) { items { tabId title lastAccessedAt } } }'

# Group inventory for a window
 tabctl query '{ groups(windowId: 123) { groupId title color collapsed tabCount } }'

# Analyze duplicates and stale tabs
 tabctl query '{ analyze(windowId: 123, staleDays: 30) { totalTabs duplicateTabs staleTabs raw } }'

# Inspect page metadata or selectors
 tabctl query 'query { inspectTabs(tabIds: [456], signals: ["page-meta"]) { totals { tabs tasks } entries { tabId signals { name valueJson } } } }'

# Generate a report with descriptions
 tabctl query '{ reportTabs(windowId: 123) { totals { tabs } entries { tabId title url description } } }'

# Capture screenshots
 tabctl query 'query { captureScreenshots(tabIds: [456], mode: "viewport") { totals { tabs tiles } entries { tabId tiles { index width height } } } }'

# Persisted browser-state checkpoints and logical group history
 tabctl query 'query { latestBrowserState { snapshotId reason groups { logicalGroupId title browserGroupId tabUrls } } }'
 tabctl query 'query { browserStateHistory(limit: 10) { snapshotId recordedAt reason eventCount eventKinds } }'
 tabctl query 'query { browserStateGroupHistory(title: "Inbox", limit: 10) { snapshotId logicalGroupId title browserGroupId tabUrls } }'
```

### Mutation examples
```bash
# Open tabs in a new grouped window
 tabctl query 'mutation { openTabs(urls: ["https://example.com", "https://example.org"], group: "Research", newWindow: true) { windowId groupId tabs { tabId url } skippedUrls { url reason } } }'

# Focus, refresh, and move tabs
 tabctl query 'mutation { focusTab(tabId: 456) { success tabId } }'
 tabctl query 'mutation { refreshTabs(tabIds: [456]) { refreshedTabs } }'
 tabctl query 'mutation { moveTab(tabIds: [456], windowId: 123, index: 0) { movedTabs } }'

# Group and window operations
 tabctl query 'mutation { updateGroup(groupId: 99, title: "Inbox", color: "blue", collapsed: false) { groupId title color } }'
 tabctl query 'mutation { assignToGroup(tabIds: [456], groupTitle: "Inbox") { groupId title } }'
 tabctl query 'mutation { ungroupTabs(tabIds: [456]) { tabId groupId } }'
 tabctl query 'mutation { gatherGroups(windowId: 123, groupTitle: "Inbox") { summary { mergedGroups movedTabs } } }'
 tabctl query 'mutation { moveGroup(groupId: 99, newWindow: true) { movedToWindowId movedTabs } }'
 tabctl query 'mutation { mergeWindows(fromWindowId: 200, toWindowId: 123, closeSource: true, confirm: true) { movedTabs sourceClosed } }'

# Close, archive, dedupe, undo, and reload
 tabctl query 'mutation { closeTabs(tabIds: [456], confirm: true) { txid closedTabs } }'
 tabctl query 'mutation { archiveTabs(windowId: 123) { txid archivedTabs } }'
 tabctl query 'mutation { deduplicateTabs(windowId: 123, confirm: true) { txid closedTabs } }'
 tabctl query 'mutation { undoAction(latest: true) { txid summary } }'
 tabctl query 'mutation { reloadExtension { reloading } }'
```

## Convenience commands

### ping
```bash
tabctl ping
tabctl --json ping
```
Use this for connectivity and runtime-version checks.

### history
```bash
tabctl history --limit 10
tabctl --json history --limit 10
```
Use this to find `txid` values for `undoAction` mutations.

## Configuration

### Config directory
`TABCTL_CONFIG_DIR` -> `$XDG_CONFIG_HOME/tabctl` -> `~/.config/tabctl`

### config.json
Optional file in the config directory.

| Field | Description |
|-------|-------------|
| `dataDir` | Override the data directory (socket, undo log, host wrapper) |

### Data directory resolution
1. `config.json` -> `dataDir` (if set)
2. `<configDir>/data/` (when `TABCTL_CONFIG_DIR` is set)
3. `$XDG_STATE_HOME/tabctl` -> `~/.local/state/tabctl`

## Policy
- Policy file: `<configDir>/policy.json`
- If the file is missing, no policy is applied.
- Protected tabs are excluded from outputs and actions.
- The default policy protects pinned tabs and group title `🔒`.

Create the default policy file:
```bash
tabctl policy --init
```

## Notes
- Browser automation should prefer `tabctl query` over removed legacy subcommands.
- `--progress` is supported for GraphQL operations that trigger progress-emitting host actions.
- `tabctl ping --json` is the canonical runtime version check.
- `tabctl history --json` returns a top-level JSON array.
- Persisted browser-state history is stored in `<dataDir>/state.db` and exposed via GraphQL query fields such as `latestBrowserState`, `browserStateHistory`, and `browserStateGroupHistory`.
- Runtime auto-sync skips reload itself, but runs for `ping`, `history`, and `query`.
