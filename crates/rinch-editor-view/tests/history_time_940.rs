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
