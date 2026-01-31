# tabctl CLI

## Quick start
```bash
npm link
tabctl --help
tabctl policy --init
```

## Policy (always enforced)
- Policy file: `$XDG_CONFIG_HOME/tabctl/policy.json` (or `~/.config/tabctl/policy.json`)
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
- `--progress`

### inspect
Run signals to collect metadata (page-meta, github-state, selector).
Options:
- `--signal <id>` (repeatable)
- `--signal-config <path>`
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

### archive
Move tabs/groups into the Archive window.
Options:
- `--all`
- `--window <id>`
- `--group <name>`
- `--group-id <id>`
- `--tab <id>` (repeatable)

### close
Close explicit targets only (policy-filtered). Requires confirmation.
Options:
- `--tab <id>` (repeatable)
- `--group <name>`
- `--group-id <id>`
- `--window <id>`
- `--confirm`

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
```bash
tabctl history --limit 20
```

### ping
Check host/extension connectivity.
```bash
tabctl ping
```
