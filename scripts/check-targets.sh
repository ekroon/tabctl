#!/usr/bin/env bash
# Cross-compile check for all platform targets.
# Validates Rust code compiles (type-checks) for Linux, Windows, and macOS,
# catching #[cfg] gate errors locally in ~30s instead of waiting for CI.
set -euo pipefail

MANIFEST="rust/Cargo.toml"
TARGETS=(
  x86_64-unknown-linux-gnu
  x86_64-pc-windows-msvc
  x86_64-apple-darwin
)

# Resolve manifest path relative to repo root
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST_PATH="$REPO_ROOT/$MANIFEST"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "ERROR: $MANIFEST_PATH not found" >&2
  exit 1
fi

# Ensure all rustup targets are installed
installed=$(rustup target list --installed)
for target in "${TARGETS[@]}"; do
  if ! echo "$installed" | grep -q "^${target}$"; then
    echo "::  Installing rustup target: $target"
    rustup target add "$target"
  fi
done

failed=0
results=()

for target in "${TARGETS[@]}"; do
  echo ""
  echo "── cargo check --target $target ──"
  if cargo check --manifest-path "$MANIFEST_PATH" --workspace --all-targets --target "$target" 2>&1; then
    results+=("✓ $target")
  else
    results+=("✗ $target")
    failed=1
  fi
done

echo ""
echo "═══════════════════════════════════"
echo "  Cross-target check results"
echo "═══════════════════════════════════"
for r in "${results[@]}"; do
  echo "  $r"
done
echo "═══════════════════════════════════"

if [[ $failed -ne 0 ]]; then
  echo "FAIL: one or more targets did not pass cargo check" >&2
  exit 1
fi

echo "OK: all targets passed"
