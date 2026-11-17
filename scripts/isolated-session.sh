#!/bin/bash
# Run a command in a session isolated from the desktop's: the one way rinch
# runs a test that needs a nested compositor or a private D-Bus session bus
# (issue #2429).
#
#   scripts/isolated-session.sh [--kwin [KWIN_OPTION...]] -- COMMAND [ARG...]
#
# The session has:
#
# - its own XDG_RUNTIME_DIR, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME and
#   XDG_STATE_HOME, under a fresh directory in /tmp (short, because a Wayland
#   socket's path must fit in 108 bytes), and HOME set to a directory there
#   (CARGO_HOME and RUSTUP_HOME keep the real ones, so cargo still works; a
#   debug-enabled app's ~/.rinch/debug discovery file lands in the session's
#   home, out of the MCP server's sight);
# - none of the desktop's WAYLAND_DISPLAY, WAYLAND_SOCKET, DISPLAY,
#   XAUTHORITY or DBUS_SESSION_BUS_ADDRESS: they are removed before anything
#   starts, the bus daemon included;
# - a dbus-daemon of its own, on a configuration with no <servicedir>, so no
#   service is ever started on demand. Whatever a test needs on the bus (a
#   fake portal, KScreen's backend) it starts itself, inside the session;
# - with --kwin, a headless KWin (kwin_wayland --virtual) on that bus, which
#   runs COMMAND as its session and exits when it exits. KWIN_OPTIONs are
#   passed to KWin (--xwayland, --output-count 2, --width 1920 ...); with
#   --xwayland COMMAND gets KWin's nested DISPLAY and XAUTHORITY. KWin starts
#   kcminit when its Xwayland starts, and kcminit aborts (dumping core) if the
#   X server goes away while it is still starting, so once Xwayland runs the
#   session waits, at most 15 s after COMMAND exits, for kcminit to finish
#   (a wrapper first on PATH runs the real kcminit and notes when it is done);
# - one process group, which is ended as a whole when COMMAND exits. The
#   script then checks that nothing of the session is left (in the group, or
#   anywhere with the session's runtime directory) and fails if anything is.
#
# COMMAND sees RINCH_SESSION_DIR, the session's directory, and can leave logs
# there. The directory is removed at the end unless RINCH_SESSION_KEEP=1. The
# script exits with COMMAND's status, or 70 when the session could not be
# set up or did not end cleanly.
#
# Never run a nested compositor or a test bus any other way: `dbus-run-session`
# and `dbus-launch` start a bus that activates the desktop's services, with
# the environment they were started from. On 2026-11-10 such sessions started
# the desktop portals and the secret service with the real session's
# environment (its runtime directory, Wayland display and home); one of them
# crashed 391 times, and orphans were left running.

set -u

die() {
    echo "isolated-session: $*" >&2
    exit 70
}

# ── Inside the session (the leader of its own process group) ───────────────

