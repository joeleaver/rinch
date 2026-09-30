//! HTML's rules for parsing integers, for the HTML import (issue #1164).
//!
//! `<ol start>`, `colspan` and `rowspan` are read by the WHATWG *rules for
//! parsing (non-negative) integers* (HTML § 2.3.4.1), not by `str::parse`:
//! skip leading ASCII whitespace (tab, LF, FF, CR, space — not NBSP), an
//! optional `-` or `+`, then at least one ASCII digit; the number ends at the
//! first non-digit, and anything after it is ignored. So `" 3"`, `"3abc"` and
//! `"2.5"` are 3, 3 and 2, where `str::parse` refused all three.
//!
//! This is a private copy of `rinch_core::dom::parse_html_integer` (#1138,
//! #1153): this crate is the pure, wasm-clean model and depends on no rinch
//! crate, and the algorithm is twenty lines. The copy is kept honest by a
//! dev-dependency test that runs both over one table of inputs.
//!
//! The table-span reader is Chrome's, not a general HTML rule: Chrome reads
//! `colSpan` / `rowSpan` with a *clamped* non-negative integer parse, where a
//! value too large for the parse is the **maximum**, not an error — measured in
//! Chrome 153, `colspan="99999999999"` is 1000 and `rowspan="99999999999"` is
//! 65534, while `ol.start` with the same value is an error (the default 1).

/// The sign and magnitude HTML's integer rules read from `s`, the magnitude
/// saturated at `u64::MAX`; `None` for an error (no digit where one is due).
fn scan(s: &str) -> Option<(bool, u64)> {
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
    let mut magnitude: u64 = 0;
    let mut any = false;
    for b in bytes.take_while(u8::is_ascii_digit) {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(b - b'0'));
        any = true;
    }
    any.then_some((negative, magnitude))
}

/// HTML's *rules for parsing integers*: the value of `s`, or `None` for an
/// error. A value outside `i32` is an error, as in Chrome 153 (`ol.start` of
/// `"2147483648"` is the default 1).
pub(crate) fn parse_html_integer(s: &str) -> Option<i32> {
    let (negative, magnitude) = scan(s)?;
    let value = if negative {
        -i64::try_from(magnitude).ok()?
    } else {
        i64::try_from(magnitude).ok()?
    };
    i32::try_from(value).ok()
}

/// Chrome's clamped non-negative integer parse, as `td.colSpan` / `td.rowSpan`
/// use it: the value of `s` clamped to `min..=max`, a value too large for the
/// parse clamped to `max`, and `None` — the caller's default — for an error or
/// a negative value (`"-0"` is 0).
pub(crate) fn parse_html_clamped_non_negative_integer(s: &str, min: u32, max: u32) -> Option<u32> {
    let (negative, magnitude) = scan(s)?;
    if negative && magnitude != 0 {
        return None;
    }
    let clamped = magnitude.clamp(u64::from(min), u64::from(max));
    Some(u32::try_from(clamped).unwrap_or(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs both copies of the integer parse are run over: rinch-core's
    /// Chrome 153 table, its review rows, and the boundaries of the
    /// saturation this copy adds.
    const SHARED: &[&str] = &[
        " 3",
        "2.5",
        "3abc",
        "+2",
        "3 ",
        "00007",
        "-01",
        "  -2",
        "-1.5",
        "\t\n\x0C\r 4",
        "0",
        "-0",
        "1e1",
        "1e30",
        "2147483647",
        "-2147483648",
        "",
        "   ",
        "-",
        "+",
        "+-2",
        "- 2",
        "abc",
        "inf",
        "NaN",
        "99999999999",
        "2147483648",
        "-2147483649",
        "2147483650",
        "-2147483650",
        "21474836470",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "99999999999999999999999",
        "-99999999999999999999999",
        "\u{a0}5",
        "\u{ff10}\u{ff13}",
    ];

    /// The copy answers exactly what `rinch_core::dom::parse_html_integer`
    /// answers, on every shared input.
    #[test]
    fn matches_rinch_core() {
        for &s in SHARED {
            assert_eq!(
                parse_html_integer(s),
                rinch_core::dom::parse_html_integer(s),
                "{s:?}"
            );
        }
    }

    /// Wherever rinch-core's non-negative parse has a value, the clamped parse
    /// is that value clamped; where it is an error, the clamped parse is an
    /// error too unless the input is a non-negative overflow, which clamps to
    /// the maximum.
    #[test]
    fn clamped_agrees_with_rinch_core_off_overflow() {
        for &s in SHARED {
            let want = match rinch_core::dom::parse_html_non_negative_integer(s) {
                Some(n) => Some(n.clamp(1, 1000)),
                None => match scan(s) {
                    Some((false, _)) => Some(1000),
                    Some((true, 0)) => unreachable!("-0 is 0 for rinch-core too"),
                    _ => None,
                },
            };
            assert_eq!(
                parse_html_clamped_non_negative_integer(s, 1, 1000),
                want,
                "{s:?}"
            );
        }
    }

    /// Chrome 153's `td.colSpan` / `td.rowSpan`, measured.
    #[test]
    fn clamped_matches_chrome_153() {
        let colspan = |s| parse_html_clamped_non_negative_integer(s, 1, 1000);
        let rowspan = |s| parse_html_clamped_non_negative_integer(s, 0, 65534);
        assert_eq!(colspan("0"), Some(1));
        assert_eq!(colspan("-0"), Some(1));
        assert_eq!(colspan("-3"), None);
        assert_eq!(colspan("1001"), Some(1000));
        assert_eq!(colspan("3000000000"), Some(1000));
        assert_eq!(colspan("99999999999999999999999"), Some(1000));
        assert_eq!(rowspan("0"), Some(0));
        assert_eq!(rowspan("-0"), Some(0));
        assert_eq!(rowspan("65535"), Some(65534));
        assert_eq!(rowspan("99999999999"), Some(65534));
    }
}
