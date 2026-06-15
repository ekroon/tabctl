---
name: smoke-test
description: 'Run the tabctl end-of-task smoke test with the current GraphQL-first CLI: unit tests, integration tests when available, an isolated browser smoke test, readTabs markdown extraction, and safe mutation/undo checks in a disposable TEST window. Use when finishing a task, before releasing, or verifying the extension/host/browser path.'
license: MIT
allowed-tools: Bash
---

# Smoke Test

Run the full end-of-task verification sequence with the automated smoke runner:

```bash
npm run test:smoke
```

Do not manually reproduce the smoke test with ad hoc `tabctl` commands during normal verification. The runner is the contract: it builds, verifies Rust, runs browser integration tests, starts an isolated Edge/Chrome instance, performs GraphQL read/mutation/undo checks, validates `readTabs`, cleans up smoke-created tabs/windows, removes the smoke profile, stops the browser, and removes the temp smoke root.

## Isolation guarantees

The smoke browser uses `scripts/smoke-browser.js`, which creates a dedicated temp root containing:

- an isolated tabctl config dir (`TABCTL_CONFIG_DIR`)
- an isolated tabctl data dir (`TABCTL_DATA_DIR`)
- an isolated browser user-data-dir
- a smoke-only profile named `smoke-<timestamp>`

The user's normal Edge/Chrome browser profile, tabctl profile registry, active extension directory, and native messaging manifest must not be mutated by the smoke test.

## Supported overrides

- `TABCTL_BIN=./rust/target/debug/tabctl` to choose the binary.
- `EDGE_PATH=/path/to/browser` to choose the browser executable.
- `TABCTL_EXTENSION_DIR=dist/extension` to choose the extension build.
- `SMOKE_BROWSER_TIMEOUT_MS=60000` to extend browser startup time.
- `SMOKE_BROWSER_VISIBLE=1` to show the isolated smoke browser for debugging. By default the smoke browser runs headless and should not create windows in the user's window manager.
- `SMOKE_KEEP_ARTIFACTS=1` to preserve the temp smoke root for debugging after teardown.

## Debugging only

If `npm run test:smoke` fails, inspect the runner output first. Manual GraphQL commands are allowed only to debug a specific failing step, and must use the smoke profile and isolated dirs printed by the runner. Never run mutations against the user's real profile, and never use removed legacy subcommands such as `tabctl open`, `tabctl list`, `tabctl close`, `tabctl archive`, `tabctl report`, `tabctl analyze`, `tabctl inspect`, `tabctl screenshot`, `tabctl group-list`, or `tabctl undo`.

## Success criteria

- `npm run build`, Rust verification, and integration tests complete.
- The smoke browser emits ready metadata for a `smoke-*` profile under a temp root without showing normal browser windows by default.
- `ping` and GraphQL `ping` succeed against that smoke profile.
- Read-only GraphQL checks return data without unexpected errors.
- `readTabs` extracts non-empty Markdown.
- Close/undo and archive/undo round-trips succeed only on smoke-created tabs/windows.
- Screenshot and inspect checks return successful GraphQL data.
- Cleanup removes smoke-created tabs/windows and tears down the smoke browser without manual commands.
