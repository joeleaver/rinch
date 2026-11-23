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

/// A redraw that presents nothing (a configure, an input method's or a focus
/// change's, at the size and frame the surface shows) past the period serves
/// the trim itself: it says `trimmed` too, before its frame's line, so the
/// next present's `untrimmed` is never the first word of a trim.
#[test]
fn a_trim_served_by_a_redraw_says_trimmed() {
    capture::listen();
    let (mut app, wide) = mounted(IdleTrim::DEFAULT);
    capture::take();

    // The clock passes the period with no turn of this window's loop (another
    // window's wake moved it), then the window is redrawn as it was.
    let now = virtual_time_now();
    rinch_core::timer::virtual_time::advance_to(now + Duration::from_millis(2_500));
    app.paint();
    let mid = capture::take();
    assert_eq!(trims(&mid), ["[PERF] trimmed"], "the redraw: {mid:?}");
    assert_eq!(mid[0], "[PERF] trimmed", "before its frame: {mid:?}");
    assert!(
        mid.get(1).is_some_and(|l| l.starts_with("[PERF] frame ")),
        "then its frame's line: {mid:?}"
    );

    // Once, not again at the next wake.
    app.advance_time(Duration::from_secs(5));
    assert_eq!(capture::take(), Vec::<String>::new(), "still idle");

    wide.set(80);
    app.settle();
    let next = capture::take();
    assert_eq!(
        trims(&next),
        ["[PERF] untrimmed"],
        "the next present: {next:?}"
    );
}

/// Another window's lines name it by id, as its frames' do.
#[test]
fn another_windows_lines_name_it() {
    capture::listen();
    let (mut app, wide) = mounted(IdleTrim::DEFAULT);
    app.doc()
        .borrow_mut()
        .tree
        .perf
        .set_label(Some("window 3".into()));
    app.advance_time(Duration::from_secs(3));
    wide.set(80);
    app.settle();
    let lines = capture::take();
    assert_eq!(
        trims(&lines),
        ["[PERF] window 3 trimmed", "[PERF] window 3 untrimmed"],
        "{lines:?}"
    );
}

/// The virtual clock's time now.
fn virtual_time_now() -> Duration {
    rinch_core::timer::virtual_time::elapsed().expect("a TestApp's clock")
}

/// Whether and when a window will trim (#2917): a software window says
/// `idle_trim=<period>ms` or `idle_trim=never` once its presentation is made,
/// before its first frame's line, so a reader knows which windows will say
/// `trimmed`. Off the default period, so a line that printed the default
/// whatever the props said would fail.
#[test]
fn a_window_says_its_idle_trim_before_its_first_frame() {
    for (trim, said) in [
        (IdleTrim::DEFAULT, "[PERF] idle_trim=2000ms"),
        (
            IdleTrim::After(Duration::from_millis(750)),
            "[PERF] idle_trim=750ms",
        ),
        (
            IdleTrim::After(Duration::from_micros(1_500)),
            "[PERF] idle_trim=1.5ms",
        ),
        (IdleTrim::Never, "[PERF] idle_trim=never"),
    ] {
        capture::listen();
        let (mut app, wide) = mounted(trim);
        let mount = capture::take();
        let policy: Vec<&str> = mount
            .iter()
            .map(String::as_str)
            .filter(|l| l.contains("idle_trim="))
            .collect();
        assert_eq!(policy, [said], "{trim:?}: {mount:?}");
        let first_frame = mount.iter().position(|l| l.starts_with("[PERF] frame "));
        let at = mount.iter().position(|l| l == said);
        assert!(
            at.is_some() && first_frame.is_some() && at < first_frame,
            "{trim:?}: said before the first frame: {mount:?}"
        );

        // Said once: not again at the next present, nor after a trim.
        app.advance_time(Duration::from_secs(10));
        wide.set(80);
        app.settle();
        let later = capture::take();
        assert!(
            !later.iter().any(|l| l.contains("idle_trim=")),
            "{trim:?}: once: {later:?}"
        );
    }
}
