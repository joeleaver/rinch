//! Grapheme-cluster boundaries for caret motion, deletion and hit testing
//! (#1684): what one arrow press, Delete or Backspace steps over, and where a
//! caret may stand.
//!
//! A caret never stands inside an extended grapheme cluster (UAX #29): an
//! emoji ZWJ sequence (`👨‍👩‍👧`), a flag (`🇫🇷`), a keycap (`1️⃣`), an emoji with
//! a variation selector (`❤️`) or a skin tone, a base and its combining marks.
//! Arrow keys and Delete step over a whole cluster. **Backspace does not
//! always** — Chrome 155 (measured with trusted keys, `<textarea>` and
//! `contenteditable` alike) deletes a whole cluster only when it is an emoji
//! sequence, and one code point otherwise, so `e` + U+0301 loses only the
//! accent, a decomposed Hangul syllable its last jamo, a Devanagari conjunct
//! its last sign: see [`backspace_start`].
//!
//! Offsets are byte offsets into the text; the `_chars` variants take and
//! answer char indices, the rich editor's position space.

use unicode_segmentation::GraphemeCursor;

/// The grapheme boundary before `byte` (0 at the start). `byte` need not be a
/// boundary itself: inside a cluster this is the cluster's start.
pub fn prev_grapheme_boundary(text: &str, byte: usize) -> usize {
    let byte = floor_char_boundary(text, byte);
    if byte == 0 {
        return 0;
    }
    let mut c = GraphemeCursor::new(byte, text.len(), true);
    c.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
}

/// The grapheme boundary after `byte` (`text.len()` at the end). Inside a
/// cluster this is the cluster's end.
pub fn next_grapheme_boundary(text: &str, byte: usize) -> usize {
    let byte = floor_char_boundary(text, byte);
    if byte >= text.len() {
        return text.len();
    }
    let mut c = GraphemeCursor::new(byte, text.len(), true);
    c.next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

/// Whether a caret may stand at `byte`: a grapheme boundary (the text's two
/// ends included). A byte inside a UTF-8 sequence is not one.
pub fn is_grapheme_boundary(text: &str, byte: usize) -> bool {
    if byte == 0 || byte >= text.len() {
        return byte <= text.len();
    }
    if !text.is_char_boundary(byte) {
        return false;
    }
    let mut c = GraphemeCursor::new(byte, text.len(), true);
    c.is_boundary(text, 0).unwrap_or(true)
}

/// Where a Backspace at `byte` starts deleting, as Chrome 155 does: the start
/// of the cluster before `byte` when that cluster is an emoji sequence or a
/// `"\r\n"`, else the start of the code point before `byte`. From inside a cluster (no
/// caret should stand there) it is the cluster's start.
///
/// An emoji sequence is a cluster holding a regional indicator, a variation
/// selector, the keycap mark U+20E3, an emoji modifier (skin tone), a tag
/// character, or a ZWJ followed by more of the cluster — the code points
/// Blink's backspace state machine joins to their base. Anything else in a
/// cluster (combining marks, Hangul jamo, Indic and Thai signs) goes one code
/// point at a time.
pub fn backspace_start(text: &str, byte: usize) -> usize {
    if !is_grapheme_boundary(text, byte) {
        return prev_grapheme_boundary(text, byte);
    }
    let end = byte.min(text.len());
    if end == 0 {
        return 0;
    }
    let start = prev_grapheme_boundary(text, end);
    let cluster = &text[start..end];
    if cluster == "\r\n" || is_emoji_sequence(cluster) {
        return start;
    }
    text[..end].char_indices().last().map_or(0, |(i, _)| i)
}

/// Whether a grapheme cluster is an emoji sequence Backspace deletes whole.
fn is_emoji_sequence(cluster: &str) -> bool {
    let mut chars = cluster.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        let u = c as u32;
        let more = chars.peek().map(|&n| n as u32);
        let joins = match u {
            // A regional indicator: a flag is a pair of them.
            0x1F1E6..=0x1F1FF => !first || more.is_some(),
            // Variation selectors, the keycap mark, skin tones, tags.
            0xFE00..=0xFE0F
            | 0xE0100..=0xE01EF
            | 0x20E3
            | 0x1F3FB..=0x1F3FF
            | 0xE0020..=0xE007F => !first,
            // A ZWJ joining a pictograph (not an Indic conjunct's ZWJ).
            0x200D => more.is_some_and(|n| n >= 0x2100),
            _ => false,
        };
        if joins {
            return true;
        }
        first = false;
    }
    false
}

