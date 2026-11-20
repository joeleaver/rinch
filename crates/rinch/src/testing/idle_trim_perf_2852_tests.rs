//! The `RINCH_PERF` lines of a software window's idle trim (#2852):
//! `trimmed` once when the trim of #2843 gives its buffers back, `untrimmed`
//! once at the next present, before that frame's own line, and neither for a
//! window that keeps its buffers (`IdleTrim::Never`).
//!
//! `RINCH_PERF` is read once per process, so the lines are read through
//! `window_perf::capture`, which is handed every line printed under it. The
//! frame lines are the harness's, from the same counters `end_frame` numbers
//! its line by.

use std::time::Duration;

use rinch_core::Signal;
use rinch_core::dom::RenderScope;
use rinch_core::element::{IdleTrim, WindowProps};

use super::TestApp;
use crate::window_perf::capture;

/// A shown software window with a box whose width a signal sets (no text),
/// trimmed as `trim` says, settled after its mount.
fn mounted(trim: IdleTrim) -> (TestApp, Signal<u32>) {
    let width = std::rc::Rc::new(std::cell::Cell::new(None));
    let w = width.clone();
    let mut app = TestApp::builder(move |scope: &mut RenderScope| {
        let wide = Signal::new(40_u32);
        w.set(Some(wide));
        let bar = scope.create_element("div");
        let b = bar.clone();
        scope.create_effect(move || {
            b.set_attribute("style", &format!("width: {}px; height: 10px", wide.get()));
        });
        bar
    })
    .window_props(WindowProps {
        idle_trim: trim,
        ..Default::default()
    })
    .mount();
    app.settle();
    let signal = width.get().expect("mounted");
    (app, signal)
}

/// The trim lines among `lines`.
fn trims(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .map(String::as_str)
        .filter(|l| l.ends_with("trimmed"))
        .collect()
}

#[test]
fn an_idle_window_says_trimmed_once_and_its_next_present_untrimmed_first() {
    capture::listen();
    let (mut app, wide) = mounted(IdleTrim::DEFAULT);
    let mount = capture::take();
    assert!(
        mount.iter().any(|l| l.starts_with("[PERF] frame ")),
        "the capture sees the mount's frame: {mount:?}"
    );
    assert!(trims(&mount).is_empty(), "the first present: {mount:?}");

    // Inside the period: kept. Off the period's own instant.
    app.advance_time(Duration::from_millis(1_900));
    assert_eq!(capture::take(), Vec::<String>::new(), "before the period");

    // Past it: trimmed, once, the main window in its frames' form.
    app.advance_time(Duration::from_millis(300));
    assert_eq!(capture::take(), ["[PERF] trimmed"], "past the period");
    app.advance_time(Duration::from_secs(10));
    assert_eq!(capture::take(), Vec::<String>::new(), "still idle");

    // The next present makes the buffers again, and says so first.
    wide.set(80);
    app.settle();
    let next = capture::take();
    assert_eq!(
        trims(&next),
        ["[PERF] untrimmed"],
        "the next present: {next:?}"
    );
    assert_eq!(next[0], "[PERF] untrimmed", "before its frame: {next:?}");
    assert!(
        next.get(1).is_some_and(|l| l.starts_with("[PERF] frame ")),
        "then its frame's line: {next:?}"
    );

    // The present after that has buffers already.
    wide.set(90);
    app.settle();
    let after = capture::take();
    assert!(!after.is_empty(), "it painted: {after:?}");
    assert!(trims(&after).is_empty(), "the present after: {after:?}");
}

#[test]
fn a_window_that_never_trims_says_neither() {
    capture::listen();
    let (mut app, wide) = mounted(IdleTrim::Never);
    app.advance_time(Duration::from_secs(30));
    wide.set(80);
    app.settle();
    let lines = capture::take();
    assert!(
        lines
            .iter()
            .filter(|l| l.starts_with("[PERF] frame "))
            .count()
            >= 2,
        "it painted before and after: {lines:?}"
    );
    assert!(trims(&lines).is_empty(), "{lines:?}");
}

/// Another window's lines name it by id, as its frames' do.
#[test]
fn another_windows_lines_name_it() {
    capture::listen();
    crate::window_perf::trimmed(Some("window 3"));
    crate::window_perf::untrimmed(Some("window 3"));
    assert_eq!(
        capture::take(),
        ["[PERF] window 3 trimmed", "[PERF] window 3 untrimmed"]
    );
}
