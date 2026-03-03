#!/usr/bin/env bash
# scripts/test-mise-release.sh — Integration test for released tabctl versions
#
# Tests both distribution channels:
#   - npm (stable):  npx tabctl@<version>
#   - mise (alpha):  mise x github:ekroon/tabctl@<version> -- tabctl
#
# Usage:
#   ./scripts/test-mise-release.sh                          # defaults
#   ./scripts/test-mise-release.sh 0.5.3 v0.6.0-alpha.9    # explicit versions
#   SKIP_MUTATIONS=1 ./scripts/test-mise-release.sh         # read-only only

set -euo pipefail

NPM_VERSION="${1:-0.5.3}"
MISE_VERSION="${2:-v0.6.0-alpha.9}"
SKIP_MUTATIONS="${SKIP_MUTATIONS:-}"

PASS=0
FAIL=0
SKIP=0
RESULTS=()

# ── Helpers ──────────────────────────────────────────────────────────────────

green() { printf '\033[32m%s\033[0m' "$1"; }
red()   { printf '\033[31m%s\033[0m' "$1"; }
gray()  { printf '\033[90m%s\033[0m' "$1"; }

record() {
  local status="$1" channel="$2" test="$3" detail="${4:-}"
  case "$status" in
    PASS) PASS=$((PASS + 1)); printf '  %s %s: %s %s\n' "$(green ✓)" "$channel" "$test" "$(gray "$detail")" ;;
    FAIL) FAIL=$((FAIL + 1)); printf '  %s %s: %s %s\n' "$(red ✗)" "$channel" "$test" "$(red "$detail")" ;;
    SKIP) SKIP=$((SKIP + 1)); printf '  %s %s: %s %s\n' "$(gray ○)" "$channel" "$test" "$(gray "$detail")" ;;
  esac
  RESULTS+=("$status|$channel|$test|$detail")
}

# Run a tabctl command via the appropriate channel.
# Usage: run_tabctl <channel> <args...>
# Outputs: stdout only (stderr suppressed)
run_tabctl() {
  local channel="$1"; shift
  case "$channel" in
    npm)  npx -y "tabctl@${NPM_VERSION}" "$@" 2>/dev/null ;;
    mise) mise x "github:ekroon/tabctl@${MISE_VERSION}" -- tabctl "$@" 2>/dev/null ;;
  esac
}

# Parse JSON and extract a field via node (works with both envelope and compact)
json_field() {
  node -e "
    const d = JSON.parse(require('fs').readFileSync(0, 'utf8'));
    const inner = d.data || d;
    const field = process.argv[1];
    const val = field.split('.').reduce((o, k) => o?.[k], inner);
    process.stdout.write(String(val ?? ''));
  " "$1"
}

# Check if output is valid JSON
is_json() {
  node -e "try { JSON.parse(require('fs').readFileSync(0,'utf8')); } catch { process.exit(1); }"
}

# ── Test suites ──────────────────────────────────────────────────────────────

test_version() {
  local ch="$1"
  local out
  # v0.5.3 npm outputs JSON for `version`, alpha outputs plain text
  local raw
  raw=$(run_tabctl "$ch" version 2>/dev/null || true)
  if echo "$raw" | is_json 2>/dev/null; then
    out=$(echo "$raw" | json_field "version")
  else
    out=$(echo "$raw" | head -1)
  fi
  if [ -n "$out" ]; then
    record PASS "$ch" "version" "$out"
  else
    record FAIL "$ch" "version" "no output"
  fi
}

test_version_json() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" version --json || true)
  if echo "$out" | is_json; then
    local ver
    ver=$(echo "$out" | json_field "version")
    record PASS "$ch" "version --json" "version=$ver"
  else
    record FAIL "$ch" "version --json" "invalid JSON"
  fi
}

test_help_json() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" help --json || true)
  if echo "$out" | is_json; then
    # Check if structured (has commands array) or plain text
    local has_commands
    has_commands=$(echo "$out" | node -e "
      const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
      const inner = d.data || d;
      process.stdout.write(Array.isArray(inner.commands) ? 'structured' : 'text-only');
    ")
    record PASS "$ch" "help --json" "$has_commands"
  else
    record FAIL "$ch" "help --json" "invalid JSON"
  fi
}

