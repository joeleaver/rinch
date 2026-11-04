//! `@media` rules at the width a document is shown at.
//!
//! A cleaned document's style sheets are applied inline, so a media query
//! cannot be left for the style engine to evaluate (and the engine would ask
//! the window's width, not the view's). Instead the queries are evaluated
//! here, against the width the document is drawn at: a block whose query
//! matches is unwrapped into its sheet, the others are dropped, and then the
//! sheet is inlined. [`signature`] says which blocks match at a width, so a
//! view can clean again only when a resize changes that.
//!
//! Supported: media types (`all`, `screen` match; `print`, `speech` do not),
//! `only`, `not`, comma lists, `and`, and the features `width`,
//! `min-width`, `max-width` and their `-device-` forms (the view's width
//! stands for the device's), in `px`, `em` and `rem` (16px), plus
//! `prefers-color-scheme: light`. Any other feature does not match.

/// Where a `@media` block sits in a sheet's text.
struct Block {
    /// From `@media` to the closing `}`.
    whole: std::ops::Range<usize>,
    /// The query, between `@media` and `{`.
    query: std::ops::Range<usize>,
    /// What is inside the braces.
    inside: std::ops::Range<usize>,
}

/// The `@media` queries in `document`'s `<style>` sheets, nested ones
/// included, in order. Read once per document: a view asks [`Queries::at`]
/// on each resize, which reads no document.
pub fn queries(document: &str) -> Queries {
    let mut out = Vec::new();
    for sheet in sheets(document) {
        collect(&document[sheet], &mut out);
    }
    Queries(out)
}

fn collect(css: &str, out: &mut Vec<String>) {
    for block in blocks(css) {
        out.push(css[block.query.clone()].trim().to_owned());
        collect(&css[block.inside], out);
    }
}

/// A document's media queries ([`queries`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queries(Vec<String>);

impl Queries {
    /// Which of them match at `width`: a view cleans again when this
    /// changes.
    pub fn at(&self, width: f32) -> Vec<bool> {
        self.0.iter().map(|q| matches(q, width)).collect()
    }
}

/// Which of `document`'s `@media` blocks match at `width`
/// (`queries(document).at(width)`).
pub fn signature(document: &str, width: f32) -> Vec<bool> {
    queries(document).at(width)
}

/// `document` with each sheet's `@media` blocks unwrapped where they match
/// at `width` (nested blocks too) and removed where they do not. Without a
/// width every block is removed.
pub(crate) fn resolve(document: &str, width: Option<f32>) -> String {
    let mut out = String::with_capacity(document.len());
    let mut at = 0;
    for sheet in sheets(document) {
        out.push_str(&document[at..sheet.start]);
        out.push_str(&resolve_css(&document[sheet.clone()], width));
        at = sheet.end;
    }
    out.push_str(&document[at..]);
    out
}

fn resolve_css(css: &str, width: Option<f32>) -> String {
    let mut out = String::with_capacity(css.len());
    let mut at = 0;
    for block in blocks(css) {
        out.push_str(&css[at..block.whole.start]);
        if width.is_some_and(|w| matches(&css[block.query.clone()], w)) {
            out.push_str(&resolve_css(&css[block.inside.clone()], width));
        }
        at = block.whole.end;
    }
    out.push_str(&css[at..]);
    out
}

/// The text ranges inside each `<style>…</style>` element.
fn sheets(document: &str) -> Vec<std::ops::Range<usize>> {
    let lower = document.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = lower[from..].find("<style") {
        let open = from + open;
        let Some(gt) = lower[open..].find('>') else {
            break;
        };
        let start = open + gt + 1;
        let Some(close) = lower[start..].find("</style") else {
            break;
        };
        out.push(start..start + close);
        from = start + close;
    }
    out
}

