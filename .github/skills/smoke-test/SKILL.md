---
name: smoke-test
description: 'Run the tabctl end-of-task smoke test: unit tests, integration tests (if available), and a live browser smoke test in a dedicated TEST- window. Use when finishing a task, before releasing, or to verify the extension and host are wired up correctly.'
license: MIT
allowed-tools: Bash
---

# Smoke Test

Run the full end-of-task verification sequence for tabctl: unit tests, integration tests, and a live browser smoke test using a disposable test window. Always run this after making code changes.

The live browser steps use a **dedicated, agent-controlled Edge instance** started by
`scripts/smoke-browser.js`. This instance is fully isolated — it uses a temp profile dir,
loads the local `dist/extension` build, and is torn down at the end. The user's real browser
is never touched.

## Prerequisites

- `npm run build` has been run so `dist/extension/manifest.json` exists.
- `jq` is available for JSON extraction.
- Edge (or Chrome) is installed on the machine.
- The debug binary `./rust/target/debug/tabctl` exists (built by `npm run build`).

## Step 0: Start the smoke browser

Start an isolated browser instance and capture the profile name. Run this first; keep the
process alive for the entire smoke test, then kill it at the end.

```bash
SMOKE_LOG=$(mktemp)
node scripts/smoke-browser.js > "$SMOKE_LOG" &
SMOKE_PID=$!

# Wait for the ready JSON line (up to 40 seconds).
for i in $(seq 1 40); do
  if [ -s "$SMOKE_LOG" ] && grep -q '"ok":true' "$SMOKE_LOG" 2>/dev/null; then
    break
  fi
  sleep 1
done

if ! grep -q '"ok":true' "$SMOKE_LOG" 2>/dev/null; then
  echo "smoke-browser did not start in time" >&2
  kill "$SMOKE_PID" 2>/dev/null || true
  exit 1
fi

SMOKE_PROFILE=$(jq -r '.profile' "$SMOKE_LOG")
echo "Smoke profile: $SMOKE_PROFILE"

# Use the debug binary so new features are available.
TABCTL="./rust/target/debug/tabctl"
```

The script sets up a named tabctl profile (`smoke-<timestamp>`) and launches Edge with
`--load-extension` pointing at the synced active extension dir. It stays running until killed.

## Step 1: Unit tests (no browser required)

```bash
npm test
```

All tests must pass. If any fail, stop and fix before continuing.

## Step 2: Integration tests (isolated headless Chrome)

```bash
npm run test:integration
```

These run against an isolated headless Chrome instance — no real tabs are touched. Run if
Chrome is available (always in CI, usually locally). If unavailable, note it and continue.

## Step 3: Connectivity ping

```bash
$TABCTL ping --profile "$SMOKE_PROFILE"
```

Must succeed. If it fails, the smoke browser is not connected — check its stderr output
and fix before proceeding.

## Step 4: Live browser smoke test (read-only)

```bash
$TABCTL list --profile "$SMOKE_PROFILE" --all
$TABCTL analyze --profile "$SMOKE_PROFILE" --stale-days 30
$TABCTL report --profile "$SMOKE_PROFILE" --format json
```

Confirm output looks reasonable: tabs listed, no unexpected errors.

## Step 5: Mutation smoke test (dedicated test window)

Run this entire block as a script. It creates a disposable test window, captures IDs, and
runs close + archive round-trips with undo verification.

```bash
set -euo pipefail

# --- Setup ---
ts=$(date +%s)
GROUP="TEST-Smoke-${ts}"

# Create test window; capture its windowId from the JSON response
WIN=$($TABCTL open --profile "$SMOKE_PROFILE" --new-window \
  --url https://example.com \
  --url https://example.org \
  --url https://example.net \
  --group "$GROUP" | jq '.windowId')

echo "Test window: $WIN  group: $GROUP"

# Verify: three tabs, group present
$TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN"
$TABCTL group-list --profile "$SMOKE_PROFILE" --window "$WIN"

# --- Close + undo ---
TAB=$($TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN" | jq '.windows[0].tabs[0].tabId')
echo "Closing tab $TAB"

TXID=$($TABCTL close --profile "$SMOKE_PROFILE" --tab "$TAB" --confirm | jq -r '.txid')
echo "Closed (txid=$TXID). Verifying tab is gone:"
$TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN"

$TABCTL undo --profile "$SMOKE_PROFILE" "$TXID"
echo "Undone. Verifying tab restored:"
$TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN"

# --- Archive + undo ---
echo "Archiving window $WIN"
TXID=$($TABCTL archive --profile "$SMOKE_PROFILE" --window "$WIN" | jq -r '.txid')
echo "Archived (txid=$TXID). Expected groups in Archive window:"
$TABCTL group-list --profile "$SMOKE_PROFILE" --all | grep -i "TEST-Smoke\|Ungrouped" || true

$TABCTL undo --profile "$SMOKE_PROFILE" "$TXID"
echo "Undone. Test window restored:"
$TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN"

# --- Screenshot + inspect check ---
FIRST_TAB=$($TABCTL list --profile "$SMOKE_PROFILE" --window "$WIN" | jq '.windows[0].tabs[0].tabId')
$TABCTL screenshot --profile "$SMOKE_PROFILE" --tab "$FIRST_TAB" --mode viewport
$TABCTL inspect --profile "$SMOKE_PROFILE" --tab "$FIRST_TAB" --signal page-meta --progress

# --- Clean up test window ---
$TABCTL close --profile "$SMOKE_PROFILE" --window "$WIN" --confirm
echo "Test window $WIN closed. Smoke test complete."
```

If `set -euo pipefail` causes the script to abort early, run sections individually and check
each output before proceeding.

## Step 6: Tear down the smoke browser

```bash
kill "$SMOKE_PID" 2>/dev/null || true
rm -f "$SMOKE_LOG"
echo "Smoke browser stopped."
```

`smoke-browser.js` handles its own cleanup on shutdown: it removes the tabctl profile and
deletes the temp dir automatically.

## Summary checklist

- [ ] `npm test` passes
- [ ] `npm run test:integration` passes (or skipped with note)
- [ ] Smoke browser started (`smoke-browser.js` ready signal received)
- [ ] `tabctl ping --profile $SMOKE_PROFILE` succeeds
- [ ] Read-only commands (`list`, `analyze`, `report`) return sensible output
- [ ] Test window created with `TEST-Smoke-<timestamp>` group
- [ ] Close + undo round-trip verified (tab disappeared, then restored)
- [ ] Archive + undo round-trip verified (groups appeared, then window restored)
- [ ] Test window cleaned up
- [ ] Smoke browser stopped (`kill $SMOKE_PID`)

## Hard stops

- Never run mutations outside the `TEST-Smoke-*` window.
- Never run `tabctl archive --all` or `tabctl close --confirm` without an explicit `--window`, `--group`, or `--tab` scope.
- Never skip `--profile "$SMOKE_PROFILE"` — always target the smoke browser, not the user's real profile.
- If `tabctl ping` fails, check `smoke-browser.js` stderr output and fix the connection before any live browser steps.
