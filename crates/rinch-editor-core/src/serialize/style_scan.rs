//! A quote- and bracket-aware top-level split of a CSS inline-style string
//! (issue #705).
//!
//! `parse_style` in `serialize::html` reads one property out of a pasted
//! element's `style` attribute. It used to split on a bare `;` and look for
//! the first `:` in each piece — wrong wherever the value itself carries
//! either byte, which is exactly what pasted HTML turns up:
//! `background-image: url(data:image/png;base64,…)` has a `;` inside the
//! `url(...)` token, and `content: "a;b"` has one inside a string. Either one
//! fabricates a false declaration boundary, and the only way that is
//! observable here is a *wrong* value coming back for a property nobody
//! declared — `parse_style` never writes, so it cannot corrupt the attribute.
//!
//! This is a private, read-only copy of the top-level split half of
//! `rinch_core::dom::inline_style::split_declarations` (#670): the same
//! quote/escape/bracket rules, with no comment stripping (this parser never
//! had any, and the issue is about declaration boundaries, not comments),
//! no `!important`/duplicate handling (`parse_style` wants the first
//! top-level match, not CSSOM's last-with-priority collapse) and no property
//! normalisation (the caller already compares case-insensitively). Kept here
//! rather than reused because `rinch-editor-core` is the pure, wasm-clean
//! editor model and depends on no rinch crate; the copy is kept honest by a
//! dev-dependency differential test against `rinch_core::dom` (see
//! `html_integer.rs` for the same pattern over a different parser).
//!
//! `strip_css_variables` in `rinch-visual-test` has the same shape of bug —
//! Refs #705 — but that crate is a test harness and can take the `rinch-core`
//! dependency directly rather than carry a second copy of this.

/// Feeds one byte of top-level tracking: quotes, backslash escapes (anywhere,
/// not just inside a string — matching `unquoted_url_end`'s own escape rule
/// so the two never disagree about where a token ends) and bracket depth.
/// Returns whether byte `i` is a *top-level* byte a separator search may act
/// on.
struct Scanner {
    depth: u32,
    quote: Option<u8>,
    escaped: bool,
}

impl Scanner {
    fn new() -> Self {
        Self {
            depth: 0,
            quote: None,
            escaped: false,
        }
    }

    fn step(&mut self, bytes: &[u8], i: usize) -> bool {
        let b = bytes[i];
        if self.escaped {
            self.escaped = false;
            return false;
        }
        if b == b'\\' {
            self.escaped = true;
            return false;
        }
        if let Some(q) = self.quote {
            if b == q {
                self.quote = None;
            }
            return false;
        }
        match b {
            b'"' | b'\'' => {
                self.quote = Some(b);
                false
            }
            b'(' | b'[' | b'{' => {
                self.depth += 1;
                false
            }
            b')' | b']' | b'}' => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            _ => self.depth == 0,
        }
    }
}

/// Split `css` into top-level declarations on a bare `;` — one inside
/// `"…"`/`'…'` or any bracketed run (`url(...)` included) is part of the
/// value, not a separator.
fn split_top_level(css: &str) -> Vec<&str> {
    let bytes = css.as_bytes();
    let mut scanner = Scanner::new();
    let mut parts = Vec::new();
    let mut start = 0usize;
    for i in 0..bytes.len() {
        if scanner.step(bytes, i) && bytes[i] == b';' {
            parts.push(&css[start..i]);
            start = i + 1;
        }
    }
    parts.push(&css[start..]);
    parts
}

/// The byte index of the first top-level `:` in `part`, honouring the same
/// quote/bracket rules.
fn top_level_colon(part: &str) -> Option<usize> {
    let bytes = part.as_bytes();
    let mut scanner = Scanner::new();
    (0..bytes.len()).find(|&i| scanner.step(bytes, i) && bytes[i] == b':')
}

/// Scan a CSS inline-style string for one property's value, by **name**
/// (compared by the caller, not here — `parse_style` folds case itself).
///
/// Quote- and bracket-aware the same way
/// `rinch_core::dom::inline_style::split_declarations` is: a `;` or `:`
/// inside a quoted string or a bracketed run (`url(...)` included) is part of
/// the value. Returns every top-level `(name, value)` pair found, both sides
/// trimmed but otherwise verbatim — the caller matches the name and filters
/// the value itself.
pub(crate) fn style_declarations(css: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for part in split_top_level(css) {
        let part_trimmed = part.trim();
        if part_trimmed.is_empty() {
            continue;
        }
        // `top_level_colon` must scan the untrimmed part so byte offsets line
        // up with it, but the leading whitespace trim above can shift them —
        // scan `part` itself and trim the two halves afterward.
        let Some(colon) = top_level_colon(part) else {
            continue;
        };
        let name = part[..colon].trim();
        let value = part[colon + 1..].trim();
        if name.is_empty() {
            continue;
        }
        out.push((name, value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_plain_style_string() {
        assert_eq!(
            style_declarations("color: red; text-align: center"),
            vec![("color", "red"), ("text-align", "center")]
        );
    }

    #[test]
    fn a_semicolon_inside_a_data_url_is_not_a_separator() {
        let css = "background-image: url(data:image/png;base64,AAAA==); color: red";
        assert_eq!(
            style_declarations(css),
            vec![
                ("background-image", "url(data:image/png;base64,AAAA==)"),
                ("color", "red"),
            ]
        );
    }

    #[test]
    fn a_semicolon_inside_a_quoted_string_is_not_a_separator() {
        assert_eq!(
            style_declarations(r#"content: "a;b"; color: red"#),
            vec![("content", r#""a;b""#), ("color", "red")]
        );
    }

    #[test]
    fn an_escaped_quote_inside_a_string_does_not_end_it() {
        // `\"` inside the double-quoted string does not close it, so the `;`
        // right after is still inside the string — matching
        // `rinch_core::dom::inline_style`'s own escape rule.
        assert_eq!(
            style_declarations(r#"content: "a\";b"; color: red"#),
            vec![("content", r#""a\";b""#), ("color", "red")]
        );
    }

    #[test]
    fn a_colon_inside_a_url_is_part_of_the_value_not_a_second_declaration() {
        assert_eq!(
            style_declarations("background-image: url(http://a/x.png)"),
            vec![("background-image", "url(http://a/x.png)")]
        );
    }

    #[test]
    fn differential_against_rinch_core_split_declarations() {
        let cases = [
            "color: red; text-align: center",
            "background-image: url(data:image/png;base64,AAAA==); color: red",
            r#"content: "a;b"; color: red"#,
            r#"content: "a\";b"; color: red"#,
            "background-image: url(http://a/x.png)",
            r"background-image: url(http://a/x\)y;z.png); color: red",
            "  color : red ;  gap:4px;  ",
            "",
            ";;;",
            "color",
            "color:",
            ":red",
        ];
        for css in cases {
            let want: Vec<(String, String)> = rinch_core::dom::split_declarations(css);
            let got: Vec<(String, String)> = style_declarations(css)
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                .collect();
            // `split_declarations` also normalises property case, collapses
            // duplicates, and strips comments — none of which `parse_style`
            // needs — so compare only where none of those apply: no
            // duplicate names and no comments in the input. Every case above
            // qualifies, which is what makes this a meaningful check rather
            // than a tautology.
            assert_eq!(got, want, "{css:?}");
        }
    }
}
