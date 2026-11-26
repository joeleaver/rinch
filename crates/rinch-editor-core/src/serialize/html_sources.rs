//! What the HTML reader knows about the applications people paste from: a
//! table of known sources, each with how its copy is recognised and the
//! colour it writes for text that has no colour of its own (#3096).
//!
//! Some applications write the full character formatting of every run as
//! CSS, their defaults included. Most defaults read as nothing already
//! (`font-weight:400`, `font-style:normal`, `text-decoration:none`,
//! `background-color:transparent`). The default text colour does not: Google
//! Docs writes `color:#000000` on every run whose colour is "automatic",
//! which is not a colour the writer chose. Read as a `text_color` mark it
//! would pin every pasted word black (unreadable on a dark theme) where the
//! editor's own text colour belongs. So inside a known source's copy, a
//! colour equal to that source's default is no colour.
//!
//! **Not what Chrome does, on purpose.** Chrome drops a pasted colour that
//! equals the *editing host's* computed colour, so on a light theme Docs'
//! black goes and on a dark theme it stays, black on dark. rinch's reader has
//! no editing host to ask, and a colour that depends on the theme at the
//! moment of the paste would change meaning with the theme. It keeps the
//! *source's own default* out instead, whatever the editor's theme.
//!
//! The rule is scoped to the source's wrapper element: a copy of a Gmail
//! message that quotes a Docs paste keeps the black Gmail itself wrote around
//! it. Content from an unknown source keeps every colour. Word and
//! LibreOffice are not in the table yet: each needs a real clipboard sample
//! to say how it is recognised and what its default colour is.

use super::html_tree::ParsedNode;
use super::style_scan::style_declarations;

/// The id prefix of the element Google Docs wraps a copy in.
const DOCS_GUID_PREFIX: &str = "docs-internal-guid-";

/// Black, opaque: `#000`, `#000000`, `rgb(0, 0, 0)`, `black`.
const BLACK: Rgba = [0, 0, 0, 255];

/// A colour as red, green, blue and alpha, each 0–255.
type Rgba = [u8; 4];

/// An application whose copies the reader recognises.
struct Source {
    /// Whether an element (its tag and kept attributes) is the element the
    /// application wraps its copy in. What is inside it is the copy.
    is_wrapper: fn(&str, &[(String, String)]) -> bool,
    /// The colour the application writes for text with no colour of its own.
    default_colour: Rgba,
}

/// The known sources.
const SOURCES: &[Source] = &[
    // Google Docs: `<b style="font-weight:normal" id="docs-internal-guid-…">`
    // around the whole copy, `color:#000000` on every automatic-colour run.
    Source {
        is_wrapper: is_docs_wrapper,
        default_colour: BLACK,
    },
    // Google Sheets: `<google-sheets-html-origin>` around the copy, and
    // `data-sheets-root` on its table (or on the one cell's `<span>`); a
    // rich-text cell's runs are `<span style="…color:#000000">`.
    Source {
        is_wrapper: is_sheets_wrapper,
        default_colour: BLACK,
    },
];

/// Whether `id` is the id Google Docs gives the element it wraps a copy in.
/// The tokenizer keeps this one `id` for the Docs entry of [`SOURCES`].
pub(super) fn is_docs_guid(id: &str) -> bool {
    id.starts_with(DOCS_GUID_PREFIX)
}

fn is_docs_wrapper(_tag: &str, attributes: &[(String, String)]) -> bool {
    attributes.iter().any(|(k, v)| k == "id" && is_docs_guid(v))
}

fn is_sheets_wrapper(tag: &str, attributes: &[(String, String)]) -> bool {
    tag == "google-sheets-html-origin" || attributes.iter().any(|(k, _)| k == "data-sheets-root")
}

/// Take a known source's default text colour out of the `style` of every
/// element inside that source's wrapper in `forest`, so the reader gives it
/// no `text_color` mark. Every other colour, and every colour outside a
/// known source's wrapper, is left as written.
pub(super) fn drop_source_default_colours(forest: &mut [ParsedNode]) {
    // An explicit stack: pasted markup can nest deeper than the call stack.
    let mut stack: Vec<(&mut ParsedNode, Option<Rgba>)> =
        forest.iter_mut().map(|n| (n, None)).collect();
    while let Some((node, mut default)) = stack.pop() {
        let ParsedNode::Element {
            tag,
            attributes,
            children,
            ..
        } = node
        else {
            continue;
        };
        if let Some(source) = SOURCES.iter().find(|s| (s.is_wrapper)(tag, attributes)) {
            default = Some(source.default_colour);
        }
        if let Some(colour) = default {
            drop_colour(attributes, colour);
        }
        stack.extend(children.iter_mut().map(|c| (c, default)));
    }
}

/// Remove every `color` declaration whose colour is `colour` from the
/// element's `style`; the other declarations stay as they were.
fn drop_colour(attributes: &mut [(String, String)], colour: Rgba) {
    let Some((_, style)) = attributes.iter_mut().find(|(k, _)| k == "style") else {
        return;
    };
    let declarations = style_declarations(style);
    let is_default = |(name, value): &(&str, &str)| {
        name.eq_ignore_ascii_case("color") && parse_colour(value) == Some(colour)
    };
    if !declarations.iter().any(is_default) {
        return;
    }
    let kept: Vec<String> = declarations
        .iter()
        .filter(|d| !is_default(d))
        .map(|(name, value)| format!("{name}:{value}"))
        .collect();
    *style = kept.join(";");
}

