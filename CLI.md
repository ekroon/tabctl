# tabctl CLI

## Quick start
```bash
npm link
tabctl --help
tabctl help --json
tabctl policy --init
tabctl skill
```

## Policy (enforced when present)
- Policy file: `$XDG_CONFIG_HOME/tabctl/policy.json` (or `~/.config/tabctl/policy.json`)
- If the file is missing, no policy is applied.
- Protected tabs are excluded from outputs and actions.
- The default policy protects pinned tabs and group title `🔒`.

Create the default policy file:
```bash
tabctl policy --init
```

## Global flags
- `--help`: command-specific help
- `--json`: JSON output
- `--pretty`: pretty-print JSON (default: true)
- `--format` is only supported by `report` (use `--json` elsewhere)

## Option Groups

Commands reference these reusable option groups to avoid documentation duplication.

### Scope Options
Filter which tabs/groups to operate on.

| Option | Description |
|--------|-------------|
| `--tab <id>` | Target specific tab(s) by ID (repeatable) |
| `--group <name>` | Target tabs in group by title |
| `--group-id <id>` | Target group by ID (use `-1` for ungrouped) |
| `--ungrouped` | Alias for `--group-id -1` |
| `--window <id|active|last-focused>` | Target tabs in specific window |
| `--all` | Target all eligible tabs |

### Pagination Options
Control result paging (default limit: 100).

| Option | Description |
|--------|-------------|
| `--limit <n>` | Maximum items to return |
| `--offset <n>` | Skip first n items |
| `--no-page` | Disable pagination, return all results |

These option groups apply anywhere referenced; they are not repeated under every command.

## Commands

### help
Show CLI help.
```bash
tabctl help
tabctl help --json
tabctl help open
```

### list
List browser tabs.
```bash
tabctl list
```
JSON output is nested under `data.windows[].tabs[]` when using `--json`.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--groups` (alias for group-list command)

### analyze
Find duplicates and stale tabs (optional GitHub state checks).

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--stale-days <n>`
- `--github`
- `--github-concurrency <n>`
- `--github-timeout-ms <ms>`
- `--window-title` (include active window title in output)
- `--progress`

If no scope is provided, all eligible tabs are analyzed.

### dedupe
Plan (and optionally close) duplicate tabs.

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--stale-days <n>`
- `--github`
- `--github-concurrency <n>`
- `--github-timeout-ms <ms>`
- `--include-stale`
- `--window-title` (include active window title in output)
- `--progress`
- `--confirm`

### inspect
Run signals to collect metadata (page-meta, github-state, selector).

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--signal-config <path>`
- `--signal <id>` (repeatable)
- `--selector <name=css|json>` (repeatable; supports `text`/`textMode` in JSON)
- `--selector-attr <attr>` (default attr for selectors)
- `--signal-concurrency <n>`
- `--signal-timeout-ms <ms>`
- `--wait-for load|dom|settle|none`
- `--wait-timeout-ms <ms>`
- `--progress`

Wait modes:
- `load` – wait for page load event
- `dom` – wait for DOMContentLoaded
- `settle` – wait for URL and title to stabilize (500ms after load); recommended for JS-heavy pages
- `none` – no waiting

Signals:
- `page-meta` (description + h1)
- `github-state` (PR/issue state when available)
- `selector` (runtime-configured selectors)

Notes:
- `--selector` implies `--signal selector`.
- Unknown signals are rejected; valid signals: `page-meta`, `github-state`, `selector`.
- Selector `attr` supports `href-url`/`src-url` to return absolute http(s) URLs.
- Selector `:contains()` is not supported; use selector text filters or screenshots.

Suggested flow for agents:
1. `tabctl screenshot --tab <id> --mode full`
2. Identify the element visually.
3. `tabctl inspect --tab <id> --signal selector --selector '{"name":"target","selector":".your-selector"}'`
4. For links, set `--selector-attr href-url` (or per-selector `attr: "href-url"`).

### screenshot
Capture screenshots for tabs (viewport or full-page tiles).

