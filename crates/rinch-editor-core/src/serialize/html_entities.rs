//! Character references of the HTML reader ([`super::html`]).
//!
//! Numeric references (`&#233;`, `&#xE9;`) by HTML's rules, the Windows-1252
//! remapping of `&#128;`..`&#159;` included (Word writes `&#146;` for an
//! apostrophe), and the 252 named references of HTML 4 plus `&apos;`. A named
//! reference needs its `;`, so `?a=1&copy=2` in a URL is left alone; a name
//! not in the table stays as written, which is what a browser shows for it.

/// Named references, sorted by name (byte order) for a binary search.
static NAMED: [(&str, char); 253] = [
    ("AElig", '\u{c6}'), ("Aacute", '\u{c1}'), ("Acirc", '\u{c2}'), ("Agrave", '\u{c0}'),
    ("Alpha", '\u{391}'), ("Aring", '\u{c5}'), ("Atilde", '\u{c3}'), ("Auml", '\u{c4}'),
    ("Beta", '\u{392}'), ("Ccedil", '\u{c7}'), ("Chi", '\u{3a7}'), ("Dagger", '\u{2021}'),
    ("Delta", '\u{394}'), ("ETH", '\u{d0}'), ("Eacute", '\u{c9}'), ("Ecirc", '\u{ca}'),
    ("Egrave", '\u{c8}'), ("Epsilon", '\u{395}'), ("Eta", '\u{397}'), ("Euml", '\u{cb}'),
    ("Gamma", '\u{393}'), ("Iacute", '\u{cd}'), ("Icirc", '\u{ce}'), ("Igrave", '\u{cc}'),
    ("Iota", '\u{399}'), ("Iuml", '\u{cf}'), ("Kappa", '\u{39a}'), ("Lambda", '\u{39b}'),
    ("Mu", '\u{39c}'), ("Ntilde", '\u{d1}'), ("Nu", '\u{39d}'), ("OElig", '\u{152}'),
    ("Oacute", '\u{d3}'), ("Ocirc", '\u{d4}'), ("Ograve", '\u{d2}'), ("Omega", '\u{3a9}'),
    ("Omicron", '\u{39f}'), ("Oslash", '\u{d8}'), ("Otilde", '\u{d5}'), ("Ouml", '\u{d6}'),
    ("Phi", '\u{3a6}'), ("Pi", '\u{3a0}'), ("Prime", '\u{2033}'), ("Psi", '\u{3a8}'),
    ("Rho", '\u{3a1}'), ("Scaron", '\u{160}'), ("Sigma", '\u{3a3}'), ("THORN", '\u{de}'),
    ("Tau", '\u{3a4}'), ("Theta", '\u{398}'), ("Uacute", '\u{da}'), ("Ucirc", '\u{db}'),
    ("Ugrave", '\u{d9}'), ("Upsilon", '\u{3a5}'), ("Uuml", '\u{dc}'), ("Xi", '\u{39e}'),
    ("Yacute", '\u{dd}'), ("Yuml", '\u{178}'), ("Zeta", '\u{396}'), ("aacute", '\u{e1}'),
    ("acirc", '\u{e2}'), ("acute", '\u{b4}'), ("aelig", '\u{e6}'), ("agrave", '\u{e0}'),
    ("alefsym", '\u{2135}'), ("alpha", '\u{3b1}'), ("amp", '\u{26}'), ("and", '\u{2227}'),
    ("ang", '\u{2220}'), ("apos", '\u{27}'), ("aring", '\u{e5}'), ("asymp", '\u{2248}'),
    ("atilde", '\u{e3}'), ("auml", '\u{e4}'), ("bdquo", '\u{201e}'), ("beta", '\u{3b2}'),
    ("brvbar", '\u{a6}'), ("bull", '\u{2022}'), ("cap", '\u{2229}'), ("ccedil", '\u{e7}'),
    ("cedil", '\u{b8}'), ("cent", '\u{a2}'), ("chi", '\u{3c7}'), ("circ", '\u{2c6}'),
    ("clubs", '\u{2663}'), ("cong", '\u{2245}'), ("copy", '\u{a9}'), ("crarr", '\u{21b5}'),
    ("cup", '\u{222a}'), ("curren", '\u{a4}'), ("dArr", '\u{21d3}'), ("dagger", '\u{2020}'),
    ("darr", '\u{2193}'), ("deg", '\u{b0}'), ("delta", '\u{3b4}'), ("diams", '\u{2666}'),
    ("divide", '\u{f7}'), ("eacute", '\u{e9}'), ("ecirc", '\u{ea}'), ("egrave", '\u{e8}'),
    ("empty", '\u{2205}'), ("emsp", '\u{2003}'), ("ensp", '\u{2002}'), ("epsilon", '\u{3b5}'),
    ("equiv", '\u{2261}'), ("eta", '\u{3b7}'), ("eth", '\u{f0}'), ("euml", '\u{eb}'),
    ("euro", '\u{20ac}'), ("exist", '\u{2203}'), ("fnof", '\u{192}'), ("forall", '\u{2200}'),
    ("frac12", '\u{bd}'), ("frac14", '\u{bc}'), ("frac34", '\u{be}'), ("frasl", '\u{2044}'),
    ("gamma", '\u{3b3}'), ("ge", '\u{2265}'), ("gt", '\u{3e}'), ("hArr", '\u{21d4}'),
    ("harr", '\u{2194}'), ("hearts", '\u{2665}'), ("hellip", '\u{2026}'), ("iacute", '\u{ed}'),
    ("icirc", '\u{ee}'), ("iexcl", '\u{a1}'), ("igrave", '\u{ec}'), ("image", '\u{2111}'),
    ("infin", '\u{221e}'), ("int", '\u{222b}'), ("iota", '\u{3b9}'), ("iquest", '\u{bf}'),
    ("isin", '\u{2208}'), ("iuml", '\u{ef}'), ("kappa", '\u{3ba}'), ("lArr", '\u{21d0}'),
    ("lambda", '\u{3bb}'), ("lang", '\u{2329}'), ("laquo", '\u{ab}'), ("larr", '\u{2190}'),
    ("lceil", '\u{2308}'), ("ldquo", '\u{201c}'), ("le", '\u{2264}'), ("lfloor", '\u{230a}'),
    ("lowast", '\u{2217}'), ("loz", '\u{25ca}'), ("lrm", '\u{200e}'), ("lsaquo", '\u{2039}'),
    ("lsquo", '\u{2018}'), ("lt", '\u{3c}'), ("macr", '\u{af}'), ("mdash", '\u{2014}'),
    ("micro", '\u{b5}'), ("middot", '\u{b7}'), ("minus", '\u{2212}'), ("mu", '\u{3bc}'),
    ("nabla", '\u{2207}'), ("nbsp", '\u{a0}'), ("ndash", '\u{2013}'), ("ne", '\u{2260}'),
    ("ni", '\u{220b}'), ("not", '\u{ac}'), ("notin", '\u{2209}'), ("nsub", '\u{2284}'),
    ("ntilde", '\u{f1}'), ("nu", '\u{3bd}'), ("oacute", '\u{f3}'), ("ocirc", '\u{f4}'),
    ("oelig", '\u{153}'), ("ograve", '\u{f2}'), ("oline", '\u{203e}'), ("omega", '\u{3c9}'),
    ("omicron", '\u{3bf}'), ("oplus", '\u{2295}'), ("or", '\u{2228}'), ("ordf", '\u{aa}'),
    ("ordm", '\u{ba}'), ("oslash", '\u{f8}'), ("otilde", '\u{f5}'), ("otimes", '\u{2297}'),
    ("ouml", '\u{f6}'), ("para", '\u{b6}'), ("part", '\u{2202}'), ("permil", '\u{2030}'),
    ("perp", '\u{22a5}'), ("phi", '\u{3c6}'), ("pi", '\u{3c0}'), ("piv", '\u{3d6}'),
    ("plusmn", '\u{b1}'), ("pound", '\u{a3}'), ("prime", '\u{2032}'), ("prod", '\u{220f}'),
    ("prop", '\u{221d}'), ("psi", '\u{3c8}'), ("quot", '\u{22}'), ("rArr", '\u{21d2}'),
    ("radic", '\u{221a}'), ("rang", '\u{232a}'), ("raquo", '\u{bb}'), ("rarr", '\u{2192}'),
    ("rceil", '\u{2309}'), ("rdquo", '\u{201d}'), ("real", '\u{211c}'), ("reg", '\u{ae}'),
    ("rfloor", '\u{230b}'), ("rho", '\u{3c1}'), ("rlm", '\u{200f}'), ("rsaquo", '\u{203a}'),
    ("rsquo", '\u{2019}'), ("sbquo", '\u{201a}'), ("scaron", '\u{161}'), ("sdot", '\u{22c5}'),
    ("sect", '\u{a7}'), ("shy", '\u{ad}'), ("sigma", '\u{3c3}'), ("sigmaf", '\u{3c2}'),
    ("sim", '\u{223c}'), ("spades", '\u{2660}'), ("sub", '\u{2282}'), ("sube", '\u{2286}'),
    ("sum", '\u{2211}'), ("sup", '\u{2283}'), ("sup1", '\u{b9}'), ("sup2", '\u{b2}'),
    ("sup3", '\u{b3}'), ("supe", '\u{2287}'), ("szlig", '\u{df}'), ("tau", '\u{3c4}'),
    ("there4", '\u{2234}'), ("theta", '\u{3b8}'), ("thetasym", '\u{3d1}'), ("thinsp", '\u{2009}'),
    ("thorn", '\u{fe}'), ("tilde", '\u{2dc}'), ("times", '\u{d7}'), ("trade", '\u{2122}'),
    ("uArr", '\u{21d1}'), ("uacute", '\u{fa}'), ("uarr", '\u{2191}'), ("ucirc", '\u{fb}'),
    ("ugrave", '\u{f9}'), ("uml", '\u{a8}'), ("upsih", '\u{3d2}'), ("upsilon", '\u{3c5}'),
    ("uuml", '\u{fc}'), ("weierp", '\u{2118}'), ("xi", '\u{3be}'), ("yacute", '\u{fd}'),
    ("yen", '\u{a5}'), ("yuml", '\u{ff}'), ("zeta", '\u{3b6}'), ("zwj", '\u{200d}'),
    ("zwnj", '\u{200c}'),
];

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