test_ping() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" ping --json || true)
  if echo "$out" | is_json; then
    local component
    component=$(echo "$out" | json_field "component")
    record PASS "$ch" "ping --json" "component=$component"
  else
    record FAIL "$ch" "ping --json" "invalid JSON or connection failed"
  fi
}

test_list() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" list --all --json || true)
  if echo "$out" | is_json; then
    local count
    count=$(echo "$out" | node -e "
      const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
      const inner = d.data || d;
      const tabs = (inner.windows || []).reduce((n, w) => n + (w.tabs || []).length, 0);
      process.stdout.write(String(tabs));
    ")
    record PASS "$ch" "list --all --json" "tabs=$count"
  else
    record FAIL "$ch" "list --all --json" "invalid JSON"
  fi
}

test_group_list() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" group-list --all --json || true)
  if echo "$out" | is_json; then
    local count
    count=$(echo "$out" | node -e "
      const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
      const inner = d.data || d;
      process.stdout.write(String((inner.groups || []).length));
    ")
    record PASS "$ch" "group-list --json" "groups=$count"
  else
    record FAIL "$ch" "group-list --json" "invalid JSON"
  fi
}

test_analyze() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" analyze --all --json || true)
  if echo "$out" | is_json; then
    local analyzed
    analyzed=$(echo "$out" | json_field "totals.analyzed")
    record PASS "$ch" "analyze --json" "analyzed=$analyzed"
  else
    record FAIL "$ch" "analyze --json" "invalid JSON"
  fi
}

test_profile_list() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" profile-list --json || true)
  if echo "$out" | is_json; then
    local count
    count=$(echo "$out" | node -e "
      const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
      const inner = d.data || d;
      process.stdout.write(String((inner.profiles || []).length));
    ")
    record PASS "$ch" "profile-list --json" "profiles=$count"
  else
    record FAIL "$ch" "profile-list --json" "invalid JSON"
  fi
}

test_profile_show() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" profile-show --json || true)
  if echo "$out" | is_json; then
    local name
    name=$(echo "$out" | json_field "name")
    record PASS "$ch" "profile-show --json" "name=$name"
  else
    record FAIL "$ch" "profile-show --json" "invalid JSON"
  fi
}

test_policy() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" policy --json || true)
  if echo "$out" | is_json; then
    record PASS "$ch" "policy --json" "ok"
  else
    record FAIL "$ch" "policy --json" "invalid JSON"
  fi
}

test_history() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" history --json || true)
  if echo "$out" | is_json; then
    local count
    count=$(echo "$out" | node -e "
      const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
      const inner = d.data || d;
      const arr = Array.isArray(inner) ? inner : [];
      process.stdout.write(String(arr.length));
    ")
    record PASS "$ch" "history --json" "entries=$count"
  else
    record FAIL "$ch" "history --json" "invalid JSON"
  fi
}

test_doctor() {
  local ch="$1"
  local out
  out=$(run_tabctl "$ch" doctor --json || true)
  if echo "$out" | is_json; then
    record PASS "$ch" "doctor --json" "ok"
  else
    record FAIL "$ch" "doctor --json" "invalid JSON"
  fi
}

_run_with_timeout() {
  local secs="$1"; shift
  # macOS-compatible timeout using background process
  "$@" &
  local pid=$!
  ( sleep "$secs" && kill "$pid" 2>/dev/null ) &
  local watcher=$!
  wait "$pid" 2>/dev/null
  local rc=$?
  kill "$watcher" 2>/dev/null
  wait "$watcher" 2>/dev/null
  return $rc
}

# ── Mutation tests (open/close/undo) ────────────────────────────────────────

