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

# Inspect page metadata
 tabctl query 'query { inspectTabs(tabIds: [456], signals: ["page-meta"]) { totals { tabs tasks } entries { tabId signals { name valueJson } } } }'

# Inspect selectors with typed attrs and filters
 tabctl query 'query { inspectTabs(windowId: 123, selectors: [{ name: "prices", selector: ".price", attr: "text", all: true }, { name: "checkout_visible", selector: "#checkout", attr: "visible" }, { name: "checkout_style", selector: "#checkout", attr: "styles", styleProps: ["color", "background-color"] }, { name: "item_count", selector: ".line-item", attr: "count" }, { name: "email_value", selector: "input[type=email]", attr: "value" }]) { totals { tabs tasks } entries { tabId url signals { name valueJson } } } }'

# Read page content as Markdown
 tabctl query 'query { readTabs(windowId: 123, extract: true, maxChars: 50000) { totals { tabs tasks } entries { tabId title url markdown chars truncated extracted status emptyReason diagnostics { sourceHtmlChars sourceTextChars documentReadyState truncatedHtml } error } } }'

# Generate a report with descriptions
 tabctl query '{ reportTabs(windowId: 123) { totals { tabs } entries { tabId title url description } } }'

# Capture screenshots
 tabctl query 'query { captureScreenshots(tabIds: [456], mode: "viewport") { totals { tabs tiles } entries { tabId tiles { index width height } } } }'

# Persisted browser-state checkpoints and logical group history
 tabctl query 'query { latestBrowserState { snapshotId reason groups { logicalGroupId title browserGroupId tabUrls } } }'
 tabctl query 'query { browserStateHistory(limit: 10) { snapshotId recordedAt reason eventCount eventKinds } }'
 tabctl query 'query { browserStateGroupHistory(title: "Inbox", limit: 10) { snapshotId logicalGroupId title browserGroupId tabUrls } }'
```

### inspectTabs

`inspectTabs` can return built-in signals like `page-meta` and selector-derived signals from `SelectorSpecInput`.

`SelectorSpecInput` fields:
- `name` — result key for the selector
- `selector` — CSS selector to run in the page
- `attr` — extraction kind or attribute name
- `all` — return all matching values as an array; ignored for `count`, `box`, `styles`, `visible`, `enabled`, and `checked`
- `text` — filter matches by text content before returning values
- `textMode` — `contains` (default), `exact`, or `starts-with`
- `styleProps` — CSS property allowlist for `attr: "styles"`

Supported `attr` values:
- `text` — text content (default)
- any DOM attribute name such as `aria-label` or `data-testid`
- `href-url` / `src-url` — resolved HTTP(S) URLs
- `html` — `outerHTML` for the matched element, truncated at the per-signal cap
- `value` — `.value` for `input`, `textarea`, and `select`; otherwise `null`
- `count` — number of matching elements after the text filter; always numeric
- `box` — first match bounding box: `{x, y, width, height, top, right, bottom, left}`
- `styles` — computed-style map for the requested `styleProps`
- `visible` — `true` when the first match is rendered and not `display:none`, `visibility:hidden`, or `opacity:0`
- `enabled` — `true` when the first match is not disabled and `aria-disabled != "true"`
- `checked` — `true` for checked inputs, otherwise `aria-checked == "true"`

Example:

```bash
tabctl query 'query { inspectTabs(windowId: 123, selectors: [{ name: "prices", selector: ".price", attr: "text", all: true, text: "$", textMode: "contains" }, { name: "buy_now_visible", selector: "button.buy-now", attr: "visible" }, { name: "buy_now_style", selector: "button.buy-now", attr: "styles", styleProps: ["color", "background-color"] }, { name: "review_count", selector: ".review", attr: "count" }, { name: "email_value", selector: "input[type=email]", attr: "value" }, { name: "tos_checked", selector: "input[name=terms]", attr: "checked" }]) { entries { tabId url signals { name valueJson } } } }'
```

### readTabs

`readTabs` reads each targeted tab's main-frame HTML and converts it to Markdown.

Arguments:
- scope via one of `windowId`, `groupId`, `groupTitle`, `tabIds`, or `ungrouped`
- `extract` — enables Kreuzberg standard preprocessing before Markdown conversion; defaults to `true`
- `maxChars` — maximum Markdown characters per tab; defaults to `50000`, max `200000`
- `maxHtmlChars` — maximum raw HTML characters read per tab; defaults to `500000`, max `650000`
- `timeoutMs` — per-tab timeout in milliseconds; defaults to `15000`

Caveats:
- Markdown conversion uses the Kreuzberg `html-to-markdown` converter in the native host.
- Check `status`, `emptyReason`, `diagnostics`, and `error` per tab; empty Markdown is not reported as a clean success.
- v1 reads the main frame only; cross-origin iframes and shadow DOM are not traversed.
- Non-scriptable URLs such as `chrome://` and `about:` are returned with `status: UNSUPPORTED_URL`.

Example:

```bash
tabctl query 'query { readTabs(groupTitle: "Research", extract: true, maxChars: 30000, timeoutMs: 15000) { totals { tabs tasks } entries { tabId windowId title url markdown chars truncated extracted status emptyReason diagnostics { sourceHtmlChars sourceTextChars documentReadyState truncatedHtml } error } } }'
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
