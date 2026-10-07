//! Character references of the HTML reader ([`super::html`]).
//!
//! Numeric references (`&#233;`, `&#xE9;`) by HTML's rules, the Windows-1252
//! remapping of `&#128;`..`&#159;` included (Word writes `&#146;` for an
//! apostrophe), and the named references of the HTML standard: 2125 names
//! ([`super::html_entities_table`], generated from the standard's
//! `entities.json`), one or two characters each.
//!
//! A name is matched as the HTML tokenizer matches it. With its `;` it is
//! the whole run of letters and digits after the `&` (`&notin;`). 106 older
//! names also decode with no `;`, and then the longest of them the run
//! starts with is the reference (`&notit;` is `¬it;`, `&ampx` is `&x`).
//! In an attribute value such a reference is left alone when `=`, a letter
//! or a digit follows it, so `?a=1&copy=2` in a URL stays as written.
//! Anything else after an `&` is text.

use super::html_entities_table::{
    COUNT, LEGACY, MAX_LEGACY, MAX_NAME, NAME_ENDS, NAMES, VALUE_ENDS, VALUES,
};

/// The `i`th name and whether it decodes with no `;`.
fn name(i: usize) -> (&'static str, bool) {
    let start = if i == 0 {
        0
    } else {
        NAME_ENDS[i - 1] & !LEGACY
    };
    let end = NAME_ENDS[i];
    (
        &NAMES[start as usize..(end & !LEGACY) as usize],
        end & LEGACY != 0,
    )
}

/// The characters the `i`th name stands for.
fn value(i: usize) -> &'static str {
    let start = if i == 0 { 0 } else { VALUE_ENDS[i - 1] };
    &VALUES[start as usize..VALUE_ENDS[i] as usize]
}

/// The index of `wanted` among the names.
fn find(wanted: &str) -> Option<usize> {
    let (mut low, mut high) = (0, COUNT);
    while low < high {
        let mid = low + (high - low) / 2;
        match name(mid).0.cmp(wanted) {
            std::cmp::Ordering::Equal => return Some(mid),
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Greater => high = mid,
        }
    }
    None
}

/// What `&#128;`..`&#159;` mean in HTML: Windows-1252, not C1 controls.
static WINDOWS_1252: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];

/// The character a numeric reference to `code` stands for.
fn numeric(code: u32) -> char {
    match code {
        0 => '\u{fffd}',
        0x80..=0x9f => WINDOWS_1252[(code - 0x80) as usize],
        _ => char::from_u32(code).unwrap_or('\u{fffd}'),
    }
}

/// The reference at the start of `s` (which starts after the `&`): what it
/// stands for is added to `out`, and the bytes of `s` it takes returned.
fn reference(s: &str, in_attribute: bool, out: &mut String) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.first() == Some(&b'#') {
        let hex = matches!(bytes.get(1), Some(b'x' | b'X'));
        let start = if hex { 2 } else { 1 };
        let radix = if hex { 16 } else { 10 };
        let mut end = start;
        let mut code: u32 = 0;
        while let Some(digit) = bytes.get(end).and_then(|b| (*b as char).to_digit(radix)) {
            // Saturate: anything past the last code point is U+FFFD anyway.
            code = code.saturating_mul(radix).saturating_add(digit);
            end += 1;
        }
        if end == start {
            return None;
        }
        if bytes.get(end) == Some(&b';') {
            end += 1;
        }
        out.push(numeric(code));
        return Some(end);
    }
    // The letters and digits after the `&`, as far as a name can reach (a
    // longer run has a letter or digit there, where a name has its `;`).
    let run = bytes
        .iter()
        .take(MAX_NAME)
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    if bytes.get(run) == Some(&b';')
        && let Some(i) = find(&s[..run])
    {
        out.push_str(value(i));
        return Some(run + 1);
    }
    // No `;`, or no such name: the longest legacy name the run starts with.
    for len in (2..=run.min(MAX_LEGACY)).rev() {
        let Some(i) = find(&s[..len]).filter(|i| name(*i).1) else {
            continue;
        };
        if in_attribute
            && bytes
                .get(len)
                .is_some_and(|b| *b == b'=' || b.is_ascii_alphanumeric())
        {
            return None;
        }
        out.push_str(value(i));
        return Some(len);
    }
    None
}

/// `s`, the text of an element, with its character references replaced, in
/// one pass.
pub(super) fn decode_entities(s: &str) -> String {
    decode(s, false)
}

/// `s`, an attribute's value, with its character references replaced.
pub(super) fn decode_attribute(s: &str) -> String {
    decode(s, true)
}

