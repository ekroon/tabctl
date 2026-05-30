---
name: smoke-test
description: 'Run the tabctl end-of-task smoke test with the current GraphQL-first CLI: unit tests, integration tests when available, an isolated browser smoke test, readTabs markdown extraction, and safe mutation/undo checks in a disposable TEST window. Use when finishing a task, before releasing, or verifying the extension/host/browser path.'
license: MIT
allowed-tools: Bash
---

# Smoke Test

Run the full end-of-task verification sequence for tabctl: unit tests, integration tests, and a live browser smoke test using an isolated browser profile. Always run this after code changes.

The live browser steps use a **dedicated, agent-controlled Edge/Chrome instance** started by `scripts/smoke-browser.js`. This instance is isolated: it uses a temp profile dir, loads the local `dist/extension` build, and is torn down at the end. The user's real browser is never touched.

## Current CLI model

Browser operations are **GraphQL-first**. Do not use removed legacy subcommands such as `tabctl list`, `tabctl open`, `tabctl close`, `tabctl archive`, `tabctl report`, `tabctl analyze`, `tabctl inspect`, `tabctl screenshot`, `tabctl group-list`, or `tabctl undo`.

Use:

```bash
$TABCTL query --profile "$SMOKE_PROFILE" 'query { ... }'
$TABCTL query --profile "$SMOKE_PROFILE" 'mutation { ... }'
```

If an operation shape is uncertain, inspect the live schema first:

```bash
$TABCTL schema
```

## Prerequisites

- `npm run build` has been run so `dist/extension/manifest.json` exists.
- `jq` is available for JSON extraction.
- Edge or Chrome is installed.
- The debug binary `./rust/target/debug/tabctl` exists.

## Step 0: Start the smoke browser

Start an isolated browser and capture the generated profile name. Keep the process alive for the whole smoke test.

```bash
SMOKE_LOG="/tmp/tabctl-smoke-$(date +%s).log"
node scripts/smoke-browser.js > "$SMOKE_LOG" 2>&1 &
SMOKE_PID=$!
echo "Smoke browser PID: $SMOKE_PID"
echo "Smoke browser log: $SMOKE_LOG"

for i in $(seq 1 40); do
  if [ -s "$SMOKE_LOG" ] && grep -q '"ok":true' "$SMOKE_LOG" 2>/dev/null; then
    break
  fi
  sleep 1
done

if ! grep -q '"ok":true' "$SMOKE_LOG" 2>/dev/null; then
  echo "smoke-browser did not start in time" >&2
  cat "$SMOKE_LOG" >&2
  exit 1
fi

SMOKE_PROFILE=$(jq -r '.profile' "$SMOKE_LOG")
TABCTL="./rust/target/debug/tabctl"
echo "Smoke profile: $SMOKE_PROFILE"
```

If using the Copilot CLI Bash tool, prefer an async shell session for the smoke browser and stop that session at teardown. If you must use `kill`, run it with a literal numeric PID, e.g. `kill 12345`; avoid variable-expanded `kill "$SMOKE_PID"` in environments that reject non-literal PIDs.

## Step 1: Unit tests

```bash
npm test
```

All tests must pass. If they fail, stop and fix before continuing.

## Step 2: Integration tests

```bash
npm run test:integration
```

These run against isolated headless Chrome. If Chrome is unavailable locally, note that and continue with the live smoke browser checks.

## Step 3: Connectivity

Run both the CLI ping command and GraphQL ping:

```bash
$TABCTL ping --profile "$SMOKE_PROFILE"

$TABCTL query --profile "$SMOKE_PROFILE" \
  'query { ping { ok latencyMs } }'
```

Both must succeed before any live browser mutation.

## Step 4: Read-only GraphQL smoke

Use GraphQL queries instead of removed read-only subcommands:

```bash
$TABCTL query --profile "$SMOKE_PROFILE" '
query {
  tabs(limit: 20) {
    total
    items { tabId windowId url title groupId groupTitle active }
  }
}'

$TABCTL query --profile "$SMOKE_PROFILE" '
query {
  analyze(staleDays: 30) {
    totalTabs
    staleTabs
    duplicateTabs
  }
}'

$TABCTL query --profile "$SMOKE_PROFILE" '
query {
  reportTabs {
    totals { tabs }
    entries { tabId windowId url title description }
  }
}'
```

Confirm the responses have `data`, no `errors`, and sensible counts.

