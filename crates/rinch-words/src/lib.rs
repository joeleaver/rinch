//! Word segmentation shared by the Rinch rich-text editor
//! (`rinch-editor-core`'s `motion`) and the desktop text fields
//! (`rinch-editable`), so a word motion, a word delete and a double-click see
//! the same words in both (#939, #1619).
//!
//! A run of text is a slice of [`Unit`]s; [`word_segments`] tiles it into
//! words, [`word_boundary`] is where a Ctrl+Left / Ctrl+Right lands and
//! [`word_range`] what a double-click selects. [`text_word_boundary`] and
//! [`text_word_at`] are the same over a plain string, in byte offsets, with
//! each `'\n'` a [`Unit::Break`] (a field's line break).
//!
//! It depends on no rinch crate, so a crate that must stay light (the field
//! engine) can use it without the editor model. The ICU dictionary that splits
//! CJK and Thai runs is the optional `dictionary` feature.

/// One position of a run of text: a char, or (in the rich editor) an inline
/// atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A character of a text node.
    Char(char),
    /// A non-text leaf other than a hard break (an image): one position, and a
    /// word of its own with the atoms right beside it (Chrome 155 takes
    /// `<img><img>` as one word).
    Atom,
    /// A hard break: one position, and always a word of its own (Chrome 155
    /// stops at each `<br>`).
    Break,
}

impl Unit {
    /// Whether the unit is a whitespace char (a [`Unit::Break`] is not).
    pub fn is_space(self) -> bool {
        matches!(self, Unit::Char(c) if c.is_whitespace())
    }
}

/// What a unit is to word segmentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Space,
    /// Letters, digits, marks and `_` of every script but the CJK ones.
    Word,
    /// Han, Hiragana and Katakana.
    Cjk,
    /// Everything else that is not whitespace: punctuation, symbols, emoji.
    Punct,
    Atom,
    Break,
}

/// Han, Hiragana, Katakana (and their iteration and prolonged-sound marks).
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005..=0x3007 | 0x3021..=0x3029 | 0x3031..=0x3035 | 0x303B..=0x303C
        | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x3134F)
}

/// Thai, Lao, Myanmar and Khmer: the scripts ICU's dictionary splits inside a
/// word run.
#[cfg_attr(not(feature = "dictionary"), allow(dead_code))]
fn is_south_east_asian(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x19E0..=0x19FF
        | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F)
}

/// A char that extends the one before it rather than starting anything: a
/// combining mark of any script (General_Category M), a format char (Cf: the
/// joiners, variation-selector-like controls) or an emoji skin-tone modifier.
/// std has no General_Category, so this asks the `regex` crate's Unicode
/// tables, which this crate links already for its input rules. Below U+0300
/// the only such char is the soft hyphen, so Latin text asks no regex.
fn is_extend(c: char) -> bool {
    use std::sync::OnceLock;
    static EXTEND: OnceLock<regex::Regex> = OnceLock::new();
    if (c as u32) < 0x300 {
        return c == '\u{AD}';
    }
    let re = EXTEND.get_or_init(|| {
        regex::Regex::new(r"^[\p{M}\p{Cf}\x{1F3FB}-\x{1F3FF}]$").expect("a valid class")
    });
    let mut buf = [0u8; 4];
    re.is_match(c.encode_utf8(&mut buf))
}

/// General_Category Nd (a decimal digit of any script: `3`, `٣`; not `½`).
fn is_decimal_digit(c: char) -> bool {
    use std::sync::OnceLock;
    static ND: OnceLock<regex::Regex> = OnceLock::new();
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    let re = ND.get_or_init(|| regex::Regex::new(r"^\p{Nd}$").expect("a valid class"));
    let mut buf = [0u8; 4];
    re.is_match(c.encode_utf8(&mut buf))
}

