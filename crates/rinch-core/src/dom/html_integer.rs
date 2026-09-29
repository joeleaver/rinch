//! HTML's rules for parsing integers and non-negative integers (issues #1138,
//! #1153).
//!
//! Every HTML attribute whose value is an integer — `tabindex`, `<textarea
//! rows>`, `<ol start>`, `<li value>`, … — is read by the WHATWG *rules for
//! parsing integers*, not by a programming language's number parser, and the
//! two disagree on ordinary markup: Rust's `i32::from_str` rejects `" 3"`,
//! `"2.5"` and `"3abc"`, which a browser reads as 3, 2 and 3, and
//! `str::parse::<f32>` accepts `"inf"`, `"NaN"` and `"1e30"`, which a browser
//! reads as an error, an error and 1. Read such an attribute through these two
//! functions, never through `parse`.
//!
//! The algorithm (HTML § 2.3.4.1):
//!
//! 1. skip leading **ASCII whitespace** — tab, LF, FF, CR and space; not NBSP
//!    or any other Unicode space;
//! 2. an optional `-` (negative) or `+`;
//! 3. at least one ASCII digit, else an **error**;
//! 4. the digits up to the first non-digit, which ends the number — anything
//!    after it is ignored.
//!
//! A value outside `i32` is an **error**, not a clamp: that is what Chrome 153
//! does for every attribute measured here (`tabindex="99999999999"` gives
//! `tabIndex == -1`, the not-set answer; `rows="2147483648"` is the default 2).
//! `-2147483648` parses; `-2147483649` is an error.

/// HTML's *rules for parsing integers*: the value of `s`, or `None` for an
/// error. See the module doc for the algorithm and what counts as an error.
pub fn parse_html_integer(s: &str) -> Option<i32> {
    let mut bytes = s
        .as_bytes()
        .iter()
        .copied()
        .skip_while(|b| matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' '))
        .peekable();
    let negative = match bytes.peek() {
        Some(b'-') => {
            bytes.next();
            true
        }
        Some(b'+') => {
            bytes.next();
            false
        }
        _ => false,
    };
    // Accumulate towards the sign, so `-2147483648` fits where its magnitude
    // would not.
    let mut value: i32 = 0;
    let mut any = false;
    for b in bytes.take_while(u8::is_ascii_digit) {
        let digit = i32::from(b - b'0');
        value = value.checked_mul(10)?;
        value = if negative {
            value.checked_sub(digit)?
        } else {
            value.checked_add(digit)?
        };
        any = true;
    }
    any.then_some(value)
}

/// HTML's *rules for parsing non-negative integers*: [`parse_html_integer`],
/// with a negative result an error too. `"-0"` is 0.
pub fn parse_html_non_negative_integer(s: &str) -> Option<u32> {
    parse_html_integer(s).and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row measured in Chrome 153 as `div.tabIndex` (an error reads
    /// `-1` there, the not-set answer — the rows that are `-1` *valid* say so)
    /// and cross-checked against `ol.start` / `li.value`, which reflect the
    /// same parse.
    #[test]
    fn matches_chrome_153() {
        let rows: &[(&str, Option<i32>)] = &[
            (" 3", Some(3)),
            ("2.5", Some(2)),
            ("3abc", Some(3)),
            ("+2", Some(2)),
            ("3 ", Some(3)),
            ("00007", Some(7)),
            ("-01", Some(-1)),
            ("  -2", Some(-2)),
            ("-1.5", Some(-1)),
            ("\t\n\x0C\r 4", Some(4)),
            ("0", Some(0)),
            ("1e1", Some(1)),
            ("1e30", Some(1)),
            ("2147483647", Some(i32::MAX)),
            ("-2147483648", Some(i32::MIN)),
            // Errors.
            ("", None),
            ("-", None),
            ("+", None),
            ("+-2", None),
            ("- 2", None),
            ("abc", None),
            ("inf", None),
            ("NaN", None),
            ("99999999999", None),
            ("2147483648", None),
            ("-2147483649", None),
            // NBSP is not ASCII whitespace; fullwidth digits are not ASCII digits.
            ("\u{a0}5", None),
            ("\u{ff10}\u{ff13}", None),
        ];
        for &(s, want) in rows {
            assert_eq!(parse_html_integer(s), want, "{s:?}");
        }
        // Not measured; the algorithm's own answers.
        assert_eq!(parse_html_integer("-0"), Some(0));
        assert_eq!(parse_html_integer("   "), None);
    }

    #[test]
    fn non_negative_rejects_a_negative_and_keeps_zero() {
        assert_eq!(parse_html_non_negative_integer(" 3x"), Some(3));
        assert_eq!(parse_html_non_negative_integer("-0"), Some(0));
        assert_eq!(parse_html_non_negative_integer("-1"), None);
        assert_eq!(parse_html_non_negative_integer("abc"), None);
        assert_eq!(parse_html_non_negative_integer("2147483648"), None);
    }
}