## Step 5: Markdown extraction smoke

When the task is to verify agent-consumable page content, open the target page in the isolated browser and read it with `readTabs`.

Example for `https://nos.nl`:

```bash
NOS_OPEN=$($TABCTL query --profile "$SMOKE_PROFILE" '
mutation {
  openTabs(urls: ["https://nos.nl"], newWindow: true) {
    windowId
    tabs { tabId url title }
  }
}')

NOS_WIN=$(echo "$NOS_OPEN" | jq -r '.data.openTabs.windowId')
NOS_TAB=$(echo "$NOS_OPEN" | jq -r '.data.openTabs.tabs[0].tabId')
echo "NOS window: $NOS_WIN  tab: $NOS_TAB"

sleep 3

NOS_READ=$($TABCTL query --profile "$SMOKE_PROFILE" "
query {
  readTabs(tabIds: [$NOS_TAB], extract: true, maxChars: 50000, timeoutMs: 15000) {
    totals { tabs tasks }
    entries {
      tabId
      url
      title
      chars
      truncated
      extracted
      markdown
    }
  }
}")

echo "$NOS_READ" | jq -r '.data.readTabs.entries[0] |
  "url: \(.url)\ntitle: \(.title)\nchars: \(.chars)\ntruncated: \(.truncated)\nextracted: \(.extracted)\n\n--- MARKDOWN PREVIEW ---\n\(.markdown[:1200])"'
```

Success criteria:

- `readTabs.totals.tabs` is `1`.
- `entries[0].chars` is greater than `0`.
- `entries[0].markdown` contains readable Markdown, not raw empty HTML.
- `entries[0].truncated` is acceptable only if `maxChars` was intentionally low.

Keep `NOS_TAB` and `NOS_WIN` for cleanup.

## Step 6: Mutation smoke test

Run mutations only against windows/tabs created by this smoke test. The setup below creates a disposable window and group, then verifies close/undo, archive/undo, screenshot, and inspect.

```bash
ts=$(date +%s)
GROUP="TEST-Smoke-${ts}"

OPEN_JSON=$($TABCTL query --profile "$SMOKE_PROFILE" "
mutation {
  openTabs(
    urls: [\"https://example.com\", \"https://example.org\", \"https://example.net\"],
    newWindow: true,
    group: \"$GROUP\"
  ) {
    windowId
    groupId
    tabs { tabId windowId url title groupId groupTitle }
  }
}")

WIN=$(echo "$OPEN_JSON" | jq -r '.data.openTabs.windowId')
TAB_IDS=$(echo "$OPEN_JSON" | jq -r '.data.openTabs.tabs | map(.tabId) | join(",")')
FIRST_TAB=$(echo "$OPEN_JSON" | jq -r '.data.openTabs.tabs[0].tabId')
echo "Test window: $WIN  group: $GROUP  tabs: $TAB_IDS"

$TABCTL query --profile "$SMOKE_PROFILE" "
query {
  window(id: $WIN) {
    windowId
    tabs { tabId url groupTitle index }
    groups { groupId title color collapsed tabCount }
  }
}"

CLOSE_JSON=$($TABCTL query --profile "$SMOKE_PROFILE" "
mutation {
  closeTabs(tabIds: [$FIRST_TAB], confirm: true) {
    txid
    closedTabs
  }
}")
CLOSE_TXID=$(echo "$CLOSE_JSON" | jq -r '.data.closeTabs.txid')
echo "Closed tab $FIRST_TAB with txid=$CLOSE_TXID"

$TABCTL query --profile "$SMOKE_PROFILE" "
mutation {
  undoAction(txid: \"$CLOSE_TXID\") {
    txid
    summary
  }
}"

$TABCTL query --profile "$SMOKE_PROFILE" "
query {
  window(id: $WIN) {
    tabs { tabId url groupTitle index }
  }
}"

ARCHIVE_JSON=$($TABCTL query --profile "$SMOKE_PROFILE" "
mutation {
  archiveTabs(windowId: $WIN) {
    txid
    archivedTabs
  }
}")
ARCHIVE_TXID=$(echo "$ARCHIVE_JSON" | jq -r '.data.archiveTabs.txid')
echo "Archived window $WIN with txid=$ARCHIVE_TXID"

$TABCTL query --profile "$SMOKE_PROFILE" "
mutation {
  undoAction(txid: \"$ARCHIVE_TXID\") {
    txid
    summary
  }
}"

RESTORED_FIRST_TAB=$($TABCTL query --profile "$SMOKE_PROFILE" "
query {
  tabs(windowId: $WIN, limit: 10) {
    items { tabId }
  }
}" | jq -r '.data.tabs.items[0].tabId')

$TABCTL query --profile "$SMOKE_PROFILE" "
query {
  captureScreenshots(tabIds: [$RESTORED_FIRST_TAB], mode: \"viewport\") {
    totals { tabs tiles }
    entries { tabId tiles { index width height } error { message } }
  }
}"

$TABCTL query --profile "$SMOKE_PROFILE" "
query {
  inspectTabs(tabIds: [$RESTORED_FIRST_TAB], signals: [\"page-meta\"], waitFor: \"dom\") {
    totals { tabs signals tasks }
    entries { tabId signals { name valueJson } }
  }
}"
```

