#!/usr/bin/env bash
set -euo pipefail

TABCTL="node $(dirname "$0")/../cli/tabctl.js"
TS=$(date +%s)
GROUP="TEST-Smoke-${TS}"
ERRORS=0

log() { echo "[smoke] $*"; }
fail() { echo "[smoke] FAIL: $*" >&2; ERRORS=$((ERRORS + 1)); }

check_json() {
  local desc="$1"
  local output="$2"
  if echo "$output" | node -e "const d=JSON.parse(require('fs').readFileSync(0,'utf8')); if(!d.ok) process.exit(1);" 2>/dev/null; then
    log "PASS: $desc"
  else
    fail "$desc"
  fi
}

log "Starting smoke test (group: $GROUP)"

# Ping
OUTPUT=$($TABCTL ping --json 2>/dev/null) || true
check_json "ping" "$OUTPUT"

# Open test window
OUTPUT=$($TABCTL open --new-window --url https://example.com --url https://example.org --group "$GROUP" --json 2>/dev/null) || true
check_json "open test window" "$OUTPUT"

# Wait for tabs to load
sleep 2

# List with scope
OUTPUT=$($TABCTL list --window last-focused --json 2>/dev/null) || true
check_json "list --window last-focused" "$OUTPUT"

# Group list
OUTPUT=$($TABCTL group-list --window last-focused --json 2>/dev/null) || true
check_json "group-list" "$OUTPUT"

# Profile show
OUTPUT=$($TABCTL profile-show --json 2>/dev/null) || true
check_json "profile-show" "$OUTPUT"

# Version
OUTPUT=$($TABCTL version --json 2>/dev/null) || true
check_json "version" "$OUTPUT"

# Cleanup: close the test group
OUTPUT=$($TABCTL close --group "$GROUP" --window last-focused --confirm --json 2>/dev/null) || true
check_json "close test group" "$OUTPUT"

log "---"
if [ "$ERRORS" -gt 0 ]; then
  log "FAILED: $ERRORS error(s)"
  exit 1
fi
log "All smoke tests passed"
