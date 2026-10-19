#!/usr/bin/env bash
# Runs `editor_node_view_real_tab` with trusted keys: builds it with
# RINCH_REAL_KEYS=1, starts Chrome (through CHROMEDRIVER) with a DevTools port and
# runs `real_keys.mjs` beside it. Needs node >= 22 (a global WebSocket).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
port="${RINCH_REAL_KEYS_PORT:-9555}"
: "${CHROMEDRIVER:?set CHROMEDRIVER to a chromedriver matching your Chrome}"
cd "$crate"
wasm="$(RINCH_REAL_KEYS=1 cargo test -p rinch-web --target wasm32-unknown-unknown \
  --test editor_node_view_real_tab --no-run --message-format=json 2>/dev/null \
  | grep -o '"executable":"[^"]*editor_node_view_real_tab[^"]*\.wasm"' | head -1 | cut -d'"' -f4)"
[ -n "$wasm" ] || { echo "build failed"; exit 1; }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"; [ -n "${helper:-}" ] && kill "$helper" 2>/dev/null || true' EXIT
cat > "$tmp/webdriver.json" <<JSON
{ "goog:chromeOptions": { "args": ["--remote-debugging-port=$port"] } }
JSON
node "$here/real_keys.mjs" "$port" &
helper=$!
cd "$tmp"
CHROMEDRIVER="$CHROMEDRIVER" wasm-bindgen-test-runner "$wasm" "$@"