Success criteria:

- The test window exists after `openTabs`.
- `closeTabs.closedTabs` is `1`.
- `undoAction` for close restores the tab.
- `archiveTabs.archivedTabs` is greater than `0`.
- `undoAction` for archive restores the test window.
- Screenshot and inspect return `data` without unexpected `errors`.

## Step 7: Cleanup smoke-created browser state

Close tabs created by the markdown and mutation smoke tests. `closeTabs` requires tab IDs, so query the window first if needed.

```bash
if [ -n "${NOS_TAB:-}" ] && [ "$NOS_TAB" != "null" ]; then
  $TABCTL query --profile "$SMOKE_PROFILE" "
  mutation {
    closeTabs(tabIds: [$NOS_TAB], confirm: true) {
      txid
      closedTabs
    }
  }" || true
fi

if [ -n "${WIN:-}" ] && [ "$WIN" != "null" ]; then
  CLEANUP_TABS=$($TABCTL query --profile "$SMOKE_PROFILE" "
  query {
    tabs(windowId: $WIN, limit: 50) {
      items { tabId }
    }
  }" | jq -r '.data.tabs.items | map(.tabId) | join(",")')

  if [ -n "$CLEANUP_TABS" ]; then
    $TABCTL query --profile "$SMOKE_PROFILE" "
    mutation {
      closeTabs(tabIds: [$CLEANUP_TABS], confirm: true) {
        txid
        closedTabs
      }
    }" || true
  fi
fi
```

## Step 8: Tear down the smoke browser

Stop the `scripts/smoke-browser.js` process and remove the log.

```bash
echo "Smoke browser PID was: $SMOKE_PID"
echo "Smoke browser log was: $SMOKE_LOG"
rm -f "$SMOKE_LOG"
```

If using a normal shell, terminate the printed numeric PID with `kill <PID>`. If using the Copilot CLI Bash tool with an async shell session, stop that shell session instead. `smoke-browser.js` removes the tabctl profile and temp dir during shutdown.

## Summary checklist

- [ ] `npm test` passes.
- [ ] `npm run test:integration` passes, or Chrome unavailability is noted.
- [ ] Smoke browser started and emitted `{"ok":true,...}`.
- [ ] `tabctl ping --profile "$SMOKE_PROFILE"` succeeds.
- [ ] GraphQL `query { ping { ok latencyMs } }` succeeds.
- [ ] Read-only GraphQL queries (`tabs`, `analyze`, `reportTabs`) return `data` and no unexpected `errors`.
- [ ] Markdown extraction with `readTabs` returns non-empty Markdown for the target page, e.g. `https://nos.nl`.
- [ ] Test window created with a `TEST-Smoke-<timestamp>` group.
- [ ] Close + undo round-trip verified.
- [ ] Archive + undo round-trip verified.
- [ ] Screenshot and inspect checks return successful GraphQL data.
- [ ] Smoke-created tabs/windows are cleaned up.
- [ ] Smoke browser process is stopped.

## Hard stops

- Never mutate the user's real profile; always use `--profile "$SMOKE_PROFILE"`.
- Never run mutations against tabs/windows that the smoke test did not create.
- Never use removed legacy CLI subcommands in this skill; use `tabctl query`.
- Never call `archiveTabs` without `windowId` or explicit `tabIds`.
- Never call `closeTabs` without explicit `tabIds` and `confirm: true` for execution.
- If ping fails, inspect the smoke-browser log and fix the connection before any live browser step.
