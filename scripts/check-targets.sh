#!/usr/bin/env bash
# Optional check of both supported macOS architectures. SQLite compiles C code,
# so run this on macOS with the Xcode command-line tools installed.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo "ERROR: tabctl supports macOS only (Apple Silicon and Intel)." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST_PATH="$REPO_ROOT/rust/Cargo.toml"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

installed="$(rustup target list --installed)"
for target in "${TARGETS[@]}"; do
  if ! grep -qx "$target" <<<"$installed"; then
    echo "ERROR: missing target $target; install with: rustup target add $target" >&2
    exit 1
  fi
done

for target in "${TARGETS[@]}"; do
  echo "── cargo check --target $target ──"
  cargo check --manifest-path "$MANIFEST_PATH" --workspace --all-targets --target "$target"
done
