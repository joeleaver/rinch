//! Keyboard extension of a read-only selection (#2953), against Chrome 158
//! (headless, Playwright's mouse and keyboard, Linux key bindings,
//! `getSelection().toString()`; the same markup in the same 600px root with
//! `font: 16px/20px sans-serif`).

use super::text_selection_copy_gesture_tests::copy_after;

/// `(name, markup, ops, Chrome's string)`, ops as `copy_after` reads them.
const KEYS: &[(&str, &str, &str, &str)] = &[
    (
        "char right",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "click:t:0.0:1;key:Shift+ArrowRight:3",
        "",
    ),
    (
        "drag then right",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma delta</span></p>",
        "drag:t:0.3:t:0.6;key:Shift+ArrowRight:2",
        "eta gamm",
    ),
    (
        "drag then left",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma delta</span></p>",
        "drag:t:0.3:t:0.6;key:Shift+ArrowLeft:2",
        "eta ",
    ),
    (
        "backdrag then right",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma delta</span></p>",
        "drag:t:0.6:t:0.3;key:Shift+ArrowRight:2",
        "a ga",
    ),
    (
        "dbl then right",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "click:t:0.5:2;key:Shift+ArrowRight:2",
        "beta g",
    ),
    (
        "dbl then left",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "click:t:0.5:2;key:Shift+ArrowLeft:2",
        "be",
    ),
    (
        "triple then left",
        "<p style=\"margin:0\">zero</p><p style=\"margin:0\"><span id=\"t\">alpha beta</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3;key:Shift+ArrowLeft:2",
        "alpha bet",
    ),
    (
        "triple then right",
        "<p style=\"margin:0\">zero</p><p style=\"margin:0\"><span id=\"t\">alpha beta</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3;key:Shift+ArrowRight:2",
        "alpha beta\n\nne",
    ),
    (
        "word right",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:0.0:t:0.07;key:Control+Shift+ArrowRight:2",
        "alpha beta",
    ),
    (
        "word left",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:1.0:t:0.93;key:Control+Shift+ArrowLeft:2",
        "beta gamma",
    ),
    (
        "across para right",
        "<p style=\"margin:0\"><span id=\"t\">abc</span></p><p style=\"margin:0\">def</p>",
        "drag:t:0.0:t:0.34;key:Shift+ArrowRight:4",
        "abc\n\nd",
    ),
    (
        "across para left",
        "<p style=\"margin:0\">abc</p><p style=\"margin:0\"><span id=\"t\">def</span></p>",
        "drag:t:1.0:t:0.34;key:Shift+ArrowLeft:4",
        "bc\n\ndef",
    ),
    (
        "down",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p><p style=\"margin:0\">delta epsilon zeta</p>",
        "drag:t:0.0:t:0.3;key:Shift+ArrowDown:1",
        "alpha beta gamma\n\ndelta ",
    ),
    (
        "up",
        "<p style=\"margin:0\">alpha beta gamma</p><p style=\"margin:0\"><span id=\"t\">delta epsilon zeta</span></p>",
        "drag:t:1.0:t:0.5;key:Shift+ArrowUp:1",
        "a gamma\n\ndelta epsilon zeta",
    ),
    (
        "down last line",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:0.0:t:0.3;key:Shift+ArrowDown:1",
        "alpha beta gamma",
    ),
    (
        "end",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p><p style=\"margin:0\">next</p>",
        "drag:t:0.0:t:0.3;key:Shift+End:1",
        "alpha beta gamma",
    ),
    (
        "home",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:1.0:t:0.6;key:Shift+Home:1",
        "alpha beta gamma",
    ),
    (
        "end wrapped",
        "<p style=\"margin:0;width:120px\"><span id=\"t\">alpha beta gamma delta epsilon</span></p>",
        "drag:t:0.0:t:0.05;key:Shift+End:1",
        "alpha beta",
    ),
    (
        "ctrl end",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta</span></p><p style=\"margin:0\">next</p>",
        "drag:t:0.0:t:0.3;key:Control+Shift+End:1",
        "alpha beta\n\nnext",
    ),
    (
        "rtl left",
        "<p dir=\"rtl\" style=\"margin:0\"><span id=\"t\">שלום עולם</span></p>",
        "drag:t:1.0:t:0.9;key:Shift+ArrowLeft:2",
        "שלו",
    ),
    (
        "rtl right",
        "<p dir=\"rtl\" style=\"margin:0\"><span id=\"t\">שלום עולם</span></p>",
        "drag:t:1.0:t:0.4;key:Shift+ArrowRight:2",
        "שלו",
    ),
    (
        "rtl word left",
        "<p dir=\"rtl\" style=\"margin:0\"><span id=\"t\">שלום עולם טוב</span></p>",
        "drag:t:1.0:t:0.9;key:Control+Shift+ArrowLeft:1",
        "שלום",
    ),
    (
        "over ib",
        "<p style=\"margin:0\"><span id=\"t\">ab</span><span style=\"display:inline-block\">cd</span>ef</p>",
        "drag:t:0.0:t:0.5;key:Shift+ArrowRight:4",
        "abcde",
    ),
    (
        "word right end para",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta</span></p><p style=\"margin:0\">next one</p>",
        "drag:t:0.6:t:0.7;key:Control+Shift+ArrowRight:3",
        "beta\n\nnext",
    ),
    (
        "across div right",
        "<div><span id=\"t\">abc</span></div><div>def</div>",
        "drag:t:0.0:t:0.34;key:Shift+ArrowRight:4",
        "abc\nd",
    ),
    (
        "down wrapped",
        "<p id=\"p\" style=\"margin:0;width:120px\">alpha beta gamma delta epsilon</p>",
        "drag:p:0.0:p:0.1;key:Shift+ArrowDown:1",
        "alpha beta g",
    ),
    (
        "up first line",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:1.0:t:0.6;key:Shift+ArrowUp:1",
        "alpha beta gamma",
    ),
    (
        "ctrl home",
        "<p style=\"margin:0\">zero</p><p style=\"margin:0\"><span id=\"t\">alpha beta</span></p>",
        "drag:t:1.0:t:0.7;key:Control+Shift+Home:1",
        "zero\n\nalpha beta",
    ),
    (
        "alt word",
        "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>",
        "drag:t:0.0:t:0.07;key:Alt+Shift+ArrowRight:1",
        "a",
    ),
    (
        "down twice keeps x",
        "<p id=\"p\" style=\"margin:0\">alpha beta gamma</p><p style=\"margin:0\">de</p><p style=\"margin:0\">delta epsilon zeta</p>",
        "drag:p:0.0:p:0.2;key:Shift+ArrowDown:2",
        "alpha beta gamma\n\nde\n\ndelta epsilon zeta",
    ),
    (
        "down margins",
        "<p id=\"p\">alpha beta gamma</p><p>delta epsilon zeta</p>",
        "drag:p:0.0:p:0.1;key:Shift+ArrowDown:1",
        "alpha beta gamma\n\ndelta eps",
    ),
];

