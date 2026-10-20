//! #940: the history's typing-merge window is armed by the handle.
//!
//! `HistoryPlugin` merges consecutive typing transactions whose
//! `Transaction::time`s are less than 500 ms apart. Nothing stamped that time,
//! so every transaction was `0` and the window was always open: a paragraph
//! typed over a minute was one undo step. `EditorHandle` now stamps every local
//! transaction it dispatches with a millisecond clock (`web_time`, so the same
//! code runs in a browser).

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};
use std::time::Duration;

fn editor(html: &str, caret: usize) -> EditorHandle {
    let handle = create_editor();
    assert!(handle.load_html(html));
    handle.set_selection(Selection::cursor(Pos(caret)));
    handle
}

fn html(handle: &EditorHandle) -> String {
    node_to_html(&handle.doc())
}

/// The issue's own pin, on the real clock: type, wait past the window, type,
/// and one undo takes back only the second run.
#[test]
fn a_pause_longer_than_the_window_starts_a_new_undo_step() {
    let handle = editor("<p>x</p>", 2);
    assert!(handle.insert_text("a"));
    assert!(handle.insert_text("b"));
    std::thread::sleep(Duration::from_millis(650));
    assert!(handle.insert_text("c"));
    assert!(handle.insert_text("d"));
    assert_eq!(html(&handle), "<p>xabcd</p>");

    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>xab</p>", "undo takes back only `cd`");
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>x</p>", "then `ab`");
}

/// The same through a command: `deleteCharBackward` (Backspace) is typing too,
/// and goes through `EditorHandle::command`, not `update`.
#[test]
fn a_pause_separates_two_runs_of_backspace() {
    let handle = editor("<p>abcdef</p>", 7);
    assert!(handle.command("deleteCharBackward"));
    assert!(handle.command("deleteCharBackward"));
    std::thread::sleep(Duration::from_millis(650));
    assert!(handle.command("deleteCharBackward"));
    assert_eq!(html(&handle), "<p>abc</p>");

    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>abcd</p>", "undo restores only the last delete");
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>abcdef</p>");
}

// ---- On a clock the fixture sets (`__set_history_clock`) -----------------

thread_local! {
    static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn fake_now() -> u64 {
    NOW.with(|n| n.get())
}

fn at(ms: u64) {
    NOW.with(|n| n.set(ms));
}

/// Installs the fixture clock for one test and restores the real one after.
struct FakeClock;

impl FakeClock {
    fn install(start: u64) -> FakeClock {
        at(start);
        rinch_editor_view::__set_history_clock(Some(fake_now));
        FakeClock
    }
}

impl Drop for FakeClock {
    fn drop(&mut self) {
        rinch_editor_view::__set_history_clock(None);
    }
}

/// The window's edge, off zero: keystrokes 499 ms apart are one step, 500 ms
/// apart are two (the history's test is `gap < 500`).
#[test]
fn the_window_closes_at_500_ms() {
    let _clock = FakeClock::install(10_000);
    let handle = editor("<p>x</p>", 2);
    assert!(handle.insert_text("a"));
    at(10_499);
    assert!(handle.insert_text("b")); // 499 ms after `a`: merged
    at(10_998);
    assert!(handle.insert_text("c")); // 499 ms after `b` (the window slides): merged
    at(11_498);
    assert!(handle.insert_text("d")); // 500 ms after `c`: a new step
    assert_eq!(html(&handle), "<p>xabcd</p>");

    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>xabc</p>");
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>x</p>", "`abc` was one step");
}

/// Typing in quick succession is still one step (the stamps are milliseconds,
/// not a finer unit), and a command's typing (Backspace) merges with it.
#[test]
fn quick_typing_and_backspace_are_one_step() {
    let _clock = FakeClock::install(50_000);
    let handle = editor("<p>x</p>", 2);
    for (i, ch) in ["h", "e", "l", "p"].iter().enumerate() {
        at(50_000 + 120 * i as u64);
        assert!(handle.insert_text(ch));
    }
    at(50_600);
    assert!(handle.command("deleteCharBackward"));
    assert_eq!(html(&handle), "<p>xhel</p>");
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>x</p>");
}

/// A transaction whose builder set a time keeps it: the handle stamps only an
/// unstamped one.
#[test]
fn an_apps_own_time_is_kept() {
    let _clock = FakeClock::install(1_000);
    let handle = editor("<p>x</p>", 2);
    assert!(handle.insert_text("a")); // stamped 1000
    // The app says this keystroke came 2 s later, though the clock says 1000.
    assert!(handle.update(|state| {
        let mut tr = state.tr();
        tr.insert_text("b").expect("a caret in a paragraph takes text");
        tr.set_time(3_000);
        Some(tr)
    }));
    assert_eq!(html(&handle), "<p>xab</p>");
    assert!(handle.command("undo"));
    assert_eq!(html(&handle), "<p>xa</p>", "the app's time put `b` in a step of its own");
}
