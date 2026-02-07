# Agent Testing Guide

This project controls a live Edge session. The testing approach must avoid touching real tabs the user cares about.

## CLI Usage Rules for Agents

### Scope-First Rule
Always specify scope options **before** running any query or mutation command. This ensures predictable results and avoids accidental broad operations.

**Required scoping pattern:**
```bash
# Good: Explicit scope
tabctl list --window 123
tabctl list --group "Work"
tabctl list --tab 456 --tab 789
tabctl close --tab 456 --confirm

# Bad: No scope (defaults to all, risky for mutations)
tabctl close --confirm  # NEVER do this
```

**Scope options (in order of specificity):**
1. `--tab <id>` - Most specific, target individual tabs
2. `--group <name>` or `--group-id <id>` - Target a group
3. `--window <id>` - Target a window
4. `--all` - Explicit "all" (only for read operations)

### Required Scope Usage
Always include an explicit scope option when running commands that accept scope (list, analyze, dedupe, inspect, report, close, archive, group-list). Use `--all` when you truly intend to target everything.

### Confirmation Rule for Destructive Commands
Destructive commands (`close`, `archive`, `dedupe --confirm`) require explicit confirmation AND explicit scope:

```bash
# Pattern: scope first, then --confirm
tabctl close --tab 456 --confirm
tabctl close --group "Temp" --window 123 --confirm
tabctl archive --window 123

# For dedupe, always preview first:
tabctl dedupe --window 123           # Preview plan
tabctl dedupe --window 123 --confirm # Execute after review
```

### Command Workflow
1. **List first** - Use `tabctl list` with scope to see what will be affected
2. **Verify IDs** - Confirm window/group/tab IDs before mutations
3. **Execute with scope** - Run mutation with explicit `--tab`, `--group`, or `--window`
4. **Check result** - Verify with `tabctl list` or `tabctl history`
5. **Undo if needed** - Use `tabctl undo --latest` or `tabctl undo <txid>`

## Commit message style
- Use Conventional Commits (`type(scope): subject`), with scope optional.
- Keep the subject in present-tense, lowercase imperative form.
- Do not add a trailing period.

## Principles (read first)
- Only mutate tabs that the test itself created.
- Never run `archive --all` or `close --apply` in a normal browsing session.
- Use a unique, recognizable prefix for test groups and windows, e.g. `TEST-Tabctl-<timestamp>`.
- Prefer `list`, `analyze`, and `report` for smoke tests; use `close` and `archive` only in a controlled test window.
- Always add or update tests for new features.
- Always end work by running unit tests and a minimal smoke test in a new window you create (see Required end-of-task checks).

## Undo is critical
- Treat undo as a first-class safety feature for every mutating action.
- Any new mutating command must record a complete undo payload and include tests.
- Undo should restore window placement, group metadata, and tab ordering whenever possible.

## Preconditions
- Edge is open.
- The extension is loaded (`extension/`) and connected to the native host.
- The native host manifest is installed (use `tabctl setup --browser edge --extension-id <id>`).
- For development, prefer the repo script (`node ./cli/tabctl.js`) so a stable global `tabctl` can stay installed.

## Unit tests (no Edge required)
These tests validate the CLI/host helpers using a mocked socket and run without Edge or the extension.

Run:
- `npm install`
- `npm test`

Notes:
- Unit tests use the compiled JS in `tests/unit/`.
- The mock socket avoids any browser interaction.

## Required end-of-task checks
Always finish with:
1. `npm test`
2. A minimal smoke test in a new window you create (safe URLs + unique `TEST-` prefix). Verify via `tabctl group-list` or `tabctl list`.
3. A screenshot-first smoke step: capture a screenshot before running selector-based extraction.

Example (recommended for development):
```bash
ts=$(date +%s)
node ./cli/tabctl.js open --new-window --url https://example.com --url https://example.org --url https://example.net --group "TEST-Smoke-${ts}"
node ./cli/tabctl.js group-list --window last-focused
```


Screenshot-first example:
```bash
tabctl screenshot --tab <tabId> --mode viewport
tabctl inspect --tab <tabId> --signal selector --selector "link=a[href]" --selector-attr href-url
```

