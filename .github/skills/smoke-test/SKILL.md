---
name: smoke-test
description: 'Run the tabctl end-of-task smoke test: unit tests, integration tests (if available), and a live browser smoke test in a dedicated TEST- window. Use when finishing a task, before releasing, or to verify the extension and host are wired up correctly.'
license: MIT
allowed-tools: Bash
---

# Smoke Test

Run the full end-of-task verification sequence for tabctl: unit tests, integration tests, and a live browser smoke test using a disposable test window. Always run this after making code changes.

## Prerequisites

- Edge (or Chrome) is open and the extension is loaded.
- The native host manifest is installed (`tabctl setup --browser edge --extension-id <id>`).
- A stable `tabctl` binary is on PATH (production install or `cargo run -p tabctl --` during development).
- `jq` is available for JSON extraction.

If the extension is not connected, `tabctl ping` will fail. Fix the connection before proceeding.

## Step 1: Verify active profile

```bash
tabctl profile-show --json
```

Check `name` and `browser`. If multiple profiles are configured, confirm you are targeting the right browser. Append `--profile <name>` to all subsequent `tabctl` commands if needed.

## Step 2: Unit tests (no browser required)

```bash
npm test
```

All tests must pass. If any fail, stop and fix before continuing.

## Step 3: Integration tests (isolated headless Chrome)

```bash
npm run test:integration
```

These run against an isolated headless Chrome instance — no real tabs are touched. Run if Chrome is available (always in CI, usually locally). If unavailable in the current environment, note it and continue.

## Step 4: Connectivity ping

```bash
tabctl ping
```

Must succeed. If it fails, the extension or native host is not connected — do not proceed with the live smoke test.

## Step 5: Live browser smoke test (read-only)

```bash
tabctl list --all
tabctl analyze --stale-days 30
tabctl report --format json
```

Confirm output looks reasonable: tabs listed, no unexpected errors.

## Step 6: Mutation smoke test (dedicated test window)

Run this entire block as a script. It creates a disposable test window, captures IDs, and runs close + archive round-trips with undo verification.

```bash
set -euo pipefail

# --- Setup ---
ts=$(date +%s)
GROUP="TEST-Smoke-${ts}"

# Create test window; capture its windowId from the JSON response
WIN=$(tabctl open --new-window \
  --url https://example.com \
  --url https://example.org \
  --url https://example.net \
  --group "$GROUP" | jq '.windowId')

echo "Test window: $WIN  group: $GROUP"

# Verify: three tabs, group present
tabctl list --window "$WIN"
tabctl group-list --window "$WIN"

# --- Close + undo ---
# Pick the first tab in the test window
TAB=$(tabctl list --window "$WIN" | jq '.windows[0].tabs[0].tabId')
echo "Closing tab $TAB"

TXID=$(tabctl close --tab "$TAB" --confirm | jq -r '.txid')
echo "Closed (txid=$TXID). Verifying tab is gone:"
tabctl list --window "$WIN"

tabctl undo "$TXID"
echo "Undone. Verifying tab restored:"
tabctl list --window "$WIN"

# --- Archive + undo ---
echo "Archiving window $WIN"
TXID=$(tabctl archive --window "$WIN" | jq -r '.txid')
echo "Archived (txid=$TXID). Expected groups in Archive window:"
echo "  W# - ${GROUP}   (grouped tabs)"
echo "  W# - Ungrouped  (ungrouped tab)"
tabctl group-list --all | grep -i "TEST-Smoke\|Ungrouped" || true

tabctl undo "$TXID"
echo "Undone. Test window restored:"
tabctl list --window "$WIN"

# --- Screenshot check (run if screenshot/inspect was changed) ---
FIRST_TAB=$(tabctl list --window "$WIN" | jq '.windows[0].tabs[0].tabId')
tabctl screenshot --tab "$FIRST_TAB" --mode viewport
tabctl inspect --tab "$FIRST_TAB" --signal page-meta --progress

# --- Clean up ---
tabctl close --window "$WIN" --confirm
echo "Test window $WIN closed. Smoke test complete."
```

If `set -euo pipefail` causes the script to abort early, run sections individually and check each output before proceeding.

## Summary checklist

- [ ] `npm test` passes
- [ ] `npm run test:integration` passes (or skipped with note)
- [ ] `tabctl ping` succeeds
- [ ] Read-only commands (`list`, `analyze`, `report`) return sensible output
- [ ] Test window created with `TEST-Smoke-<timestamp>` group
- [ ] Close + undo round-trip verified (tab disappeared, then restored)
- [ ] Archive + undo round-trip verified (groups appeared, then window restored)
- [ ] Test window cleaned up

## Hard stops

- Never run mutations outside the `TEST-Smoke-*` window.
- Never run `tabctl archive --all` or `tabctl close --confirm` without an explicit `--window`, `--group`, or `--tab` scope.
- If `tabctl ping` fails, fix the connection before any live browser steps.