**Uses:** [Scope Options](#scope-options)

Additional options:
- `--mode viewport|full`
- `--format png|jpeg`
- `--quality <n>` (jpeg only)
- `--tile-max-dim <px>` (full mode only)
- `--max-bytes <n>`
- `--wait-for load|dom|settle|none`
- `--wait-timeout-ms <ms>`
- `--out <dir>` (writes per-tab folders)
- `--progress`

Output:
- Writes files to `./.tabctl/screenshots/<timestamp>` by default and includes `writtenTo` in JSON output.

Examples:
```bash
tabctl screenshot --tab 123 --mode viewport
tabctl screenshot --tab 123 --mode full --tile-max-dim 1500 --max-bytes 2000000
```


### focus
Focus a tab by id.
```bash
tabctl focus --tab <id>
```

### refresh
Refresh a tab by id.
```bash
tabctl refresh --tab <id>
```

### open
Open new tabs and optionally group them in a target window.
Options:
- `--url <url>` (repeatable)
- `--group <name>` (new group title)
- `--color <name>` (group color)
- `--before-tab <id>`
- `--after-tab <id>`
- `--after-group <name>` (insert tabs after this group)
- `--window <id|active|last-focused|new>`
- `--new-window`
- `--window-group <name>` (window containing a group with this title)
- `--window-tab <id>` (window containing this tab)
- `--window-url <substring>` (window containing a tab whose URL includes this substring)

Allowed colors: `grey`, `blue`, `red`, `yellow`, `green`, `pink`, `purple`, `cyan`, `orange`.

Example:
```bash
tabctl open --url https://example.com --group "Docs" --color blue
```

If no window selector is provided, the focused window is used (fallback to last-focused if needed).

### group-list
List groups with window ids/labels and tab counts.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

### group
Alias for `group-list`.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

### group-update
Update group metadata (title, color, or collapsed state).
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles)
- `--title <name>`
- `--color <name>`
- `--collapsed`
- `--expanded`

### group-ungroup
Remove all tabs from a group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles)

### group-assign
Move existing tabs into an existing group (or create one).
Options:
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id|active|last-focused>` (disambiguate group titles or create target)
- `--create` (create group if missing)
- `--color <name>`
- `--collapsed`
- `--expanded`

### move-tab
Move a single tab before/after a tab or group.
Options:
- `--tab <id>`
- `--before-tab <id>`
- `--after-tab <id>`
- `--before-group <name>`
- `--after-group <name>`
- `--window <id|active|last-focused>` (disambiguate group names)
- `--new-window`

Note: `--before-group`/`--after-group` only position tabs; use `group-assign` to move tabs into a group.

### move-group
Move a group before/after a tab or group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--before-tab <id>`
- `--after-tab <id>`
- `--before-group <name>`
- `--after-group <name>`
- `--window <id|active|last-focused>` (disambiguate group names)
- `--new-window`

Note: `--before-group`/`--after-group` only position groups; they do not merge groups.

### merge-window
Move tabs from one window into another (policy-filtered).
Options:
- `--from <id>`
- `--to <id>`
- `--close-source` (requires `--confirm`)
- `--confirm`

### setup
Install the native host manifest (macOS only).
Options:
- `--browser edge|chrome` (required)
- `--extension-id <id>` (required; or `TABCTL_EXTENSION_ID`)
- `--node <path>` (optional; or `TABCTL_NODE`)

Run once per browser:
```bash
tabctl setup --browser edge --extension-id <edge-id>
tabctl setup --browser chrome --extension-id <chrome-id>
```

### policy
Show the current policy summary and path, or create a default policy file.
Options:
- `--init`

### archive
Move tabs/groups into the Archive window.

**Uses:** [Scope Options](#scope-options)

### close
Close explicit targets only (policy-filtered). Requires confirmation for direct close.

**Uses:** [Scope Options](#scope-options) (except `--all`)

Additional options:
- `--apply <analysisId>`
- `--confirm`
- `--dry-run` (alias for `analyze`)

Note: policy enforcement blocks `close --apply`; use explicit tab targets.

### report
Generate a report for eligible tabs.

**Uses:** [Scope Options](#scope-options), [Pagination Options](#pagination-options)

Additional options:
- `--format json|md|csv`
- `--out <path>`

### undo
Restore the last action by transaction id.
```bash
tabctl undo <txid>
```
Options:
- `--txid <id>` (alias for positional)
- `--latest` (undo most recent transaction)

Examples:
```bash
tabctl undo tx-123
tabctl undo --txid tx-123
tabctl undo --latest
```

### history
List recent actions.
Options:
- `--limit <n>`
```bash
tabctl history --limit 20
```
Returns a JSON array under `data`.

JSON example:
```bash
tabctl history --json | jq -r '.data[] | {txid, action, summary}'
```

### skill
Install the tabctl agent skill for local agents (uses the Skills CLI under the hood).
Options:
- `--agent <name>` (repeatable, used for install hint)
- `--global`

```bash
tabctl skill
```

### version
Show CLI version information.
```bash
tabctl version
```
Environment:
- `TABCTL_VERSION_MODE=release` (force release version without git sha)
- `TABCTL_VERSION_MODE=dev` (force dev version with git sha)

### ping
Check host/extension connectivity.
```bash
tabctl ping
```

Notes:
- Use `--group-id -1` or `--ungrouped` to target ungrouped tabs.
 - `screenshot --out` writes per-tab folders into the target directory.
## Runtime state
- Socket: `$XDG_STATE_HOME/tabctl/tabctl.sock` (or `~/.local/state/tabctl/tabctl.sock`)
- Undo log: `$XDG_STATE_HOME/tabctl/undo.jsonl` (or `~/.local/state/tabctl/undo.jsonl`)