/// The cases of [`KEYS`] named `names`, checked together.
fn keys_as_chrome(names: &[&str]) {
    let off: Vec<String> = names
        .iter()
        .map(|&n| {
            *KEYS
                .iter()
                .find(|c| c.0 == n)
                .unwrap_or_else(|| panic!("no case {n}"))
        })
        .filter_map(|(name, html, ops, chrome)| {
            let html: &'static str = html;
            let t = copy_after(html, ops);
            (t != chrome).then(|| format!("{name}: {t:?}, Chrome {chrome:?}"))
        })
        .collect();
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn shift_arrows_extend_a_selection_by_characters() {
    // A character a press, the paragraph break one more, an inline-block's
    // text stepped through; the anchor stays (a drag back keeps its own),
    // and a double or triple click's selection moves by characters from
    // its end. A collapsed selection (a click) is no caret here: nothing.
    keys_as_chrome(&[
        "char right",
        "drag then right",
        "drag then left",
        "backdrag then right",
        "dbl then right",
        "dbl then left",
        "triple then left",
        "triple then right",
        "across para right",
        "across para left",
        "across div right",
        "over ib",
        "alt word",
    ]);
}

#[test]
fn a_plain_arrow_leaves_a_read_only_selection() {
    // Chrome 158 (caret browsing off): no caret to move, the selection stays.
    let html = "<p style=\"margin:0\"><span id=\"t\">alpha beta gamma</span></p>";
    let before = copy_after(html, "drag:t:0.0:t:0.5");
    assert!(!before.is_empty());
    assert_eq!(
        copy_after(html, "drag:t:0.0:t:0.5;key:ArrowRight:1"),
        before
    );
    assert_eq!(copy_after(html, "drag:t:0.0:t:0.5;key:ArrowDown:1"), before);
}

#[test]
fn the_word_chord_extends_by_words() {
    // Ctrl on Linux: to the next word's end, the previous one's start, and
    // a root's edge to the next root first. Alt is no text chord there
    // ("alt word": nothing moves).
    keys_as_chrome(&["word right", "word left", "word right end para"]);
}

#[test]
fn shift_up_and_down_extend_by_lines() {
    // Into the next paragraph past its margin, at the same x kept across a
    // shorter line; from the last line to the text's end, the first to its
    // start.
    keys_as_chrome(&[
        "down",
        "up",
        "down wrapped",
        "down last line",
        "up first line",
        "down twice keeps x",
        "down margins",
    ]);
}

#[test]
fn shift_home_and_end_extend_to_the_line_and_the_document_edges() {
    keys_as_chrome(&["end", "home", "end wrapped", "ctrl end", "ctrl home"]);
}

#[test]
fn on_a_right_to_left_line_left_moves_forward() {
    // Left is forward in a right-to-left root, the word chord too.
    keys_as_chrome(&["rtl left", "rtl right", "rtl word left"]);
}