test_mutations() {
  local ch="$1"
  if [ -n "$SKIP_MUTATIONS" ]; then
    record SKIP "$ch" "open/close/undo" "SKIP_MUTATIONS=1"
    return
  fi

  local ts
  ts=$(date +%s)
  local group_name="TEST-release-${ch}-${ts}"

  # Open test window
  local open_out
  open_out=$(run_tabctl "$ch" open --new-window --url https://example.com --url https://example.org --group "$group_name" --json || true)
  if ! echo "$open_out" | is_json; then
    record FAIL "$ch" "open (mutation)" "invalid JSON"
    return
  fi

  local tab_ids window_id
  tab_ids=$(echo "$open_out" | node -e "
    const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
    const inner = d.data || d;
    process.stdout.write((inner.created || []).map(t => t.tabId).join(' '));
  ")
  window_id=$(echo "$open_out" | node -e "
    const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
    const inner = d.data || d;
    process.stdout.write(String(inner.windowId || ''));
  ")

  if [ -z "$tab_ids" ]; then
    record FAIL "$ch" "open (mutation)" "no tabs created"
    return
  fi
  record PASS "$ch" "open --new-window" "tabs=$tab_ids window=$window_id"

  # Close test tabs
  local close_args=()
  for tid in $tab_ids; do
    close_args+=(--tab "$tid")
  done
  local close_out
  close_out=$(run_tabctl "$ch" close "${close_args[@]}" --confirm --json || true)
  if echo "$close_out" | is_json; then
    local closed
    closed=$(echo "$close_out" | json_field "summary.closedTabs")
    local txid
    txid=$(echo "$close_out" | json_field "txid")
    record PASS "$ch" "close --confirm" "closed=$closed txid=$txid"
  else
    record FAIL "$ch" "close --confirm" "invalid JSON"
    return
  fi

  # Undo
  local undo_out
  undo_out=$(run_tabctl "$ch" undo --latest --json || true)
  if echo "$undo_out" | is_json; then
    local restored
    restored=$(echo "$undo_out" | json_field "summary.restoredTabs")
    record PASS "$ch" "undo --latest" "restored=$restored"
  else
    record FAIL "$ch" "undo --latest" "invalid JSON"
  fi

  # Cleanup: find and close restored tabs
  sleep 1
  local list_out
  list_out=$(run_tabctl "$ch" list --all --json || true)
  local cleanup_ids
  cleanup_ids=$(echo "$list_out" | node -e "
    const d = JSON.parse(require('fs').readFileSync(0,'utf8'));
    const inner = d.data || d;
    const ids = [];
    for (const w of inner.windows || []) {
      for (const t of w.tabs || []) {
        if (t.url?.includes('example.com') || t.url?.includes('example.org')) ids.push(t.tabId);
      }
    }
    // Also find the new-tab page in the test window
    process.stdout.write(ids.join(' '));
  " 2>/dev/null || true)

  if [ -n "$cleanup_ids" ]; then
    local cleanup_args=()
    for tid in $cleanup_ids; do
      cleanup_args+=(--tab "$tid")
    done
    run_tabctl "$ch" close "${cleanup_args[@]}" --confirm --json >/dev/null 2>&1 || true
    record PASS "$ch" "cleanup" "closed test tabs"
  fi
}

# ── Main ─────────────────────────────────────────────────────────────────────

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  tabctl Release Integration Test                            ║"
echo "╠══════════════════════════════════════════════════════════════╣"
printf '║  npm stable:  %-45s║\n' "$NPM_VERSION"
printf '║  mise alpha:  %-45s║\n' "$MISE_VERSION"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

for channel in npm mise; do
  case "$channel" in
    npm)  echo "── npm (tabctl@$NPM_VERSION) ──" ;;
    mise) echo "── mise (github:ekroon/tabctl@$MISE_VERSION) ──" ;;
  esac

  test_version      "$channel"
  test_version_json "$channel"
  test_help_json    "$channel"
  test_ping         "$channel"
  test_list         "$channel"
  test_group_list   "$channel"
  test_analyze      "$channel"
  test_profile_list "$channel"
  test_profile_show "$channel"
  test_policy       "$channel"
  test_history      "$channel"
  test_doctor       "$channel"
  test_mutations    "$channel"
  echo ""
done

# ── Summary ──────────────────────────────────────────────────────────────────

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
printf '  Results: %s passed, %s failed, %s skipped\n' \
  "$(green "$PASS")" \
  "$(red "$FAIL")" \
  "$(gray "$SKIP")"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ "$FAIL" -gt 0 ]; then
  echo ""
  echo "Failed tests:"
  for r in "${RESULTS[@]}"; do
    IFS='|' read -r status channel test detail <<< "$r"
    if [ "$status" = "FAIL" ]; then
      printf '  %s %s: %s — %s\n' "$(red ✗)" "$channel" "$test" "$detail"
    fi
  done
  exit 1
fi
