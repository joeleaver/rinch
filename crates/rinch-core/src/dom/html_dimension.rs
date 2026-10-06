//! HTML's rules for parsing dimension values (issue #684).
//!
//! The `width` and `height` attributes of `<img>`, `<video>`, `<iframe>` and
//! the other elements HTML maps them for are *dimension attributes*: a length
//! in CSS pixels or a percentage, read by an algorithm of their own (HTML
//! § 2.3.4.4), not by the integer rules ([`super::parse_html_integer`]) and
//! not by a CSS parser.
//!
//! 1. skip leading **ASCII whitespace**;
//! 2. at least one ASCII digit, else an **error** — no sign, no leading `.`;
//! 3. the digits, then an optional `.` and fraction digits;
//! 4. a `%` right there makes it a percentage; anything else after the
//!    number is ignored (`"100px"` and `"100abc"` are 100 pixels).
//!
//! One departure from the Standard, which is Chrome 153's: a `*` right after
//! the number is a legacy *relative* length (`<col width="2*">`), and it is an
//! **error** here — `<img width="50*">` is 0 wide in Chrome, not 50.

/// A parsed dimension attribute: CSS pixels or a percentage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HtmlDimension {
    /// A length in CSS pixels.
    Length(f64),
    /// A percentage (`50.0` for `"50%"`).
    Percentage(f64),
}

/// HTML's *rules for parsing dimension values*: the value of `s`, or `None`
/// for an error. Zero is a value (`"0"` is a 0px length). See the module doc
/// for the algorithm.
pub fn parse_html_dimension(s: &str) -> Option<HtmlDimension> {
    let bytes = s.as_bytes();
    let mut i = bytes
        .iter()
        .position(|b| !matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' '))?;
    if !bytes[i].is_ascii_digit() {
        return None;
    }
    let mut value = 0f64;
    while let Some(b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        value = value * 10.0 + f64::from(b - b'0');
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let mut divisor = 1f64;
        while let Some(b) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
            divisor *= 10.0;
            value += f64::from(b - b'0') / divisor;
            i += 1;
        }
    }
    if !value.is_finite() {
        return None;
    }
    match bytes.get(i) {
        Some(b'%') => Some(HtmlDimension::Percentage(value)),
        Some(b'*') => None,
        _ => Some(HtmlDimension::Length(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::HtmlDimension::{Length, Percentage};
    use super::*;

    /// Every row measured in Chrome 153 as the laid-out width of
    /// `<img width=…>` in a 400px container (an error is 0 wide, like `"0"`;
    /// the two are told apart there by the mapped `aspect-ratio`).
    #[test]
    fn matches_chrome_153() {
        let rows: &[(&str, Option<HtmlDimension>)] = &[
            ("100", Some(Length(100.0))),
            ("100px", Some(Length(100.0))),
            ("100abc", Some(Length(100.0))),
            (" 100", Some(Length(100.0))),
            ("\t\n\x0C\r 50", Some(Length(50.0))),
            ("100.7", Some(Length(100.7))),
            ("1e2", Some(Length(1.0))),
            ("0", Some(Length(0.0))),
            ("50%", Some(Percentage(50.0))),
            ("12.5%", Some(Percentage(12.5))),
            // Errors.
            ("", None),
            (".5", None),
            ("+100", None),
            ("-5", None),
            ("abc", None),
            ("%", None),
            ("50*", None),
        ];
        for &(s, want) in rows {
            assert_eq!(parse_html_dimension(s), want, "{s:?}");
        }
        // Not measured; the algorithm's own answers.
        assert_eq!(parse_html_dimension("   "), None);
        assert_eq!(parse_html_dimension("7."), Some(Length(7.0)));
        assert_eq!(parse_html_dimension("7.%"), Some(Percentage(7.0)));
        assert_eq!(parse_html_dimension("7 %"), Some(Length(7.0)));
        assert_eq!(parse_html_dimension("\u{a0}5"), None);
        assert_eq!(parse_html_dimension(&"9".repeat(400)), None);
    }
}
