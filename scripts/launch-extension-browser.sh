#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXTENSION_DIR="${TABCTL_EXTENSION_DIR:-"${ROOT_DIR}/extension"}"
PROFILE_DIR="${TABCTL_PROFILE_DIR:-"${ROOT_DIR}/.tabctl/profile"}"
REMOTE_DEBUGGING_PORT="${TABCTL_REMOTE_DEBUGGING_PORT:-9222}"
BROWSER_BIN="${TABCTL_BROWSER_BIN:-""}"

if [[ -z "${BROWSER_BIN}" ]]; then
  for candidate in msedge microsoft-edge google-chrome chromium chromium-browser; do
    if command -v "${candidate}" >/dev/null 2>&1; then
      BROWSER_BIN="${candidate}"
      break
    fi
  done
fi

if [[ -z "${BROWSER_BIN}" ]]; then
  echo "Browser binary not found. Set TABCTL_BROWSER_BIN to a Chrome/Edge executable." >&2
  exit 1
fi

mkdir -p "${PROFILE_DIR}"

exec "${BROWSER_BIN}" \
  --user-data-dir="${PROFILE_DIR}" \
  --disable-extensions-except="${EXTENSION_DIR}" \
  --load-extension="${EXTENSION_DIR}" \
  --remote-debugging-port="${REMOTE_DEBUGGING_PORT}" \
  --no-first-run \
  --no-default-browser-check \
  ${TABCTL_BROWSER_ARGS:-}
