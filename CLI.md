# tabctl CLI

## Quick start
```bash
npm link
tabctl --help
tabctl help --json
tabctl policy --init
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
- `--help`: human-readable help
- `--json`: JSON help output (use with `tabctl help --json`)

## Commands

### help
Show CLI help.
```bash
tabctl help
tabctl help --json
```

### list
List eligible tabs and groups only.
```bash
tabctl list
```

### analyze
Find duplicates and stale tabs (optional GitHub state checks).
Options:
- `--stale-days <n>`
- `--github`
- `--github-concurrency <n>`
- `--github-timeout-ms <ms>`
- `--tab <id>` (repeatable)
- `--window-title` (include active window title in output)
- `--progress`

### inspect
Run signals to collect metadata (page-meta, github-state, selector).
Options:
- `--signal-config <path>`
- `--signal <id>` (repeatable)
- `--selector <name=css|json>` (repeatable)
- `--signal-concurrency <n>`
- `--signal-timeout-ms <ms>`
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id>`
- `--all`
- `--progress`

Signals:
- `page-meta` (description + h1)
- `github-state` (PR/issue state when available)
- `selector` (runtime-configured selectors)

### focus
Focus a tab by id.
```bash
tabctl focus --tab <id>
```

### open
Open new tabs and optionally group them in a target window.
Options:
- `--url <url>` (repeatable)
- `--group <name>` (new group title)
- `--after-group <name>` (insert tabs after this group)
- `--window <id>`
- `--new-window`
- `--window-group <name>` (window containing a group with this title)
- `--window-tab <id>` (window containing this tab)
- `--window-url <substring>` (window containing a tab whose URL includes this substring)

If no window selector is provided, the focused window is used.

### group-list
List groups with their window and tab counts.
Options:
- `--window <id>`

### group-update
Update group metadata (title, color, or collapsed state).
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id>` (disambiguate group titles)
- `--title <name>`
- `--color <name>`
- `--collapsed`
- `--expanded`

### group-ungroup
Remove all tabs from a group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--window <id>` (disambiguate group titles)

### group-assign
Move existing tabs into an existing group (or create one).
Options:
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id>` (disambiguate group titles or create target)
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
- `--window <id>` (disambiguate group names)
- `--new-window`

### move-group
Move a group before/after a tab or group.
Options:
- `--group <name>`
- `--group-id <id>`
- `--before-tab <id>`
- `--after-tab <id>`
- `--before-group <name>`
- `--after-group <name>`
- `--window <id>` (disambiguate group names)
- `--new-window`

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
- `--extension-id <id>` (required; or `TABARCHIVE_EXTENSION_ID`)
- `--node <path>` (optional; or `TABARCHIVE_NODE`)

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
Options:
- `--all`
- `--window <id>`
- `--group <name>`
- `--group-id <id>`
- `--tab <id>` (repeatable)

### close
Close explicit targets only (policy-filtered). Requires confirmation for direct close.
Options:
- `--apply <analysisId>`
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id>`
- `--confirm`
- `--dry-run` (alias for `analyze`)

Note: policy enforcement blocks `close --apply`; use explicit tab targets.

### report
Generate a report for eligible tabs.
Options:
- `--format json|md|csv`
- `--out <path>`
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id>`
- `--all`

### undo
Restore the last action by transaction id.
```bash
tabctl undo <txid>
```

### history
List recent actions.
Options:
- `--limit <n>`
```bash
tabctl history --limit 20
```

### ping
Check host/extension connectivity.
```bash
tabctl ping
```
