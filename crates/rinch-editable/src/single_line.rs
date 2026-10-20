//! What a single-line field (`<input>`) makes of inserted text that holds
//! line breaks (issue #824).

/// The text a **single-line** field inserts when the user inserts `text`
/// (a paste, an IME commit, a soft keyboard's committed characters).
///
/// A single-line field holds no line break, so one in inserted text would
/// be painted as extra lines outside the field's box. Chrome 155 decides
/// what goes in instead (measured with `execCommand('insertText')` on an
/// `<input>`, which takes the text through the same insertion filter a
/// paste does): line breaks at the **end** of the text are dropped, and
/// every other `\r\n`, `\r` or `\n` becomes one space — `"a\nb"` is
/// `"a b"`, `"a\r\n\r\nb"` is `"a  b"`, `"a\n"` is `"a"`, `"\n"` is `""`.
/// Nothing else is touched: a tab, U+2028 and U+0085 go in as they are.
///
/// This is the rule for **user-inserted** text. A programmatic write of the
/// field's value is sanitized by a different rule (HTML's value
/// sanitization *strips* line breaks), which this function does not apply.
pub fn single_line_insert(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(['\r', '\n']) {
        return std::borrow::Cow::Borrowed(text);
    }
    let kept = text.trim_end_matches(['\r', '\n']);
    let mut out = String::with_capacity(kept.len());
    let mut chars = kept.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::single_line_insert;

    /// Each row is Chrome 155's answer for the same text inserted into an
    /// `<input>` (`execCommand('insertText')`).
    #[test]
    fn line_breaks_become_spaces_and_trailing_ones_go() {
        for (input, chrome) in [
            ("line1\nline2\r\nline3\rend", "line1 line2 line3 end"),
            ("a\n\n", "a"),
            ("a\r\n", "a"),
            ("a\r", "a"),
            ("\nb", " b"),
            ("a\n\nb", "a  b"),
            ("a\r\n\r\nb", "a  b"),
            (" a \n b ", " a   b "),
            ("\n\n", ""),
            ("\n", ""),
            ("a\u{2028}b", "a\u{2028}b"),
            ("a\u{85}b", "a\u{85}b"),
            ("a\tb", "a\tb"),
            ("a\u{b}b", "a\u{b}b"),
            ("plain", "plain"),
        ] {
            assert_eq!(single_line_insert(input), chrome, "{input:?}");
        }
    }
}
