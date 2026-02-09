# tabctl CLI

## Quick start
```bash
npm link
tabctl --help
tabctl help --json
tabctl policy --init
tabctl skill
```

## Configuration

### Config directory
`TABCTL_CONFIG_DIR` → `$XDG_CONFIG_HOME/tabctl` → `~/.config/tabctl`

Set `TABCTL_CONFIG_DIR` to override where tabctl looks for `config.json` and `policy.json`.

### config.json
Optional file in the config directory. Supported fields:

| Field | Description |
|-------|-------------|
| `dataDir` | Override the data directory (socket, undo log, host wrapper) |

### Data directory resolution
1. `config.json` → `dataDir` (if set)
2. `<configDir>/data/` (when `TABCTL_CONFIG_DIR` is set)
3. `$XDG_STATE_HOME/tabctl` → `~/.local/state/tabctl`

## Policy (enforced when present)
- Policy file: `<configDir>/policy.json` (default: `~/.config/tabctl/policy.json`)
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
- `--profile <name>`: override active profile for this command
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
- `--group <name>` (add tabs to this group; reuses an existing group with the same name by default)
- `--color <name>` (group color, applied when creating a new group)
- `--new-group` (force create a new group even if one with the same name exists)
- `--allow-duplicates` (open URLs even if already present in the target group)
- `--before-tab <id>`
- `--after-tab <id>`
- `--after-group <name>` (insert tabs after this group)
- `--window <id|active|last-focused|new>`
- `--new-window`
- `--window-group <name>` (window containing a group with this title)
- `--window-tab <id>` (window containing this tab)
- `--window-url <substring>` (window containing a tab whose URL includes this substring)

Allowed colors: `grey`, `blue`, `red`, `yellow`, `green`, `pink`, `purple`, `cyan`, `orange`.

When `--group` matches an already-open group, new tabs are added to it and duplicate URLs are skipped.
Use `--new-group` to always create a separate group, and `--allow-duplicates` to open URLs that already exist in the group.

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
Install the native host manifest and register a profile (macOS only).
Options:
- `--browser edge|chrome` (required)
- `--extension-id <id>` (required; or `TABCTL_EXTENSION_ID`)
- `--node <path>` (optional; or `TABCTL_NODE`)
- `--name <name>` (optional; defaults to browser name)
- `--dev` (coming soon; dev/CI mode via CDP)

Each run creates or updates a profile in `profiles.json`. The first profile registered becomes the default.

Run once per browser:
```bash
tabctl setup --browser edge --extension-id <edge-id>
tabctl setup --browser chrome --extension-id <chrome-id> --name chrome-work
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

### profile-list
List configured profiles.
```bash
tabctl profile-list
```

### profile-show
Show active profile details (name, browser, data directory, socket path, etc.).
```bash
tabctl profile-show
tabctl profile-show --json
```

### profile-switch
Switch the default profile.
```bash
tabctl profile-switch <name>
```

### profile-remove
Remove a profile from the registry. Does not delete native host manifests.
```bash
tabctl profile-remove <name>
```

### ping
Check host/extension connectivity.
```bash
tabctl ping
```

Notes:
- Use `--group-id -1` or `--ungrouped` to target ungrouped tabs.
 - `screenshot --out` writes per-tab folders into the target directory.
## Environment variables

| Variable | Description |
|----------|-------------|
| `TABCTL_CONFIG_DIR` | Override config directory (default: `$XDG_CONFIG_HOME/tabctl`) |
| `TABCTL_EXTENSION_ID` | Extension ID for `setup` command |
| `TABCTL_NODE` | Node binary path for `setup` command |
| `TABCTL_PROFILE` | Override active profile (same as `--profile` flag) |
| `TABCTL_VERSION_MODE` | `release` or `dev` for version output |

## Profiles
Each `tabctl setup` run registers a profile in `<configDir>/profiles.json`. A profile stores the browser type, extension ID, and data directory. The first profile registered becomes the default.

Profile resolution order:
1. `--profile <name>` flag (or `TABCTL_PROFILE` env var)
2. Default profile from `profiles.json`
3. Legacy mode (no profiles configured)

Each profile gets its own data directory with a separate socket and undo log. Policy is shared across all profiles.

## Runtime state
- Socket: `<dataDir>/tabctl.sock` (default: `~/.local/state/tabctl/tabctl.sock`)
- Undo log: `<dataDir>/undo.jsonl` (default: `~/.local/state/tabctl/undo.jsonl`)
- Profile registry: `<configDir>/profiles.json`

See [Configuration](#configuration) for how the data directory is resolved.