## Safe smoke tests (no mutations)
Run these anytime:
- `tabctl ping`
- `tabctl list`
- `tabctl analyze --stale-days 30`
- `tabctl inspect --tab <tabId> --signal page-meta --progress`
- `tabctl inspect --tab <tabId> --signal selector --selector "price=.price" --progress`
- `tabctl report --format json` (no `--out`)

## Controlled mutation tests (real Edge, minimal risk)
Only use a dedicated test window and clearly labeled groups.

### Setup a test window
1. Create a new Edge window.
2. Open three safe URLs: `https://example.com`, `https://example.org`, `https://example.net`.
3. Group the first two tabs and name the group `TEST-Tabctl-<timestamp>`.
4. Leave the third tab ungrouped.

### Archive test
1. Run `tabctl list` to find the test window id.
2. Archive only that window:
   `tabctl archive --window <windowId>`
3. Confirm in Edge:
   - A single Archive window exists.
   - A group named `W# - TEST-Tabctl-<timestamp>` exists.
   - An `W# - Ungrouped` group exists for the ungrouped tab.
4. Undo:
   `tabctl undo <txid>`

### Close test
1. Open a new tab in the test window (e.g. `https://example.com`).
2. Find its `tabId` via `tabctl list`.
3. Close that tab only:
   `tabctl close --tab <tabId> --confirm`
4. Undo:
   `tabctl undo <txid>`

### Report test
1. Run `tabctl report --window <windowId> --format md --out /tmp/tab-report.md`.
2. Verify the report includes descriptions for the example pages.

## Safe usage of analyze/close
`tabctl close` only works with explicit targets and is blocked for protected tabs by policy.

Recommended safe pattern:
1. Use a dedicated test profile or a brand-new Edge window with only test tabs.
2. Run `tabctl analyze` (add `--github` only when you accept slower analysis).
3. Use `tabctl close --tab <tabId> --confirm` for a single test tab.
4. Run `tabctl undo <txid>` to restore.

## Undo history sanity checks (synthetic)
You can validate undo with a single safe tab by inserting a synthetic record.

1. Append a single JSON line to `$XDG_STATE_HOME/tabctl/undo.jsonl` (or `~/.local/state/tabctl/undo.jsonl`) (manual or via a tiny script).
2. Use a unique `txid` and a safe URL (`https://example.com`).

Example line (single tab, forces a new window):

```
{"txid":"tx-test-undo-1","createdAt":1700000000000,"action":"close","summary":{"closedTabs":1},"undo":{"action":"close","tabs":[{"url":"https://example.com","title":"Example","pinned":false,"active":false,"from":{"windowId":0,"index":0,"groupId":-1,"groupTitle":null,"groupColor":null,"groupCollapsed":null}}]}}
```

Then run:
- `tabctl undo tx-test-undo-1`

This should open a single tab in a new window.

## Mocking strategy (for destructive paths)
When testing `archive --all` or `close --apply` behavior, use a mocked host instead of a live browser.

Suggested approach:
- Start a local UNIX socket server that returns canned JSON responses.
- Point the CLI at it with `TABCTL_SOCKET=/path/to/mock.sock`.
- Verify CLI formatting and error handling without touching Edge.

Minimal Node mock example:

```
// Run with: node mock-socket.js
const net = require("net");
const fs = require("fs");
const path = require("path");
const sock = "/tmp/tabctl-mock.sock";
if (fs.existsSync(sock)) fs.unlinkSync(sock);
net.createServer((socket) => {
  socket.on("data", (buf) => {
    const req = JSON.parse(String(buf).trim());
    const res = { ok: true, action: req.action, requestId: req.id, data: { mock: true } };
    socket.write(JSON.stringify(res) + "\n");
  });
}).listen(sock, () => console.log("Mock listening", sock));
```

Then:
- `TABCTL_SOCKET=/tmp/tabctl-mock.sock tabctl archive --all`

## Hard stop rules
- Never run `tabctl archive --all` or `tabctl close --apply` in a normal profile.
- Never run `tabctl close` without explicit `--tab`, `--group`, or `--window` targets.
- Always verify window ids and group names before any mutation.
