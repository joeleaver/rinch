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
# The helper's log goes to a file, shown when a test fails or the helper dies
# (#1839: it died silently between two test binaries, and the failing test said
# only that its click was never sent).
helper_log="$tmp/real_keys.log"
node "$here/real_keys.mjs" "$port" >"$helper_log" 2>&1 &
helper=$!
show_helper_log() {
  echo "---- real_keys.mjs log ($helper_log) ----"
  cat "$helper_log" || true
  echo "---- end of real_keys.mjs log ----"
}
cd "$tmp"
for wasm in "${wasms[@]}"; do
  if ! kill -0 "$helper" 2>/dev/null; then
    echo "real_keys.mjs is no longer running"
    show_helper_log
    exit 1
  fi
  if ! CHROMEDRIVER="$CHROMEDRIVER" wasm-bindgen-test-runner "$wasm" --include-ignored "$@"; then
    show_helper_log
    exit 1
  fi
done
