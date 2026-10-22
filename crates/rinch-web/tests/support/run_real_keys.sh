#!/usr/bin/env bash
# Runs the trusted-input tests (`editor_node_view_real_tab`, `real_touch_1582_1588`,
# `clipboard_copy_then`, `real_touch_picker_1771`):
# builds each with RINCH_REAL_KEYS=1, starts Chrome (through CHROMEDRIVER) with a
# DevTools port and runs `real_keys.mjs` beside it, which sends the keys and touches
# the page asks for. Needs node >= 22 (a global WebSocket). Arguments after the
# script are passed to every test runner.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
port="${RINCH_REAL_KEYS_PORT:-9555}"
: "${CHROMEDRIVER:?set CHROMEDRIVER to a chromedriver matching your Chrome}"
tests=(editor_node_view_real_tab real_touch_1582_1588 clipboard_copy_then real_touch_picker_1771)
cd "$crate"
wasms=()
for t in "${tests[@]}"; do
  # Build first with the ordinary output, so a compile error reaches the log as the
  # compiler wrote it (the JSON run below would bury it in stdout). The JSON run then
  # compiles nothing and only names the test binary.
  RINCH_REAL_KEYS=1 cargo test -p rinch-web --target wasm32-unknown-unknown \
    --test "$t" --no-run
  wasm="$(RINCH_REAL_KEYS=1 cargo test -p rinch-web --target wasm32-unknown-unknown \
    --test "$t" --no-run --message-format=json \
    | grep -o "\"executable\":\"[^\"]*$t[^\"]*\\.wasm\"" | head -1 | cut -d'"' -f4)"
  [ -n "$wasm" ] || { echo "the build named no $t wasm"; exit 1; }
  wasms+=("$wasm")
done
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"; [ -n "${helper:-}" ] && kill "$helper" 2>/dev/null || true' EXIT
cat > "$tmp/webdriver.json" <<JSON
{ "goog:chromeOptions": { "args": ["--remote-debugging-port=$port"] } }
JSON
node "$here/real_keys.mjs" "$port" &
helper=$!
cd "$tmp"
for wasm in "${wasms[@]}"; do
  CHROMEDRIVER="$CHROMEDRIVER" wasm-bindgen-test-runner "$wasm" --include-ignored "$@"
done