fn decode(s: &str, in_attribute: bool) -> String {
    let Some(first) = s.find('&') else {
        return s.to_string();
    };
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..first]);
    let mut rest = &s[first..];
    while let Some(at) = rest.find('&') {
        // A count for the tests that pin how reading scales.
        super::html_tree::step();
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        match reference(rest, in_attribute, &mut out) {
            Some(len) => rest = &rest[len..],
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated table is what `find` and `reference` take it for.
    #[test]
    fn the_table_is_sorted_and_within_its_bounds() {
        let names: Vec<(&str, bool)> = (0..COUNT).map(name).collect();
        assert!(names.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(names.len(), 2125);
        let legacy: Vec<&str> = names.iter().filter(|n| n.1).map(|n| n.0).collect();
        // 2125 names with a `;` and 106 without: the standard's 2231.
        assert_eq!(legacy.len(), 106);
        assert_eq!(names.iter().map(|n| n.0.len()).max(), Some(MAX_NAME));
        assert_eq!(legacy.iter().map(|n| n.len()).max(), Some(MAX_LEGACY));
        assert!(legacy.iter().all(|n| n.len() >= 2));
        assert!(
            names
                .iter()
                .all(|n| n.0.bytes().all(|b| b.is_ascii_alphanumeric()))
        );
        assert_eq!(NAME_ENDS[COUNT - 1] & !LEGACY, NAMES.len() as u16);
        assert_eq!(VALUE_ENDS[COUNT - 1] as usize, VALUES.len());
        for i in 0..COUNT {
            assert_eq!(find(name(i).0), Some(i));
            let chars = value(i).chars().count();
            assert!(chars == 1 || chars == 2, "{}", name(i).0);
        }
        // One of each end of the table, and of each kind.
        let get = |n: &str| find(n).map(value);
        assert_eq!(get("AElig"), Some("\u{c6}"));
        assert_eq!(get("zwnj"), Some("\u{200c}"));
        assert_eq!(get("amp"), Some("&"));
        assert_eq!(get("NotEqualTilde"), Some("\u{2242}\u{338}"));
        assert_eq!(get("Amp"), None);
    }

    #[test]
    fn references_decode_as_a_browser_decodes_them() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&#39;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(
            decode_entities("&nbsp;&eacute;&Eacute;&mdash;&hellip;&rsquo;&euro;"),
            "\u{a0}\u{e9}\u{c9}\u{2014}\u{2026}\u{2019}\u{20ac}"
        );
        assert_eq!(
            decode_entities("&#233;&#xE9;&#Xe9;&#x1F600;&#233"),
            "\u{e9}\u{e9}\u{e9}\u{1f600}\u{e9}"
        );
        // Word's apostrophe, an en dash and a bullet: Windows-1252.
        assert_eq!(
            decode_entities("&#146;&#150;&#149;"),
            "\u{2019}\u{2013}\u{2022}"
        );
        // Not a character: U+FFFD, as in a browser.
        assert_eq!(
            decode_entities("&#0;&#xD800;&#x110000;&#99999999999;"),
            "\u{fffd}\u{fffd}\u{fffd}\u{fffd}"
        );
        // One pass: what a reference decodes to is not decoded again.
        assert_eq!(
            decode_entities("&amp;lt; &amp;amp; &amp;#233;"),
            "&lt; &amp; &#233;"
        );
        // Not references: left as written.
        assert_eq!(
            decode_entities("R&D &nosuch; &lang &# &#x; & &; &a &a1;"),
            "R&D &nosuch; &lang &# &#x; & &; &a &a1;"
        );
        // A legacy name needs no `;` (#1415); in an attribute it does when
        // `=`, a letter or a digit follows.
        assert_eq!(
            decode_entities("&copy=2 &notit; &notin; &ampamp; &middot1 &Aacutes &de"),
            "\u{a9}=2 \u{ac}it; \u{2209} &amp; \u{b7}1 \u{c1}s &de"
        );
        assert_eq!(
            decode_attribute("?a&copy=2&reg2&notit;&amp;&lt&gt &copy;&copy"),
            "?a&copy=2&reg2&notit;&<> \u{a9}\u{a9}"
        );
        // A run longer than any name, at the end of the input and before it.
        let name = "CounterClockwiseContourIntegral";
        assert_eq!(name.len(), MAX_NAME);
        assert_eq!(
            decode_entities(&format!("&{name}; &{name} &{name}s;")),
            format!("\u{2233} &{name} &{name}s;")
        );
        let long = "a".repeat(MAX_NAME + 1);
        assert_eq!(decode_entities(&format!("&{long}")), format!("&{long}"));
        assert_eq!(
            decode_entities(&format!("&{long}; &amp{long};")),
            format!("&{long}; &{long};")
        );
        assert_eq!(
            decode_entities("multi\u{e9}&amp;\u{4e2d}&"),
            "multi\u{e9}&\u{4e2d}&"
        );
    }
}
