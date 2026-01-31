#!/usr/bin/env bash
set -euo pipefail

EXT_ID="${1:-${TABARCHIVE_EXTENSION_ID:-}}"
if [[ -z "$EXT_ID" ]]; then
  echo "Usage: $0 <extension-id>"
  echo "Or set TABARCHIVE_EXTENSION_ID in the environment."
  exit 1
fi

if [[ ! "$EXT_ID" =~ ^[a-p]{32}$ ]]; then
  echo "Extension ID looks unusual: $EXT_ID"
  echo "Expected 32 lowercase characters (a-p)."
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_JS="$ROOT/host/host.js"

if [[ ! -f "$HOST_JS" ]]; then
  echo "Host script not found at $HOST_JS"
  echo "Run: npm run build"
  exit 1
fi

NODE_BIN="${TABARCHIVE_NODE:-$(command -v node || true)}"
if [[ -z "$NODE_BIN" ]]; then
  echo "Node binary not found. Set TABARCHIVE_NODE to an absolute path."
  exit 1
fi

if [[ ! -x "$NODE_BIN" ]]; then
  echo "Node binary not executable: $NODE_BIN"
  exit 1
fi

WRAPPER_DIR="$HOME/.tabarchive"
WRAPPER_PATH="$WRAPPER_DIR/tabarchive-host.sh"

mkdir -p "$WRAPPER_DIR"
cat > "$WRAPPER_PATH" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$NODE_BIN" "$HOST_JS"
EOF
chmod +x "$WRAPPER_PATH"

DEFAULT_POLICY="$ROOT/config/policy.example.json"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
POLICY_DIR="$CONFIG_HOME/tabctl"
POLICY_PATH="$POLICY_DIR/policy.json"
if [[ -f "$DEFAULT_POLICY" && ! -f "$POLICY_PATH" ]]; then
  mkdir -p "$POLICY_DIR"
  cp "$DEFAULT_POLICY" "$POLICY_PATH"
  echo "Default policy installed at: $POLICY_PATH"
fi

HOST_DIR="$HOME/Library/Application Support/Microsoft Edge/NativeMessagingHosts"
MANIFEST_PATH="$HOST_DIR/com.erwinkroon.tabctl.json"

mkdir -p "$HOST_DIR"

cat > "$MANIFEST_PATH" <<EOF
{
  "name": "com.erwinkroon.tabctl",
  "description": "Tab archive native host",
  "path": "$WRAPPER_PATH",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://$EXT_ID/"]
}
EOF

echo "Native host manifest written to: $MANIFEST_PATH"
echo "Host path: $WRAPPER_PATH"
echo "Node path: $NODE_BIN"
