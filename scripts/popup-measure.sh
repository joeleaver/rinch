#!/bin/bash
# Measure how long a popup takes to open (#2552): the popup-menu example on a
# 40 px bar in a nested compositor, its menu, submenu and tooltip opened by a
# virtual pointer, and the `[PERF] window N first frame` lines it prints
# (each popup's time from open_popup to its first presented frame, and the
# steps in between). Run it inside an isolated session:
#
#   scripts/isolated-session.sh -- scripts/popup-measure.sh OUT_DIR
#
# OUT_DIR gets app.log (the app's output, with a `STEP …` line before each
# step), steps.log and the compositor's log. Settings, from the environment:
#
#   COMPOSITOR  sway (default; headless), labwc (headless) or kwin
#               (kwin_wayland --virtual)
#   SWAY_SCALE  sway's output scale (1 by default; 1.5 for a fractional one)
#   BAR_Y       the bar's top in logical pixels (680; 440 at scale 1.5),
#               with POPUP_INPUT_W/H the output's logical size for popup-input
#   BIN         the popup-menu binary (default target/release/popup-menu)
#   INPUT       the popup-input binary (scripts/popup-input)
#   RENDERER    RINCH_RENDERER for the app: software (default) or gpu
#   N           menus opened back to back (default 11)
#   IDLES       seconds idle before each later menu (default "5 60")
#   WLR_RENDERER for sway: pixman (default) or gles2 (a GPU the app's Vulkan
#               can present to; with pixman it gets llvmpipe)
#   MESA_CACHE  MESA_SHADER_CACHE_DIR for the app (the session's cache
#               directory is new, so cold, by default)
#   PROF        a command put before the app's (perf record …, gdb --args)
#
# On KWin the input goes through org_kde_kwin_fake_input, which KWin offers
# only to an executable a desktop file grants it to: the file is installed in
# the session's data directory before KWin starts.
set -u
OUT=${1:?usage: popup-measure.sh OUT_DIR}
mkdir -p "$OUT"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$ROOT/target/release/popup-menu}
INPUT=$(readlink -f "${INPUT:-$ROOT/scripts/popup-input/target/debug/popup-input}")
[ -x "$BIN" ] && [ -x "$INPUT" ] || { echo "popup-measure: no $BIN or $INPUT" >&2; exit 2; }
: "${RINCH_SESSION_DIR:?run it inside scripts/isolated-session.sh}"
S=$RINCH_SESSION_DIR

case ${COMPOSITOR:-sway} in
sway)
    printf 'output HEADLESS-1 resolution 1280x720 position 0 0 scale %s\ndefault_border none\n' \
        "${SWAY_SCALE:-1}" > "$S/sway.cfg"
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=${WLR_RENDERER:-pixman} \
        sway -c "$S/sway.cfg" > "$OUT/compositor.log" 2>&1 &
    ;;
labwc)
    mkdir -p "$S/labwc"
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=${WLR_RENDERER:-pixman} \
        labwc -C "$S/labwc" > "$OUT/compositor.log" 2>&1 &
    ;;
kwin)
    mkdir -p "$XDG_DATA_HOME/applications"
    printf '[Desktop Entry]\nType=Application\nName=popup-input\nExec=%s\nNoDisplay=true\nX-KDE-Wayland-Interfaces=org_kde_kwin_fake_input\n' \
        "$INPUT" > "$XDG_DATA_HOME/applications/popup-input.desktop"
    kwin_wayland --virtual --socket wayland-k --width 1280 --height 720 --no-lockscreen \
        --no-global-shortcuts --no-kactivities > "$OUT/compositor.log" 2>&1 < /dev/null &
    ;;
*) echo "popup-measure: COMPOSITOR is sway, labwc or kwin" >&2; exit 2 ;;
esac
COMP=$!
trap 'kill $COMP 2>/dev/null; wait' EXIT
for _ in $(seq 1 100); do ls "$XDG_RUNTIME_DIR"/wayland-* > /dev/null 2>&1 && break; sleep 0.1; done
WAYLAND_DISPLAY=$(basename "$(ls "$XDG_RUNTIME_DIR"/wayland-* | grep -v lock | head -1)")
export WAYLAND_DISPLAY
[ -n "${MESA_CACHE:-}" ] && export MESA_SHADER_CACHE_DIR=$MESA_CACHE
env RUST_LOG="${RUST_LOG:-info}" POPUP_SCRIPT=1 POPUP_SCRIPT_MS="${APP_MS:-100000}" RINCH_PERF=1 \
    RINCH_RENDERER="${RENDERER:-software}" ${PROF:-} "$BIN" >> "$OUT/app.log" 2>&1 &
APP=$!
sleep 2
mkfifo "$S/input.fifo"
"$INPUT" < "$S/input.fifo" 2> "$OUT/input.log" &
INP=$!
exec 9> "$S/input.fifo"
trap 'exec 9>&-; kill $APP $INP $COMP 2>/dev/null; wait' EXIT
sleep 1.5
B=${BAR_Y:-680}
step() { echo "$(date +%s.%N) STEP $*" >> "$OUT/steps.log"; echo "STEP $*" >> "$OUT/app.log"; }
input() { echo "$*" >&9; }
click() { input move 30 $((B + 20)) sleep 50 click; }
step "menu 1"; click; sleep 1
MY=$(grep 'placed menu' "$OUT/app.log" | tail -1 | sed -E 's/.* y=(-?[0-9.]+) .*/\1/')
MH=$(grep 'placed menu' "$OUT/app.log" | tail -1 | sed -E 's/.* h=([0-9.]+) .*/\1/')
MORE_Y=$(awk -v b="$B" -v y="${MY:-0}" -v h="${MH:-0}" 'BEGIN { printf "%d", b + y + h - 50 }')
step "submenu 1"; input move 60 $MORE_Y; sleep 1
step "escape"; input key 1; sleep 0.8
step "close"; click; sleep 0.8
for i in $(seq 2 "${N:-11}"); do
    step "menu $i"; click; sleep 0.5
    step "close"; click; sleep 0.5
done
for IDLE in ${IDLES-5 60}; do
    step "idle ${IDLE}s"; sleep "$IDLE"
    step "menu after ${IDLE}s idle"; click; sleep 0.8
    step "submenu after ${IDLE}s idle"; input move 60 $MORE_Y; sleep 0.8
    step "close"; click; sleep 0.8
done
step end