/// `byte` moved down to a char boundary (and clamped to the text).
fn floor_char_boundary(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// The byte offset of char index `i` (the text's length past its end).
fn char_to_byte(text: &str, i: usize) -> usize {
    text.char_indices().nth(i).map_or(text.len(), |(b, _)| b)
}

/// The char index of byte offset `b` (a char boundary).
fn byte_to_char(text: &str, b: usize) -> usize {
    text[..b].chars().count()
}

/// [`prev_grapheme_boundary`] in char indices.
pub fn prev_grapheme_boundary_chars(text: &str, i: usize) -> usize {
    byte_to_char(text, prev_grapheme_boundary(text, char_to_byte(text, i)))
}

/// [`next_grapheme_boundary`] in char indices.
pub fn next_grapheme_boundary_chars(text: &str, i: usize) -> usize {
    byte_to_char(text, next_grapheme_boundary(text, char_to_byte(text, i)))
}

/// [`backspace_start`] in char indices.
pub fn backspace_start_chars(text: &str, i: usize) -> usize {
    byte_to_char(text, backspace_start(text, char_to_byte(text, i)))
}

/// [`is_grapheme_boundary`] in char indices.
pub fn is_grapheme_boundary_chars(text: &str, i: usize) -> bool {
    is_grapheme_boundary(text, char_to_byte(text, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrome 155, trusted keys into a `<textarea>` (and the same in a
    /// `contenteditable`), caret before the final `b`: what Backspace leaves,
    /// and where one ArrowRight from after `a` lands (UTF-16 there; the
    /// cluster's end here).
    const CHROME_155: &[(&str, &str, &str)] = &[
        // (text, after Backspace before the last 'b', cluster after 'a')
        ("a\u{2764}\u{FE0F}b", "ab", "\u{2764}\u{FE0F}"),
        (
            "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b",
            "ab",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        ),
        ("a\u{1F1EB}\u{1F1F7}b", "ab", "\u{1F1EB}\u{1F1F7}"),
        ("a1\u{FE0F}\u{20E3}b", "ab", "1\u{FE0F}\u{20E3}"),
        ("a\u{1F44D}\u{1F3FD}b", "ab", "\u{1F44D}\u{1F3FD}"),
        ("ae\u{301}b", "aeb", "e\u{301}"),
        (
            "a\u{1100}\u{1161}\u{11A8}b",
            "a\u{1100}\u{1161}b",
            "\u{1100}\u{1161}\u{11A8}",
        ),
        (
            "a\u{915}\u{94D}\u{937}\u{93F}b",
            "a\u{915}\u{94D}\u{937}b",
            "\u{915}\u{94D}\u{937}\u{93F}",
        ),
        (
            "a\u{E01}\u{E34}\u{E48}b",
            "a\u{E01}\u{E34}b",
            "\u{E01}\u{E34}\u{E48}",
        ),
        (
            "a\u{1F1EB}\u{1F1F7}\u{1F1E9}\u{1F1EA}b",
            "a\u{1F1EB}\u{1F1F7}b",
            "\u{1F1EB}\u{1F1F7}",
        ),
        (
            "a\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}b",
            "ab",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
        ),
        ("ae\u{301}\u{323}b", "ae\u{301}b", "e\u{301}\u{323}"),
        ("a\u{2764}\u{FE0E}b", "ab", "\u{2764}\u{FE0E}"),
        (
            "a\u{1F469}\u{1F3FD}\u{200D}\u{1F4BB}b",
            "ab",
            "\u{1F469}\u{1F3FD}\u{200D}\u{1F4BB}",
        ),
    ];

    #[test]
    fn backspace_delete_and_arrows_match_chrome_155() {
        for &(text, after_bs, cluster) in CHROME_155 {
            let before_b = text.len() - 1;
            let start = backspace_start(text, before_b);
            let left = format!("{}{}", &text[..start], &text[before_b..]);
            assert_eq!(left, after_bs, "Backspace in {text:?}");
            // ArrowRight / Delete from after 'a' cross the whole cluster.
            assert_eq!(
                next_grapheme_boundary(text, 1),
                1 + cluster.len(),
                "{text:?}"
            );
            // ArrowLeft from before 'b' crosses the last cluster.
            assert_eq!(
                prev_grapheme_boundary(text, before_b),
                before_b - last_cluster(text, before_b).len(),
                "{text:?}"
            );
        }
    }

    fn last_cluster(text: &str, end: usize) -> &str {
        use unicode_segmentation::UnicodeSegmentation;
        text[..end].graphemes(true).last().unwrap()
    }

    #[test]
    fn no_boundary_inside_a_cluster() {
        let t = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b";
        let interior: Vec<usize> = (2..t.len() - 1)
            .filter(|&b| is_grapheme_boundary(t, b))
            .collect();
        assert!(interior.is_empty(), "{interior:?}");
        // Inside a cluster, the neighbours are its edges.
        assert_eq!(prev_grapheme_boundary(t, 8), 1);
        assert_eq!(next_grapheme_boundary(t, 8), t.len() - 1);
        // A Backspace from inside a cluster deletes back to the cluster's start.
        assert_eq!(backspace_start(t, 8), 1);
    }

    #[test]
    fn crlf_is_one_step_and_one_backspace() {
        let t = "a\r\nb";
        assert_eq!(next_grapheme_boundary(t, 1), 3);
        assert_eq!(backspace_start(t, 3), 1);
    }

    #[test]
    fn char_index_variants_agree() {
        let t = "x\u{1F1EB}\u{1F1F7}e\u{301}";
        assert_eq!(next_grapheme_boundary_chars(t, 1), 3);
        assert_eq!(prev_grapheme_boundary_chars(t, 3), 1);
        assert_eq!(backspace_start_chars(t, 5), 4);
        assert_eq!(backspace_start_chars(t, 3), 1);
        assert!(!is_grapheme_boundary_chars(t, 2));
    }
}
