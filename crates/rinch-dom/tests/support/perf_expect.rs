//! Exact whole-frame counter assertions, shared by the performance regression
//! scenarios (`perf_regression_scenarios.rs`). Included with `#[path]`, so it is
//! not a test target of its own.
//!
//! The contract is the one `perf_counter_baselines.rs` set in #877: every
//! non-`time_*` counter is compared exactly, and a counter the baseline does not
//! list must be `0`. A rise is a path that got more expensive; a fall is either
//! a fix (update the baseline and say why in the PR) or an increment that
//! stopped counting.
//!
//! **Updating a baseline.** A failing assertion prints the frame it saw in the
//! exact form the baseline is written in, ready to paste over the old one.
//! `PERF_BASELINE_PRINT=1` prints every scenario's frame, passing or not:
//!
//! ```text
//! PERF_BASELINE_PRINT=1 cargo test -p rinch-dom --test perf_regression_scenarios -- --nocapture
//! ```

#![allow(dead_code)]

use rinch_dom::perf::{Counter, FrameStats};

/// The frame as a paste-ready baseline: one `(Counter, value),` line per
/// non-zero, non-timing counter, in declaration order.
pub fn baseline_text(stats: &FrameStats) -> String {
    let mut s = String::new();
    for (c, v) in stats.iter() {
        if v != 0 && !c.name().starts_with("time_") {
            s.push_str(&format!("            ({c:?}, {v}),\n"));
        }
    }
    s
}

/// Assert the whole frame: every counter in `baseline` has exactly that value,
/// every other non-timing counter is `0`.
#[track_caller]
pub fn expect(name: &str, stats: &FrameStats, baseline: &[(Counter, u64)]) {
    if std::env::var_os("PERF_BASELINE_PRINT").is_some() {
        println!("{name}:\n{}", baseline_text(stats));
    }
    for (i, (c, _)) in baseline.iter().enumerate() {
        assert!(
            !baseline[..i].iter().any(|(d, _)| d == c),
            "{name}: {} listed twice in its baseline",
            c.name()
        );
        assert!(
            !c.name().starts_with("time_"),
            "{name}: {} is a timing and cannot be baselined",
            c.name()
        );
    }
    let mut diffs = Vec::new();
    for (c, v) in stats.iter() {
        if c.name().starts_with("time_") {
            continue;
        }
        let want = baseline
            .iter()
            .find(|(b, _)| *b == c)
            .map_or(0, |(_, w)| *w);
        if v != want {
            diffs.push(format!("{}: {v} (baseline {want})", c.name()));
        }
    }
    assert!(
        diffs.is_empty(),
        "{name}: the frame differs from its recorded baseline.\n  {}\n\
         A rise is a path that got more expensive; a fall to 0 may be an \
         increment that stopped counting. If a change moved these on purpose, \
         paste the frame below over the baseline and say why in the PR.\n\
         Actual frame:\n{}",
        diffs.join("\n  "),
        baseline_text(stats)
    );
}