/// A CSS colour value as RGBA: hex (`#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa`), `rgb()`/`rgba()` (comma or space separated, numbers or
/// percentages, an optional `/ alpha`), or the names `black` and `white`.
/// Anything else (another name, `hsl()`, `currentcolor`) is `None`, which no
/// source default equals, so such a colour is always kept.
fn parse_colour(value: &str) -> Option<Rgba> {
    let v = value.trim().trim_end_matches("!important").trim();
    if let Some(hex) = v.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = v.to_ascii_lowercase();
    match lower.as_str() {
        "black" => return Some(BLACK),
        "white" => return Some([255, 255, 255, 255]),
        _ => {}
    }
    let args = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = args
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    let (rgb, alpha) = match parts.as_slice() {
        [r, g, b] => ([*r, *g, *b], None),
        [r, g, b, a] => ([*r, *g, *b], Some(*a)),
        _ => return None,
    };
    let channel = |p: &str| -> Option<u8> {
        match p.strip_suffix('%') {
            Some(pct) => unit(pct.parse::<f64>().ok()? / 100.0),
            None => unit(p.parse::<f64>().ok()? / 255.0),
        }
    };
    let alpha = match alpha {
        None => 255,
        Some(a) => match a.strip_suffix('%') {
            Some(pct) => unit(pct.parse::<f64>().ok()? / 100.0)?,
            None => unit(a.parse::<f64>().ok()?)?,
        },
    };
    Some([channel(rgb[0])?, channel(rgb[1])?, channel(rgb[2])?, alpha])
}

/// `fraction` (clamped to 0–1) as a 0–255 channel; `None` for NaN.
fn unit(fraction: f64) -> Option<u8> {
    if fraction.is_nan() {
        return None;
    }
    // Clamped to 0..=255 first, so the cast cannot truncate.
    Some((fraction.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 | 4 => {
            let short = |i| digit(i).map(|d| d * 17);
            let alpha = if hex.len() == 4 { short(3)? } else { 255 };
            Some([short(0)?, short(1)?, short(2)?, alpha])
        }
        6 | 8 => {
            let alpha = if hex.len() == 8 { pair(6)? } else { 255 };
            Some([pair(0)?, pair(2)?, pair(4)?, alpha])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::html_tree::HtmlFragmentParser;

    fn forest(html: &str) -> Vec<ParsedNode> {
        HtmlFragmentParser::new(html).parse()
    }

    /// The `style` of every element in `forest`, in document order.
    fn styles(forest: &[ParsedNode]) -> Vec<String> {
        let mut out = Vec::new();
        for node in forest {
            if let ParsedNode::Element {
                attributes,
                children,
                ..
            } = node
            {
                if let Some((_, s)) = attributes.iter().find(|(k, _)| k == "style") {
                    out.push(s.clone());
                }
                out.extend(styles(children));
            }
        }
        out
    }

    fn after(html: &str) -> Vec<String> {
        let mut f = forest(html);
        drop_source_default_colours(&mut f);
        styles(&f)
    }

    #[test]
    fn every_spelling_of_black_is_parsed_as_black() {
        for black in [
            "#000",
            "#000000",
            "#000F",
            "#000000ff",
            "rgb(0, 0, 0)",
            "rgb(0 0 0)",
            "rgba(0,0,0,1)",
            "rgb(0% 0% 0% / 100%)",
            "black",
            "BLACK",
            " #000000 !important",
        ] {
            assert_eq!(parse_colour(black), Some(BLACK), "{black}");
        }
        for other in [
            "#000001",
            "#0000",
            "rgba(0,0,0,0.5)",
            "rgb(1,0,0)",
            "navy",
            "hsl(0, 0%, 0%)",
            "currentcolor",
            "#00",
            "#00000g",
        ] {
            assert_ne!(parse_colour(other), Some(BLACK), "{other}");
        }
    }

    #[test]
    fn a_docs_copy_loses_its_black_and_keeps_its_other_colours() {
        assert_eq!(
            after(
                "<b id=\"docs-internal-guid-1\"><p><span style=\"font-size:11pt;color:#000000;\">a</span>\
                 <span style=\"color:rgb(0, 0, 0)\">b</span><span style=\"color:black\">c</span>\
                 <span style=\"color:#1155cc\">d</span></p></b>"
            ),
            ["font-size:11pt", "", "", "color:#1155cc"]
        );
    }

    #[test]
    fn a_sheets_copy_loses_its_black() {
        assert_eq!(
            after(
                "<google-sheets-html-origin><table><tr><td><span style=\"color:#000000;font-weight:bold\">a</span>\
                 </td></tr></table></google-sheets-html-origin>"
            ),
            ["font-weight:bold"]
        );
        assert_eq!(
            after("<span data-sheets-root=\"1\" style=\"color:#000\">a</span>"),
            [""]
        );
    }

    #[test]
    fn black_outside_a_known_wrapper_is_kept() {
        // A Gmail message quoting a Docs paste: Gmail's own black stays.
        assert_eq!(
            after(
                "<div><span style=\"color:#000000\">mine</span><blockquote>\
                 <b id=\"docs-internal-guid-2\"><span style=\"color:#000000\">quoted</span></b>\
                 </blockquote></div>"
            ),
            ["color:#000000", ""]
        );
        // No known source at all: every colour is kept.
        assert_eq!(
            after("<p><span style=\"color:#000000\">a</span></p>"),
            ["color:#000000"]
        );
        assert_eq!(
            after("<b style=\"font-weight:normal;\"><span style=\"color:#000\">a</span></b>"),
            ["font-weight:normal;", "color:#000"]
        );
    }
}
