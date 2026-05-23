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
TMP_DIR=""

cleanup() {
  if [[ -n "$TMP_DIR" ]]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "ERROR: $MANIFEST_PATH not found" >&2
  exit 1
fi

write_zig_cc_wrapper() {
  local output="$1"
  local rust_target="$2"
  local zig_target="$3"

  cat >"$output" <<EOF
#!/usr/bin/env bash
set -euo pipefail

args=()
for arg in "\$@"; do
  case "\$arg" in
    --target=${rust_target})
      args+=(--target=${zig_target})
      ;;
    *)
      args+=("\$arg")
      ;;
  esac
done

exec zig cc "\${args[@]}"
EOF
  chmod +x "$output"
}

find_llvm_lib() {
  if command -v llvm-lib >/dev/null 2>&1; then
    command -v llvm-lib
    return 0
  fi

  local brew_prefix=""
  if command -v brew >/dev/null 2>&1; then
    brew_prefix="$(brew --prefix llvm@21 2>/dev/null || true)"
    if [[ -n "$brew_prefix" && -x "$brew_prefix/bin/llvm-lib" ]]; then
      echo "$brew_prefix/bin/llvm-lib"
      return 0
    fi
  fi

  for candidate in \
    /opt/homebrew/opt/llvm@21/bin/llvm-lib \
    /usr/local/opt/llvm@21/bin/llvm-lib; do
    if [[ -x "$candidate" ]]; then
      echo "$candidate"
      return 0
    fi
  done

  return 1
}

configure_macos_zig_cross_cc() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  [[ -z "${CI:-}" ]] || return 0
  command -v zig >/dev/null 2>&1 || return 0

  TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tabctl-cross-XXXXXX")"

  if [[ -z "${CC_x86_64_unknown_linux_gnu:-}" ]]; then
    write_zig_cc_wrapper \
      "$TMP_DIR/zig-cc-linux" \
      "x86_64-unknown-linux-gnu" \
      "x86_64-linux-gnu"
    export CC_x86_64_unknown_linux_gnu="$TMP_DIR/zig-cc-linux"
  fi

  if [[ -z "${CC_x86_64_pc_windows_msvc:-}" ]]; then
    # libsqlite3-sys needs only a C compiler/archive step during cargo check.
    # Zig's MinGW target provides the C runtime headers that are unavailable
    # when compiling from macOS to Rust's MSVC target without Visual Studio.
    write_zig_cc_wrapper \
      "$TMP_DIR/zig-cc-windows-msvc" \
      "x86_64-pc-windows-msvc" \
      "x86_64-windows-gnu"
    export CC_x86_64_pc_windows_msvc="$TMP_DIR/zig-cc-windows-msvc"
  fi

  if [[ -z "${AR_x86_64_pc_windows_msvc:-}" ]]; then
    if llvm_lib="$(find_llvm_lib)"; then
      export AR_x86_64_pc_windows_msvc="$llvm_lib"
    else
      echo "WARN: llvm-lib not found; install Homebrew zig/llvm@21 if Windows cross-check fails." >&2
    fi
  fi
}

configure_macos_zig_cross_cc

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
