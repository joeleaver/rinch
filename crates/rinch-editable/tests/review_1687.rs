//! Review of #1687: probes.
use rinch_editable::{EditCommand, EditableDocument, EditableState, Selection, StringDocument};

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

#[test]
fn review_1687_undo_and_redo_of_a_cluster_backspace() {
    let text = format!("a{FAMILY}b");
    let mut s = EditableState::new(StringDocument::with_text(&text));
    let caret = 1 + FAMILY.len();
    s.selection = Selection::cursor(caret);
    s.execute(EditCommand::DeleteBackward);
    assert_eq!(s.document.to_text(), "ab");
    assert_eq!(s.selection.head.0, 1);
    s.execute(EditCommand::Undo);
    assert_eq!(
        s.document.to_text(),
        text,
        "undo restores the whole cluster"
    );
    assert_eq!(s.selection.head.0, caret, "caret back after the cluster");
    s.execute(EditCommand::Redo);
    assert_eq!(s.document.to_text(), "ab");
}

/// Chrome 155 (trusted keys, measured in this review): a variation selector
/// after a non-emoji base, and a skin tone after a non-modifier-base, lose
/// only that code point to Backspace.
#[test]
#[ignore = "#1693: Chrome's answer for these clusters, not met yet"]
fn review_1687_backspace_rows_chrome_answers_differently() {
    for (text, chrome) in [
        ("ae\u{301}\u{FE0F}b", "ae\u{301}b"),
        ("a\u{2764}\u{1F3FB}b", "a\u{2764}b"),
        (
            "a\u{915}\u{94D}\u{200D}\u{937}b",
            "a\u{915}\u{94D}\u{200D}b",
        ),
    ] {
        let mut s = EditableState::new(StringDocument::with_text(text));
        s.selection = Selection::cursor(text.len() - 1);
        s.execute(EditCommand::DeleteBackward);
        let got = s.document.to_text();
        eprintln!("{text:?}: rinch {got:?} chrome {chrome:?}");
        assert_eq!(got, chrome, "{text:?}");
    }
}
