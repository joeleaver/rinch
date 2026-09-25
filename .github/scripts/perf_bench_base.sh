#!/usr/bin/env bash
# Runs `crates/rinch-bench` on the base checkout for the `Perf` workflow
# (`.github/workflows/perf.yml`), trying two copies of the benchmark sources:
#
#   1. The HEAD's copy, copied over the base. Both sides then run exactly the
#      same scenarios, so every Δ is the library alone.
#   2. If that does not build or run on the base -- usually because the PR adds
#      a benchmark that calls an API the same PR introduces (issue #1036) -- the
#      BASE's own copy, restored along with the base's Cargo.lock. Every
#      benchmark both copies define is still compared; one only the head
#      defines is reported as `new`. A benchmark whose scenario the PR changed
#      then compares two different scenarios, which the report says.
#
# Only when neither copy runs is there no baseline (`ran=false`); the compare
# step fails the check for that when the base has the crate.
#
# Inputs (environment): BASE_DIR, HEAD_DIR (the two checkouts), OUT (the base's
# Gungraun JSON lines), BENCH_ARGS, CARGO (default `cargo`; the self-test
# substitutes a fake), GITHUB_OUTPUT. Outputs:
#   has_crate  true | false   the base checkout has `crates/rinch-bench`
#   ran        true | false   OUT holds a complete base run
#   sources    head | base | none   which copy produced OUT
#
# Self-test: `.github/scripts/test_perf_scripts.py`.
set -uo pipefail

: "${BASE_DIR:?}" "${HEAD_DIR:?}" "${OUT:?}"
CARGO=${CARGO:-cargo}
GITHUB_OUTPUT=${GITHUB_OUTPUT:-/dev/null}
BENCH_ARGS=${BENCH_ARGS:-}

crate="$BASE_DIR/crates/rinch-bench"
saved=$(mktemp -d)
trap 'rm -rf "$saved"' EXIT

has_crate=false
if [ -d "$crate" ]; then
  has_crate=true
  cp -a "$crate" "$saved/rinch-bench"
fi
[ -f "$BASE_DIR/Cargo.lock" ] && cp -a "$BASE_DIR/Cargo.lock" "$saved/Cargo.lock"
echo "has_crate=$has_crate" >> "$GITHUB_OUTPUT"

run_bench() {
  # Not --locked: with the head's sources, the base's Cargo.lock may not know
  # a dependency they add.
  # shellcheck disable=SC2086  # BENCH_ARGS is a list of flags
  (cd "$BASE_DIR" && $CARGO bench -p rinch-bench -- $BENCH_ARGS) > "$OUT"
}

finish() {
  echo "ran=$1" >> "$GITHUB_OUTPUT"
  echo "sources=$2" >> "$GITHUB_OUTPUT"
  exit 0
}

rm -rf "$crate"
echo "::group::base with the head's benchmark sources"
if cp -r "$HEAD_DIR/crates/rinch-bench" "$crate" && run_bench; then
  echo "::endgroup::"
  finish true head
fi
echo "::endgroup::"

if [ "$has_crate" != true ]; then
  echo "::warning title=perf baseline::the head's benchmarks did not build or run on the base, which has no crates/rinch-bench of its own"
  : > "$OUT"
  finish false none
fi

echo "::warning title=perf baseline::the head's benchmark sources did not build or run on the base; running the base's own copy, so benchmarks only the head defines are not compared"
rm -rf "$crate"
cp -a "$saved/rinch-bench" "$crate"
if [ -f "$saved/Cargo.lock" ]; then
  cp -a "$saved/Cargo.lock" "$BASE_DIR/Cargo.lock"
fi
echo "::group::base with its own benchmark sources"
if run_bench; then
  echo "::endgroup::"
  finish true base
fi
echo "::endgroup::"
echo "::warning title=perf baseline::the benchmarks did not build or run on the base with either copy of the sources"
: > "$OUT"
finish false none