if [ "${1:-}" = "--inner" ]; then
    T=$2
    shift 2
    kwin=0
    kwin_options=()
    if [ "${1:-}" = "--kwin" ]; then
        kwin=1
        shift
        while [ $# -gt 0 ] && [ "$1" != "--" ]; do
            kwin_options+=("$1")
            shift
        done
    fi
    [ "${1:-}" = "--" ] || die "no -- before the command"
    shift

    echo $$ > "$T/pgid"
    mkdir -m 700 "$T/run" "$T/config" "$T/data" "$T/cache" "$T/state" "$T/home" ||
        die "cannot create the session's directories"

    unset WAYLAND_DISPLAY WAYLAND_SOCKET DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS
    # cargo and rustup keep working with HOME moved.
    export CARGO_HOME=${CARGO_HOME:-$HOME/.cargo} RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
    export XDG_RUNTIME_DIR=$T/run XDG_CONFIG_HOME=$T/config XDG_DATA_HOME=$T/data \
        XDG_CACHE_HOME=$T/cache XDG_STATE_HOME=$T/state HOME=$T/home \
        RINCH_SESSION_DIR=$T

    # A session bus that serves only what connects to it: no <servicedir>, so
    # nothing is activated; anyone may own any name; AppArmor mediation off
    # (Ubuntu confines some clients with rules written for the real bus).
    cat > "$T/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$T/run</listen>
  <auth>EXTERNAL</auth>
  <apparmor mode="disabled"/>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
    dbus-daemon --config-file="$T/bus.conf" --nofork --print-address=3 \
        3> "$T/bus.address" > "$T/dbus.log" 2>&1 < /dev/null &
    for _ in $(seq 100); do
        [ -s "$T/bus.address" ] && break
        sleep 0.1
    done
    [ -s "$T/bus.address" ] || die "dbus-daemon did not start (see $T/dbus.log)"
    DBUS_SESSION_BUS_ADDRESS=$(head -n1 "$T/bus.address")
    export DBUS_SESSION_BUS_ADDRESS

    if [ $kwin = 0 ]; then
        "$@"
        echo $? > "$T/status"
        exit 0
    fi

    # KWin starts kcminit by name when Xwayland comes up; a wrapper first on
    # PATH says when it has finished (see command.sh below).
    real_kcminit=$(command -v kcminit)
    if [ -n "$real_kcminit" ]; then
        mkdir -m 700 "$T/bin"
        {
            echo '#!/bin/sh'
            printf '%q "$@"\n' "$real_kcminit"
            echo 'status=$?'
            printf ': > %q\n' "$T/kcminit.done"
            echo 'exit $status'
        } > "$T/bin/kcminit"
        chmod +x "$T/bin/kcminit"
        export PATH=$T/bin:$PATH
    fi

    # KWin runs COMMAND as its session (--exit-with-session takes one path, so
    # COMMAND goes in a wrapper) and exits when it does.
    # KWin's own output goes to kwin.log; COMMAND's to this script's.
    {
        echo '#!/bin/bash'
        # Appending: reopening a file this script writes to must not truncate it.
        printf 'exec >> %q 2>> %q\n' "/proc/$$/fd/1" "/proc/$$/fd/2"
        printf '%q ' "$@"
        echo
        printf 'echo $? > %q\n' "$T/status"
        # With Xwayland, KWin runs kcminit as the X server starts (Xwayland
        # starts at the first X client), and kcminit aborts, dumping core, when
        # its X server goes away while it is still starting: once Xwayland
        # runs, let kcminit finish (at most 15 s) before the session ends.
        printf 'if [ -n "${DISPLAY:-}" ] && [ -x %q ] && pgrep -g 0 -x Xwayland > /dev/null; then\n' "$T/bin/kcminit"
        printf '    for _ in $(seq 150); do [ -e %q ] && break; sleep 0.1; done\n' "$T/kcminit.done"
        echo 'fi'
    } > "$T/command.sh"
    chmod +x "$T/command.sh"
    kwin_wayland --virtual --socket wayland-0 --no-lockscreen --no-global-shortcuts \
        --no-kactivities "${kwin_options[@]}" --exit-with-session "$T/command.sh" \
        > "$T/kwin.log" 2>&1 < /dev/null
    exit 0
fi

# ── Outside: start the session, wait for it, end it, check it ──────────────

[ $# -gt 0 ] || die "usage: $0 [--kwin [KWIN_OPTION...]] -- COMMAND [ARG...]"

T=$(mktemp -d /tmp/rinch-session-XXXXXX) || die "cannot create the session directory"
chmod 700 "$T"

# setsid makes the inner script the leader of a new process group (and
# session), so the whole session can be ended with one signal.
setsid "$0" --inner "$T" "$@" &
leader=$!
# Ctrl-C reaches this script only (the session is its own process group):
# end the session then, as when the command exits.
interrupted=0
trap 'interrupted=1' INT TERM HUP
wait "$leader"

pgid=$(cat "$T/pgid" 2>/dev/null) || die "the session did not start (see $T)"
[ "$pgid" = "$leader" ] || die "the session's process group is $pgid, not $leader"

# End what is left of the group: SIGTERM, then SIGKILL after 5 s.
kill -TERM -- "-$pgid" 2>/dev/null
for _ in $(seq 50); do
    pgrep -g "$pgid" > /dev/null || break
    sleep 0.1
done
kill -KILL -- "-$pgid" 2>/dev/null
sleep 0.2

left=$(pgrep -a -g "$pgid")
# A process that left the group (setsid, double fork) still has the session's
# runtime directory in its environment.
strays=$(grep -lszax "XDG_RUNTIME_DIR=$T/run" /proc/[0-9]*/environ 2>/dev/null)
if [ -n "$left$strays" ]; then
    echo "isolated-session: processes of the session are still running:" >&2
    [ -n "$left" ] && echo "$left" >&2
    for f in $strays; do
        p=${f#/proc/}
        p=${p%/environ}
        echo "$p $(cat "/proc/$p/cmdline" 2>/dev/null | tr '\0' ' ')" >&2
    done
    echo "isolated-session: kept $T" >&2
    exit 70
fi

if [ $interrupted = 1 ]; then
    echo "isolated-session: interrupted; the session was ended (kept $T)" >&2
    exit 130
fi
status=$(cat "$T/status" 2>/dev/null) || {
    echo "isolated-session: the command did not run (see $T)" >&2
    exit 70
}
if [ "${RINCH_SESSION_KEEP:-0}" = 1 ]; then
    echo "isolated-session: kept $T" >&2
else
    rm -rf "$T"
fi
exit "$status"
