//! Review of #1619 (PR #1625): fixtures the PR's own suite did not have.

/// A caret at the seam of two word segments (Latin then CJK) selects the one
/// on its right, as `text_word_at`'s doc says; kills a left-first mutant.
#[test]
fn a_seam_between_two_words_takes_the_right_one() {
    let t = "abc漢字";
    assert_eq!(rinch_words::text_word_at(t, 3), Some((3, t.len())));
    assert_eq!(rinch_words::text_word_at(t, 2), Some((0, 3)));
}

/// The stretch a word motion segments ends at a line break: one Ctrl+Right
/// from the start of a value of unspaced lines segments the first line only,
/// not the whole value (base64, minified JSON or CJK in a textarea).
#[cfg(feature = "cost-counters")]
#[test]
fn the_segmented_stretch_ends_at_a_line_break() {
    let line = "A".repeat(76);
    let text: String = (0..200).map(|_| format!("{line}\n")).collect();
    let before = rinch_words::segmented_units();
    assert_eq!(rinch_words::text_word_boundary(&text, 0, true), 76);
    let back = rinch_words::text_word_boundary(&text, text.len() - 1, false);
    assert_eq!(back, text.len() - 77);
    assert_eq!(rinch_words::segmented_units() - before, 2 * 76);
}