/// An emoji (Extended_Pictographic): a run of them is one word to a
/// double-click, where any other punctuation run gives one char.
fn is_pictographic(c: char) -> bool {
    use std::sync::OnceLock;
    static PICTO: OnceLock<regex::Regex> = OnceLock::new();
    if c.is_ascii() {
        return false;
    }
    let re = PICTO
        .get_or_init(|| regex::Regex::new(r"^\p{Extended_Pictographic}$").expect("a valid class"));
    let mut buf = [0u8; 4];
    re.is_match(c.encode_utf8(&mut buf))
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_cjk(c) {
        Class::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// The word segments of a run of units, as `(start, end)` unit offsets
/// that tile it: maximal runs of one class — whitespace, word characters, CJK,
/// punctuation, images — with every hard break a segment of its own (#938,
/// #939). A combining mark or format char joins the unit before it (unless
/// that is whitespace); an apostrophe between two word characters stays in the
/// word (`don't`), and a `.` `,` `;` `'` `’` between two digits stays in the
/// number (`3.14`, `1,000`: UAX #29's MidNum). With the `dictionary`
/// feature ICU's dictionary segmenter splits a CJK run (`漢字です` is `漢字`
/// `です`) and the Thai, Lao, Myanmar or Khmer stretches of a word run
/// (`ทุกสองสัปดาห์`), but never where such a stretch meets other letters
/// (`abcไทย` is one word); without it those runs are one segment each.
///
/// Measured against Chrome 155 (contenteditable): `foo.bar` is `foo` `.` `bar`,
/// `a.b.c` is five words, `foo...` is `foo` `...`, `3.14`, `don't` and an emoji
/// ZWJ sequence are one word each, two adjacent images are one word and each
/// `<br>` is one. These are not plain UAX #29 word boundaries, which keep
/// `foo.bar` whole; Chrome's editing splits it, as this does.
pub fn word_segments(units: &[Unit]) -> Vec<(usize, usize)> {
    #[cfg(feature = "cost-counters")]
    SEGMENTED_UNITS.with(|n| n.set(n.get() + units.len()));
    let mut classes: Vec<Class> = Vec::with_capacity(units.len());
    for &unit in units {
        let mut k = match unit {
            Unit::Atom => Class::Atom,
            Unit::Break => Class::Break,
            Unit::Char(c) => class_of(c),
        };
        if let (Unit::Char(c), Some(&prev)) = (unit, classes.last())
            && is_extend(c)
            && !matches!(prev, Class::Atom | Class::Break | Class::Space)
        {
            k = prev;
        }
        classes.push(k);
    }
    let digit = |i: usize| matches!(units[i], Unit::Char(c) if is_decimal_digit(c));
    for i in 1..units.len().saturating_sub(1) {
        let Unit::Char(c) = units[i] else { continue };
        // An apostrophe between two word characters is part of the word.
        let apostrophe = matches!(c, '\'' | '\u{2019}')
            && classes[i - 1] == Class::Word
            && classes[i + 1] == Class::Word;
        // A separator between two digits is part of the number.
        let in_number =
            matches!(c, '.' | ',' | ';' | '\'' | '\u{2019}') && digit(i - 1) && digit(i + 1);
        if apostrophe || in_number {
            classes[i] = Class::Word;
        }
    }
    let mut segments = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let k = classes[i];
        let mut j = i + 1;
        if k != Class::Break {
            while j < units.len() && classes[j] == k {
                j += 1;
            }
        }
        match k {
            Class::Cjk => split_by_dictionary(units, i, j, true, &mut segments),
            Class::Word => split_by_dictionary(units, i, j, false, &mut segments),
            _ => segments.push((i, j)),
        }
        i = j;
    }
    segments
}

#[cfg(feature = "cost-counters")]
thread_local! {
    static SEGMENTED_UNITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Units classified by [`word_segments`] on this thread so far (test-only
/// `cost-counters` feature): a word motion segments the stretch around the
/// caret, not the whole block or field.
#[cfg(feature = "cost-counters")]
#[doc(hidden)]
pub fn segmented_units() -> usize {
    SEGMENTED_UNITS.with(|n| n.get())
}

/// Push the segments of the run `units[start..end]`. With `word-dictionary`:
/// a CJK run (`whole`) is split where ICU's dictionary breaks it; a word run
/// only inside its South-East Asian stretches (no break where one meets other
/// letters). Without the feature, the whole run.
#[cfg(feature = "dictionary")]
fn split_by_dictionary(
    units: &[Unit],
    start: usize,
    end: usize,
    whole: bool,
    out: &mut Vec<(usize, usize)>,
) {
    use icu_segmenter::WordSegmenter;
    use icu_segmenter::options::WordBreakInvariantOptions;
    let ch = |i: usize| match units[i] {
        Unit::Char(c) => c,
        _ => ' ',
    };
    let mut prev = start;
    let mut a = start;
    while a < end {
        // The stretch the dictionary splits: the whole CJK run, or each
        // maximal South-East Asian stretch of a word run.
        if !whole && !is_south_east_asian(ch(a)) {
            a += 1;
            continue;
        }
        let mut b = a + 1;
        while b < end && (whole || is_south_east_asian(ch(b)) || is_extend(ch(b))) {
            b += 1;
        }
        let text: String = (a..b).map(ch).collect();
        let mut byte_to_unit = Vec::with_capacity(text.len() + 1);
        for (n, c) in text.chars().enumerate() {
            byte_to_unit.extend(std::iter::repeat_n(a + n, c.len_utf8()));
        }
        byte_to_unit.push(b);
        for byte in
            WordSegmenter::new_dictionary(WordBreakInvariantOptions::default()).segment_str(&text)
        {
            let at = byte_to_unit[byte];
            // Breaks inside the stretch only: its edges are not breaks.
            if at > a && at < b && at > prev {
                out.push((prev, at));
                prev = at;
            }
        }
        a = b;
    }
    if prev < end {
        out.push((prev, end));
    }
}

#[cfg(not(feature = "dictionary"))]
fn split_by_dictionary(
    _: &[Unit],
    start: usize,
    end: usize,
    _whole: bool,
    out: &mut Vec<(usize, usize)>,
) {
    out.push((start, end));
}

/// The [word segment](word_segments) holding unit `i` (`i < units.len()`).
/// Segmentation never
/// crosses whitespace (a segment is a whitespace run or holds none), so only
/// the stretch between the whitespace on either side of `i` is segmented: the
/// work is linear in that whitespace-free stretch — the word in spaced text,
/// but the whole paragraph in CJK or Thai written without spaces (measured in
/// the review of #1613, one Ctrl+Right in a 65k-char Thai paragraph with no
/// spaces: 30.5 ms with the dictionary, 13.1 ms without; CJK 60k: 5.0 / 1.6 ms).
pub fn segment_around(units: &[Unit], i: usize) -> (usize, usize) {
    let space = units[i].is_space();
    let mut lo = i;
    while lo > 0 && units[lo - 1].is_space() == space {
        lo -= 1;
    }
    let mut hi = i + 1;
    while hi < units.len() && units[hi].is_space() == space {
        hi += 1;
    }
    if space {
        return (lo, hi);
    }
    let segments = word_segments(&units[lo..hi]);
    let k = segments.partition_point(|&(_, end)| end <= i - lo);
    (lo + segments[k].0, lo + segments[k].1)
}

/// Whether a segment is a punctuation run (no word character in it).
fn is_punct_segment(units: &[Unit]) -> bool {
    units
        .iter()
        .all(|u| matches!(u, Unit::Char(c) if class_of(*c) == Class::Punct))
}

/// Where a word motion from the gap before unit `i` lands, as a unit offset.
///
/// Backward (Ctrl+Left): past any whitespace before `i`, then to the start of
/// the [word segment](word_segments) before it. Forward (Ctrl+Right): past any
/// whitespace after `i`, then to the end of the segment after it — the end of
/// the next word, as on Linux and macOS. A word delete removes exactly this
/// range.
pub fn word_boundary(units: &[Unit], i: usize, forward: bool) -> usize {
    let mut i = i.min(units.len());
    if forward {
        while i < units.len() && units[i].is_space() {
            i += 1;
        }
        if i < units.len() {
            i = segment_around(units, i).1;
        }
    } else {
        while i > 0 && units[i - 1].is_space() {
            i -= 1;
        }
        if i > 0 {
            i = segment_around(units, i - 1).0;
        }
    }
    i
}

/// The [word segment](word_segments) a double-click at the gap before unit
/// `off` selects (a run of whitespace, a word, images, a hard break), chosen
/// by the unit to the right of the gap, or the one to its left at the end and
/// at a word's trailing edge (right unit whitespace, left not). On a
/// punctuation run it is that one char (with its marks), as in Chrome 155
/// (`foo|...bar` selects one `.`, `foo|——bar` one `—`); a run of emoji stays
/// whole (`😀😀`). Collapsed `(off, off)` for no units.
pub fn word_range(units: &[Unit], off: usize) -> (usize, usize) {
    if units.is_empty() {
        return (off, off);
    }
    let off = off.min(units.len());
    let classify_right =
        off < units.len() && (!units[off].is_space() || off == 0 || units[off - 1].is_space());
    let idx = if classify_right { off } else { off - 1 };
    let (start, end) = segment_around(units, idx);
    match units[idx] {
        Unit::Char(c) if !is_pictographic(c) && is_punct_segment(&units[start..end]) => {
            // The char and any marks on it.
            let mut e = idx + 1;
            while e < end && matches!(units[e], Unit::Char(m) if is_extend(m)) {
                e += 1;
            }
            (idx, e)
        }
        _ => (start, end),
    }
}

/// Whether a unit is a word character: a letter, digit or `_` of any script
/// (CJK included), not whitespace, punctuation, an atom or a break.
pub fn is_word_char(unit: Unit) -> bool {
    matches!(unit, Unit::Char(c) if matches!(class_of(c), Class::Word | Class::Cjk))
}

// ===================================================================
// Plain text (a text field's value), in byte offsets
// ===================================================================

/// The units of a plain string, one per char, with each `'\n'` a
/// [`Unit::Break`]: a line break of a `<textarea>` is a word of its own, as a
/// hard break is in the editor (Chrome 155: Ctrl+Right at a line's end moves
/// past the break and no further). Also returns the byte offset of each unit,
/// plus the string's length at the end.
pub fn text_units(text: &str) -> (Vec<Unit>, Vec<usize>) {
    let mut units = Vec::with_capacity(text.len());
    let mut bytes = Vec::with_capacity(text.len() + 1);
    for (b, c) in text.char_indices() {
        bytes.push(b);
        units.push(if c == '\n' {
            Unit::Break
        } else {
            Unit::Char(c)
        });
    }
    bytes.push(text.len());
    (units, bytes)
}

/// The unit index of byte offset `byte` (the unit starting at or before it).
fn unit_at(bytes: &[usize], byte: usize) -> usize {
    bytes.partition_point(|&b| b <= byte).saturating_sub(1)
}

/// [`word_boundary`] over a plain string, from and to byte offsets: where
/// Ctrl+Left (`forward = false`) or Ctrl+Right lands from `byte`, with a
/// `'\n'` a word of its own. Measured against Chrome 155's `<input>` and
/// `<textarea>` (Linux): `foo-bar|` → `foo-|bar`, `|foo-bar` → `foo|-bar`,
/// `|path/to/file` → `path|/to/file`, and `3.14`, `1,000`, `don't` and
/// `snake_case` are one word each.
///
/// A line break is one stop both ways: Ctrl+Right at a line's end lands just
/// past it, Ctrl+Left at a line's start just before it (the previous line's
/// end), and the next press goes on to the next word. That backward half is
/// rinch's, not Chrome's: Chrome 155's `<textarea>` never takes Ctrl+Left out
/// of the caret's line (`ab-\n|cd` stays put), and rinch makes it the mirror
/// of Ctrl+Right instead (Joe's decision, #1619).
pub fn text_word_boundary(text: &str, byte: usize, forward: bool) -> usize {
    let (units, bytes) = text_units(text);
    let i = unit_at(&bytes, byte.min(text.len()));
    bytes[word_boundary(&units, i, forward)]
}

/// The word around a caret at byte `byte` of a plain string, as a byte range
/// — what a long press selects in a text field: the [word
/// segment](word_segments) of the word character right of the caret, else of
/// the one left of it. `None` when neither side is a word character (the
/// caret is on whitespace or punctuation, or the string is empty).
pub fn text_word_at(text: &str, byte: usize) -> Option<(usize, usize)> {
    let (units, bytes) = text_units(text);
    let i = unit_at(&bytes, byte.min(text.len()));
    let idx = if i < units.len() && is_word_char(units[i]) {
        i
    } else if i > 0 && is_word_char(units[i - 1]) {
        i - 1
    } else {
        return None;
    };
    let (start, end) = segment_around(&units, idx);
    Some((bytes[start], bytes[end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte offsets around multi-byte chars come back on char boundaries, and
    /// an offset inside a char or past the end is snapped, not panicked on.
    #[test]
    fn text_offsets_are_bytes_on_char_boundaries() {
        let text = "héllo-wörld";
        let dash = text.find('-').unwrap();
        assert_eq!(text_word_boundary(text, text.len(), false), dash + 1);
        assert_eq!(text_word_boundary(text, 0, true), dash);
        let o = text.find('ö').unwrap();
        assert_eq!(text_word_at(text, o + 1), Some((dash + 1, text.len())));
        assert_eq!(text_word_boundary(text, 999, false), dash + 1);
        assert_eq!(text_word_boundary("", 0, false), 0);
        assert_eq!(text_word_boundary("", 0, true), 0);
        assert_eq!(text_word_at("", 0), None);
    }

    /// A line break is a word of its own (a `<textarea>`'s, as the editor's
    /// hard break): one stop of the word motion in either direction.
    #[test]
    fn a_line_break_is_a_unit_of_its_own() {
        assert_eq!(
            text_units("a\nb").0,
            vec![Unit::Char('a'), Unit::Break, Unit::Char('b')]
        );
        assert_eq!(text_word_boundary("ab\ncd", 2, true), 3);
        assert_eq!(text_word_boundary("ab\ncd", 3, false), 2);
        assert_eq!(text_word_boundary("ab\ncd", 5, false), 3);
    }
}
