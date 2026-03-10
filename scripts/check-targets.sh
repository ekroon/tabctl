#!/usr/bin/env bash
# Optional local cross-target check.
# Verifies the targets that are practical on the current host and surfaces
# missing cross-compilers explicitly. This is manual/opt-in because some
# dependencies (for example libsqlite3-sys via rusqlite) compile C code even
# during `cargo check --target ...`.
set -euo pipefail

MANIFEST="rust/Cargo.toml"

# Resolve manifest path relative to repo root
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST_PATH="$REPO_ROOT/$MANIFEST"
HOST_OS="$(uname -s)"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "ERROR: $MANIFEST_PATH not found" >&2
  exit 1
fi

TARGETS=()

required_tool_for_target() {
  case "$1" in
    x86_64-unknown-linux-gnu)
      echo "x86_64-linux-gnu-gcc"
      ;;
    x86_64-pc-windows-gnu)
      echo "x86_64-w64-mingw32-gcc"
      ;;
  esac
}

install_hint_for_target() {
  case "$1" in
    x86_64-unknown-linux-gnu)
      echo "install a Linux cross-compiler (for example a Homebrew/toolchain package that provides x86_64-linux-gnu-gcc)"
      ;;
    x86_64-pc-windows-gnu)
      if [[ "$HOST_OS" == "Darwin" ]]; then
        echo "brew install mingw-w64"
      else
        echo "install mingw-w64 (for example: sudo apt-get install mingw-w64)"
      fi
      ;;
  esac
}

case "$HOST_OS" in
  Darwin)
    TARGETS=(
      x86_64-apple-darwin
      x86_64-unknown-linux-gnu
      x86_64-pc-windows-gnu
    )
    ;;
  Linux)
    TARGETS=(
      x86_64-unknown-linux-gnu
      x86_64-pc-windows-gnu
    )
    ;;
  *)
    echo "ERROR: unsupported host OS '$HOST_OS' for scripts/check-targets.sh" >&2
    exit 1
    ;;
esac

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
missing=()

for target in "${TARGETS[@]}"; do
  required_tool="$(required_tool_for_target "$target")"
  if [[ -n "$required_tool" ]] && ! command -v "$required_tool" >/dev/null 2>&1; then
    results+=("! $target (missing $required_tool)")
    missing+=("$target: $(install_hint_for_target "$target")")
    failed=1
    continue
  fi

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

if [[ ${#missing[@]} -gt 0 ]]; then
  echo ""
  echo "Missing local cross-compilers:"
  for item in "${missing[@]}"; do
    echo "  - $item"
  done
  echo ""
  echo "This check is optional and manual because the workspace includes native C builds during cargo check."
fi

if [[ $failed -ne 0 ]]; then
  echo "FAIL: one or more local targets were unavailable or did not pass cargo check" >&2
  exit 1
fi

echo "OK: all targets passed"
