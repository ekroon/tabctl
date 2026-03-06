---
name: tabctl
description: Manage and analyze Edge tabs and groups with tabctl. Use when asked to list, search, analyze stale or duplicate tabs, generate reports, or organize tabs. Prefer read-only commands and only run mutating actions with explicit targets and confirmation.
---

# Tab Control

Use tabctl to inspect and analyze tabs safely, then perform targeted actions only when requested.

## GraphQL API (preferred for agents)

Use `tabctl query` for field-selective reads and mutations in a single roundtrip.
Use `tabctl schema` to discover available types and fields.

### Read examples

```bash
# Get only tabId and url for all windows
tabctl query '{ windows { windowId tabs { tabId url } } }'

# Paginated tabs (default limit 20)
tabctl query '{ tabs { items { tabId url title } total hasMore } }'

# Next page
tabctl query '{ tabs(offset: 20) { items { tabId url } total hasMore } }'

# Get tabs in a specific window
tabctl query '{ tabs(windowId: 123) { items { tabId title url } total } }'

# Get groups with tab counts
tabctl query '{ groups { groupId title tabCount } }'

# Get a single tab by ID
tabctl query '{ tab(id: 456) { tabId url title active groupTitle } }'
```

### Mutation examples

```bash
# Close tabs and get remaining tabs in one call
tabctl query 'mutation { closeTabs(tabIds: [123, 456], confirm: true) { txid closedTabs remainingTabs { tabId url } } }'

# Open tabs and get the new tab IDs
tabctl query 'mutation { openTabs(urls: ["https://example.com"], group: "Research") { tabs { tabId url } } }'

# Refresh tabs
tabctl query 'mutation { refreshTabs(tabIds: [123]) { refreshedTabs } }'
```

### Introspection

```bash
tabctl schema                    # Full SDL
tabctl query '{ __type(name: "Tab") { fields { name } } }'  # Discover Tab fields
```

## Safety

- Prefer read-only commands: list, analyze, inspect, report, query (without mutations).
- Never run `archive --all` or `close --apply` in a normal session.
- Only mutate explicit targets (`--tab`, `--group`, `--window`) and use `--confirm` for close.
- Respect policy: protected tabs are excluded.
- Use screenshots only when you need visual context.

## Discover commands

- Use `tabctl help` (or `tabctl help --json`) to discover commands and flags.
- For specific commands, use `tabctl <command> --help`.

## Common tasks (CLI)

- List tabs in a window: `tabctl list --window <id|active|last-focused>`
- List tabs by recency: `tabctl list --all --sort last-accessed`
- List ungrouped tabs: `tabctl list --ungrouped`
- List with pagination: `tabctl list --all --limit 10 --offset 0`
- Close tabs: `tabctl close --tab <id1> --tab <id2> --confirm`
- Refresh a tab: `tabctl refresh --tab <id>`
- Generate a report: `tabctl report --format md` (add scope flags as needed)
- Get page metadata: `tabctl inspect --tab <id> --signal page-meta`
- Get page metadata after JS loads: `tabctl inspect --tab <id> --signal page-meta --wait-for settle`
- Extract links safely (absolute http(s) only): `tabctl inspect --tab <id> --selector '{"name":"links","selector":"a[href]","attr":"href-url","all":true}'`
- Capture visual context when needed: `tabctl screenshot --tab <id> --mode full`
- Undo most recent change: `tabctl undo --latest`
- Undo by txid: `tabctl undo <txid>` (from `tabctl history --json | jq -r '.[] | .txid'`)

## Recency & sorting

Tabs include `lastAccessedAt` (millisecond timestamp) — the most recent time each tab was viewed. This data persists in SQLite across browser restarts and profiles, keyed by normalized URL.

### Sort tabs by most-recently-accessed

```bash
# CLI — most-recently-accessed first
tabctl list --all --sort last-accessed

# Other sort options: title, url, index (default)
tabctl list --window active --sort title

# GraphQL — with orderBy
tabctl query '{ tabs(orderBy: LAST_ACCESSED_DESC) { items { tabId title url lastAccessedAt } } }'
tabctl query '{ tabs(orderBy: TITLE_ASC) { items { tabId title } } }'
```

### Use lastAccessedAt for analysis

```bash
# Find tabs not accessed in 7+ days
tabctl query '{ tabs { items { tabId title lastAccessedAt } } }' --json | python3 -c "
import json, sys, time
data = json.load(sys.stdin)
cutoff = (time.time() - 7*86400) * 1000
for t in data['data']['tabs']['items']:
    ts = t.get('lastAccessedAt') or 0
    if ts < cutoff:
        print(f\"{t['tabId']}: {t['title'][:60]}\")
"
```

## Narrow scope

Use one or more of: `--window`, `--group`, `--group-id`, `--tab`, `--ungrouped`.
If scope is unclear, ask for it before running mutating commands.

## Troubleshooting

- Check profile health: `tabctl doctor`
- Auto-repair broken profiles and resync extension files: `tabctl doctor --fix`
- Verify connection + runtime version sync: `tabctl ping --json`
- Read version state only from `ping`/`version` surfaces (not from `open`/`list` output payloads).
- For local release-like sync checks during development, run a scoped command with `TABCTL_AUTO_SYNC_MODE=release-like` (example: `TABCTL_AUTO_SYNC_MODE=release-like tabctl list --all`).

## Output flags

- Use `--json` for JSON output (list/analyze/inspect/etc.).
- `--format` is only for `report` (e.g., `tabctl report --format md`).

## Wait modes for inspect/screenshot

Use `--wait-for` to control when inspection runs:

- `load` – wait for page load event (may miss JS-set titles)
- `dom` – wait for DOMContentLoaded
- `settle` – wait for URL and title to stabilize (500ms quiet period); use for JS-heavy pages or freshly opened tabs
- `none` – no waiting (default)

Example for newly opened tabs:
```bash
tabctl open --url "https://example.com" --json
tabctl inspect --tab <new_tab_id> --wait-for settle --wait-timeout-ms 10000 --json
```
