#!/usr/bin/env bash
# The `Compare` step of `.github/workflows/perf.yml`: turns the two benchmark
# steps' outputs into `perf_compare.py` flags, writes the report and the job
# summary, and sets `status` (0 pass, 1 regression / no usable baseline, 2 a bug
# in perf_compare.py, 3 the head's benchmarks did not run).
#
# Inputs (environment, set by perf.yml): RUNNER_TEMP (holds base.jsonl,
# head.jsonl, base.sources-diff; report.md is written there), THRESHOLD,
# ACCEPTED, BASE_SHA, HEAD_SHA, PR_HEAD_SHA, BASE_HAS_CRATE, BASE_RAN,
# BASE_SOURCES, BASE_SOURCES_CHANGED, HEAD_OUTCOME, RUN_URL, GITHUB_OUTPUT,
# GITHUB_STEP_SUMMARY. The self-test runs this script with the env mapping
# read out of perf.yml itself (`.github/scripts/test_perf_scripts.py`).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
report="$RUNNER_TEMP/report.md"
if [ "${HEAD_OUTCOME:-}" != "success" ] || [ ! -s "$RUNNER_TEMP/head.jsonl" ]; then
  {
    echo "<!-- rinch-perf-bench -->"
    echo "## Instruction counts (Callgrind \`Ir\`)"
    echo
    echo "❌ The benchmarks did not build or run on the merge commit \`${HEAD_SHA:0:7}\`"
    echo "(step outcome: \`${HEAD_OUTCOME:-not run}\`), so there is nothing to compare."
    echo "See [the run](${RUN_URL:-})."
  } > "$report"
  cat "$report" >> "$GITHUB_STEP_SUMMARY"
  echo "status=3" >> "$GITHUB_OUTPUT"
  exit 0
fi
args=(--head "$RUNNER_TEMP/head.jsonl" --threshold "${THRESHOLD:-3}"
      --head-label "merge ${HEAD_SHA:0:7}"
      --base "$RUNNER_TEMP/base.jsonl"
      --base-label "main ${BASE_SHA:0:7}"
      --note "Base is the merge commit's first parent (the tip of \`main\` the PR was merged onto); head is the merge commit (PR head \`${PR_HEAD_SHA:0:7}\`)."
      --out "$report")
if [ "${BASE_RAN:-}" != "true" ] && [ "${BASE_HAS_CRATE:-}" = "true" ]; then
  args+=(--base-failed)
fi
if [ "${BASE_SOURCES:-}" = "base" ]; then
  args+=(--base-own-sources)
  if [ "${BASE_SOURCES_CHANGED:-}" = "true" ]; then
    args+=(--base-sources-diff "$RUNNER_TEMP/base.sources-diff")
  fi
fi
if [ "${ACCEPTED:-}" = "true" ]; then
  args+=(--accepted)
fi
rm -f "$report"
set +e
python3 "$here/perf_compare.py" "${args[@]}"
status=$?
set -e
if [ "$status" -eq 2 ]; then
  {
    echo "<!-- rinch-perf-bench -->"
    echo "## Instruction counts (Callgrind \`Ir\`)"
    echo
    echo "❌ \`perf_compare.py\` failed (exit 2: a malformed input or a crash in the script)."
    echo "That is a bug in the workflow, not a regression. See [the run](${RUN_URL:-})."
  } > "$report"
fi
if [ -s "$report" ]; then
  cat "$report" >> "$GITHUB_STEP_SUMMARY"
fi
echo "status=$status" >> "$GITHUB_OUTPUT"