/// The top-level `@media` blocks in `css`, skipping comments and strings.
fn blocks(css: &str) -> Vec<Block> {
    let bytes = css.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = comment_end(bytes, i);
                continue;
            }
            q @ (b'"' | b'\'') => {
                i = skip_string(bytes, i, q);
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'@' if depth == 0
                && css[i..].len() >= 6
                && css[i + 1..i + 6].eq_ignore_ascii_case("media") =>
            {
                let query_start = i + 6;
                let Some(open) = css[query_start..].find('{').map(|o| query_start + o) else {
                    break;
                };
                let Some(close) = matching_brace(bytes, open) else {
                    break;
                };
                out.push(Block {
                    whole: i..close + 1,
                    query: query_start..open,
                    inside: open + 1..close,
                });
                i = close + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Past the `*/` closing the comment opened at `start`.
fn comment_end(bytes: &[u8], start: usize) -> usize {
    bytes[start + 2..]
        .windows(2)
        .position(|w| w == b"*/")
        .map_or(bytes.len(), |e| start + 2 + e + 2)
}

fn skip_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The `}` closing the `{` at `open`.
fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = comment_end(bytes, i);
                continue;
            }
            q @ (b'"' | b'\'') => {
                i = skip_string(bytes, i, q);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether a media query list matches at `width`.
pub(crate) fn matches(list: &str, width: f32) -> bool {
    list.split(',').any(|query| one(query, width))
}

fn one(query: &str, width: f32) -> bool {
    let lower = query.trim().to_ascii_lowercase();
    let (negated, rest) = match lower.strip_prefix("not ") {
        Some(rest) => (true, rest.trim()),
        None => (false, lower.as_str()),
    };
    let rest = rest.strip_prefix("only ").unwrap_or(rest).trim();
    let mut ok = true;
    let mut parts = rest.split(" and ").map(str::trim).peekable();
    // A leading media type.
    if let Some(first) = parts.peek()
        && !first.starts_with('(')
    {
        ok = matches!(*first, "all" | "screen" | "");
        parts.next();
    }
    for part in parts {
        ok &= feature(part, width);
    }
    ok != negated
}

/// One `(name: value)` feature.
fn feature(part: &str, width: f32) -> bool {
    let Some(inner) = part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) else {
        return false;
    };
    let Some((name, value)) = inner.split_once(':') else {
        return false;
    };
    let (name, value) = (name.trim(), value.trim());
    if name == "prefers-color-scheme" {
        return value == "light";
    }
    let Some(px) = length(value) else {
        return false;
    };
    match name {
        "min-width" | "min-device-width" => width >= px,
        "max-width" | "max-device-width" => width <= px,
        "width" | "device-width" => (width - px).abs() < 0.5,
        _ => false,
    }
}

fn length(value: &str) -> Option<f32> {
    let (number, unit) = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map_or((value, ""), |at| value.split_at(at));
    let n: f32 = number.parse().ok()?;
    match unit.trim() {
        "px" | "" => Some(n),
        "em" | "rem" => Some(n * 16.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_sheet_of_comments_is_read_in_linear_time() {
        // A 1 MB sheet of comments inside @media took 2.1 s per resize.
        let comments = "/* x */".repeat(150_000);
        let doc =
            format!("<style>@media (max-width: 600px) {{ {comments} .a{{color:red}} }}</style>");
        let t = std::time::Instant::now();
        assert_eq!(signature(&doc, 400.0), [true]);
        assert!(resolve(&doc, Some(400.0)).contains("color:red"));
        assert!(t.elapsed().as_millis() < 500, "took {:?}", t.elapsed());
    }

    #[test]
    fn queries_match_as_a_browser_would_at_a_width() {
        assert!(matches("only screen and (max-width: 600px)", 480.0));
        assert!(!matches("only screen and (max-width: 600px)", 720.0));
        assert!(matches("(min-width: 40em)", 720.0));
        assert!(!matches("print", 720.0));
        assert!(matches(
            "print, screen and (max-device-width: 480px)",
            400.0
        ));
        assert!(matches("not all and (max-width: 600px)", 720.0));
        assert!(!matches("(prefers-color-scheme: dark)", 720.0));
        assert!(!matches("(orientation: portrait)", 720.0));
    }

    #[test]
    fn matching_blocks_are_unwrapped_and_the_rest_removed() {
        let doc = "<style>.a{color:red} @media (max-width: 600px) { .a{color:blue} @media (max-width: 300px) { .a{color:green} } } /* @media x { } */ .b{content:'}'}</style><p class=a>x</p>";
        let narrow = resolve(doc, Some(400.0));
        assert!(narrow.contains(".a{color:blue}"), "{narrow}");
        assert!(!narrow.contains("green"), "{narrow}");
        assert!(!narrow.contains("@media (max"), "{narrow}");
        let wide = resolve(doc, Some(800.0));
        assert!(!wide.contains("blue"), "{wide}");
        assert!(wide.contains(".b{content:'}'}"), "{wide}");
        assert_eq!(signature(doc, 400.0), [true, false]);
        assert_eq!(signature(doc, 300.0), [true, true]);
        assert_eq!(signature(doc, 800.0), [false, false]);
        // A nested block is seen on its own: 300 and 400 differ in it.
        let nested = "<style>@media screen { @media (max-width: 350px) { .a{color:red} } }</style>";
        assert_ne!(signature(nested, 300.0), signature(nested, 400.0));
    }
}