/// The reference at the start of `s` (which starts after the `&`): the
/// character it stands for and how many bytes of `s` it takes.
fn reference(s: &str) -> Option<(char, usize)> {
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
        return Some((numeric(code), end));
    }
    // No name in the table is longer than 8 letters.
    let len = bytes
        .iter()
        .take(10)
        .position(|b| !b.is_ascii_alphanumeric())?;
    if bytes[len] != b';' {
        return None;
    }
    let name = &s[..len];
    let at = NAMED.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
    Some((NAMED[at].1, len + 1))
}

/// `s` with its character references replaced, in one pass.
pub(super) fn decode_entities(s: &str) -> String {
    let Some(first) = s.find('&') else {
        return s.to_string();
    };
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..first]);
    let mut rest = &s[first..];
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        match reference(rest) {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_for_the_search() {
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn references_decode_as_a_browser_decodes_them() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&#39;"), "a & b <c> \"d\" 'e'");
        assert_eq!(decode_entities("&nbsp;&eacute;&Eacute;&mdash;&hellip;&rsquo;&euro;"), "\u{a0}\u{e9}\u{c9}\u{2014}\u{2026}\u{2019}\u{20ac}");
        assert_eq!(decode_entities("&#233;&#xE9;&#Xe9;&#x1F600;&#233"), "\u{e9}\u{e9}\u{e9}\u{1f600}\u{e9}");
        // Word's apostrophe, an en dash and a bullet: Windows-1252.
        assert_eq!(decode_entities("&#146;&#150;&#149;"), "\u{2019}\u{2013}\u{2022}");
        // Not a character: U+FFFD, as in a browser.
        assert_eq!(decode_entities("&#0;&#xD800;&#x110000;&#99999999999;"), "\u{fffd}\u{fffd}\u{fffd}\u{fffd}");
        // One pass: what a reference decodes to is not decoded again.
        assert_eq!(decode_entities("&amp;lt; &amp;amp; &amp;#233;"), "&lt; &amp; &#233;");
        // Not references: left as written.
        assert_eq!(decode_entities("R&D &nosuch; &copy=2 &# &#x; & &;"), "R&D &nosuch; &copy=2 &# &#x; & &;");
        assert_eq!(decode_entities("multi\u{e9}&amp;\u{4e2d}&"), "multi\u{e9}&\u{4e2d}&");
    }
}
