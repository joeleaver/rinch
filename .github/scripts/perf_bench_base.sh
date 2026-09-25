#!/usr/bin/env bash
# Current (pre-#1036) base-benchmark logic, extracted verbatim from perf.yml.
set -uo pipefail
BASE_DIR=${BASE_DIR:?} HEAD_DIR=${HEAD_DIR:?} OUT=${OUT:?}
CARGO=${CARGO:-cargo}
GITHUB_OUTPUT=${GITHUB_OUTPUT:-/dev/null}
if [ -d "$BASE_DIR/crates/rinch-bench" ]; then
  echo "has_crate=true" >> "$GITHUB_OUTPUT"
else
  echo "has_crate=false" >> "$GITHUB_OUTPUT"
fi
rm -rf "$BASE_DIR/crates/rinch-bench"
cp -r "$HEAD_DIR/crates/rinch-bench" "$BASE_DIR/crates/rinch-bench"
if (cd "$BASE_DIR" && $CARGO bench -p rinch-bench -- ${BENCH_ARGS:-}) > "$OUT"; then
  echo "ran=true" >> "$GITHUB_OUTPUT"
else
  : > "$OUT"
  echo "ran=false" >> "$GITHUB_OUTPUT"
fi
