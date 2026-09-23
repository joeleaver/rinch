## Summary

<!-- What changed and why. Link the issue: "Fixes #NNN" only if it fully closes it. -->

## Testing

<!-- What you ran (RINCH_TREE_CHECK=1 cargo test ..., clippy, a live app via MCP) and what it showed. -->

## Performance

The exact counter baselines are the performance contract: every counter of each
scenario's frame is asserted exactly, so a PR that makes a path more expensive
fails them. See `docs/src/guide/performance.md#the-baselines-are-the-contract`.

- [ ] No performance baseline changed.
- [ ] A baseline changed. For **each** scenario that moved, the counters that
      moved, old → new, and why (a fix proving itself, or a cost this PR accepts
      and explains):

<!--
e.g. perf_regression_tests::a_wheel_scroll_…: paint_nodes_visited 504 → 24 —
paint now culls a scroller's rows outside its viewport.
Regenerate with PERF_BASELINE_PRINT=1 (a failing assertion also prints the
paste-ready frame).
-->

- [ ] The `Perf` job's instruction counts rose past the threshold, and this PR
      carries the `perf-regression-accepted` label: which benchmark, by how
      much, and why the cost is accepted.
