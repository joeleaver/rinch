//! The reduced-motion preference reaches a running app (#2209): the app's
//! override restyles the window on its next frame, through the shell's
//! per-frame comparison (`App::adopt_reduced_motion`), and the reactive getter
//! re-runs what reads it.
//!
//! One test in its own binary: the preference is process-wide, and a second
//! test running beside this one would see it change.

#![cfg(all(feature = "testing", feature = "windowing"))]

use rinch::prelude::*;
use rinch::testing::TestApp;

#[test]
fn the_override_restyles_a_running_window_and_wakes_the_getter() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                style {
                    ".t { width: 100px; height: 10px }
                     @media (prefers-reduced-motion: reduce) { .t { width: 7px } }"
                }
                div { class: "t", id: "t" }
                p { id: "flag", {|| if rinch::prefers_reduced_motion() { "reduce" } else { "motion" }} }
            }
        }
    });
    app.settle();
    assert_eq!(app.bounds("#t").width, 100.0);
    assert_eq!(app.text("#flag"), "motion");

    rinch::set_reduced_motion(Some(true));
    app.settle();
    assert_eq!(
        app.bounds("#t").width,
        7.0,
        "the override reaches the open window's sheets on its next frame"
    );
    assert_eq!(
        app.text("#flag"),
        "reduce",
        "and re-runs what reads the getter"
    );

    rinch::set_reduced_motion(None);
    app.settle();
    assert_eq!(
        app.bounds("#t").width,
        100.0,
        "back to the OS: no preference here"
    );
    assert_eq!(app.text("#flag"), "motion");
}
