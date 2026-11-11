//! Presentational hints: what the HTML Standard's rendering section maps
//! legacy attributes to (#1419).
//!
//! Each element's mapping is CSS text, parsed into
//! `Node::presentational_hints`, so an author rule beats it as it does in a
//! browser. Attribute values are not trusted: a value goes into the text only
//! through a filter that admits nothing that could end a declaration.
//!
//! `cellpadding` and a table's `border` reach the cells, and a cell can be
//! attached to a table after its hints are built, so those two are not stored
//! on the cell: [`cell_hints_css`] reads them as the cell is cascaded.

use crate::node::Node;
use rinch_core::dom::{HtmlDimension, parse_html_dimension};
use std::fmt::Write;

/// Whether writing `name` on `tag` can change that element's hints.
pub(crate) fn maps_attribute(tag: &str, name: &str) -> bool {
    match name {
        "bgcolor" => matches!(
            tag,
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "body"
        ),
        "width" => matches!(tag, "table" | "td" | "th" | "col" | "colgroup" | "hr"),
        "height" => matches!(tag, "table" | "tr" | "td" | "th"),
        "align" => matches!(
            tag,
            "table"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "td"
                | "th"
                | "p"
                | "div"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "img"
                | "caption"
        ),
        "valign" => matches!(
            tag,
            "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "img"
        ),
        "nowrap" => matches!(tag, "td" | "th"),
        "cellspacing" | "bordercolor" => tag == "table",
        "border" => matches!(tag, "table" | "img"),
        "hspace" | "vspace" => tag == "img",
        "color" => matches!(tag, "font" | "hr"),
        "face" => tag == "font",
        "size" => matches!(tag, "font" | "hr"),
        "text" => tag == "body",
        _ => false,
    }
}

/// Whether a table's attribute changes what its cells map.
pub(crate) fn reaches_cells(tag: &str, name: &str) -> bool {
    tag == "table" && matches!(name, "cellpadding" | "border" | "bordercolor")
}

/// A colour attribute as CSS: a name, a hash colour, or the bare hex legacy
/// mail writes (`bgcolor="ff0000"`).
fn color(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '#') {
        return None;
    }
    let hex = v.strip_prefix('#').unwrap_or(v);
    let is_hex = matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit());
    Some(if is_hex {
        format!("#{hex}")
    } else {
        v.to_owned()
    })
}

/// A dimension value as CSS: a length in px or a percentage (`width`,
/// `height`, `hspace`, `vspace`).
fn dimension(v: &str) -> Option<String> {
    match parse_html_dimension(v)? {
        HtmlDimension::Length(v) => Some(format!("{v}px")),
        HtmlDimension::Percentage(v) => Some(format!("{v}%")),
    }
}

/// The HTML Standard's whitespace, which its integer parsers skip.
const HTML_SPACE: [char; 5] = ['\t', '\n', '\x0C', '\r', ' '];
/// What Chrome's lenient `ToInt` skips (`cellpadding`, `<hr size>`): the
/// HTML whitespace and a vertical tab.
const TO_INT_SPACE: [char; 6] = ['\t', '\n', '\x0B', '\x0C', '\r', ' '];

/// An optional sign, then digits; whatever follows them is ignored, so
/// `"10px"` is 10 and `"1e1"` is 1. `None` when there is no digit, or when
/// the number is too long to be any `i32` or `u32` (an overflow is an error
/// in Chrome, not a clamp).
fn signed_digits(v: &str) -> Option<(bool, i64)> {
    let (negative, digits) = match v.as_bytes().first()? {
        b'-' => (true, &v[1..]),
        b'+' => (false, &v[1..]),
        _ => (false, v),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let mut n: i64 = 0;
    for b in digits[..end].bytes() {
        n = n * 10 + i64::from(b - b'0');
        if n > i64::from(u32::MAX) + 1 {
            return None;
        }
    }
    (end > 0).then_some((negative, n))
}

/// The rules for parsing non-negative integers, as Chrome parses them into
/// a `u32`: below zero or past `u32::MAX` is an error (`"-0"` is 0).
fn non_negative_integer(v: &str) -> Option<f32> {
    let (negative, n) = signed_digits(v.trim_start_matches(HTML_SPACE))?;
    let n = if negative { -n } else { n };
    u32::try_from(n).ok().map(|n| n as f32)
}

/// Chrome's lenient `ToInt` (`cellpadding`, `<hr size>`): an `i32` by the
/// rules for parsing integers, also skipping a leading vertical tab, where
/// an error (an overflow too) is 0.
fn to_int(v: &str) -> i32 {
    signed_digits(v.trim_start_matches(TO_INT_SPACE))
        .and_then(|(negative, n)| i32::try_from(if negative { -n } else { n }).ok())
        .unwrap_or(0)
}

/// `cellpadding` as Chrome reads it: [`to_int`], where an error and a
/// negative number are both 0. An empty value maps nothing (the UA's 1px).
fn cell_padding(v: &str) -> Option<f32> {
    (!v.is_empty()).then(|| to_int(v).max(0) as f32)
}

/// A table's `border`: a non-negative integer, and 1 when it fails to parse
/// (an empty `border` is a 1px border, as in Chrome).
fn table_border(v: &str) -> f32 {
    non_negative_integer(v).unwrap_or(1.0)
}

/// A length by the rules for parsing dimension values (`"10px"` and
/// `"10.5"` are lengths; the rest of the value is ignored), with no
/// percentage: `cellspacing`.
fn dimension_length(v: &str) -> Option<f64> {
    match parse_html_dimension(v)? {
        HtmlDimension::Length(v) => Some(v),
        HtmlDimension::Percentage(_) => None,
    }
}

/// `<font face>`: a family list, each name a CSS string.
fn families(v: &str) -> String {
    v.split(',')
        .map(|f| {
            let name: String = f
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
                .collect();
            format!("\"{}\"", name.trim())
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `<font size>`: 1 to 7, or relative to 3, by the rules for parsing a
/// legacy font size: a sign makes it relative, the digits after it count
/// and the rest is ignored (`"4px"` is 4, `"+1abc"` is 4).
fn font_size(v: &str) -> Option<&'static str> {
    let v = v.trim_start_matches(HTML_SPACE);
    // Chrome reads the digits as an `i32`; past `i32::MAX` they are 0
    // (`"2147483648"` is size 1, `"+2147483648"` is 3).
    let (_, n) = signed_digits(v)?;
    let n = if n > i64::from(i32::MAX) {
        0
    } else {
        n.min(100) as i32
    };
    let n = match v.as_bytes()[0] {
        b'+' => 3 + n,
        b'-' => 3 - n,
        _ => n,
    };
    Some(match n.clamp(1, 7) {
        1 => "x-small",
        2 => "small",
        3 => "medium",
        4 => "large",
        5 => "x-large",
        6 => "xx-large",
        _ => "xxx-large",
    })
}

/// The declarations `node`'s own attributes map to.
pub(crate) fn hints_css(node: &Node) -> Option<String> {
    let tag = node.tag()?;
    let attr = |name: &str| node.attributes.get(name).map(String::as_str);
    let mut css = String::new();
    let part = matches!(tag, "thead" | "tbody" | "tfoot" | "tr" | "td" | "th");

    if let Some(c) = attr("bgcolor").and_then(color)
        && maps_attribute(tag, "bgcolor")
    {
        write!(css, "background-color:{c};").unwrap();
    }
    if maps_attribute(tag, "width")
        && let Some(v) = attr("width").and_then(dimension)
    {
        write!(css, "width:{v};").unwrap();
    }
    if maps_attribute(tag, "height")
        && let Some(v) = attr("height").and_then(dimension)
    {
        write!(css, "height:{v};").unwrap();
    }
    if let Some(a) = attr("align").map(|a| a.trim().to_ascii_lowercase()) {
        let text = matches!(a.as_str(), "left" | "right" | "center" | "justify");
        // These align the blocks under them too (HTML rendering §15.3.3):
        // Stylo's `-moz-` values, which a table does not inherit.
        let aligns_blocks = matches!(
            tag,
            "div" | "caption" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
        );
        match tag {
            "table" if a == "center" => css.push_str("margin-left:auto;margin-right:auto;"),
            "table" | "img" if a == "left" || a == "right" => {
                write!(css, "float:{a};").unwrap();
            }
            // Chrome 157's keywords (Blink's `ApplyAlignmentAttributeToStyle`):
            // `bottom` is the baseline, `absbottom` the bottom, `center` is
            // `middle`. Mapped as `bottom`, a picture alone on its line lost
            // the strut's descent below it, 5px short of Chrome's line
            // (#2475). Blink's `middle` and `center` are
            // `-webkit-baseline-middle` (the box's middle on the baseline),
            // which rinch has not got: `middle` is 1px off in a 30px picture.
            "img" => {
                let v = match a.as_str() {
                    "top" => Some("top"),
                    "middle" | "center" | "absmiddle" | "abscenter" => Some("middle"),
                    "bottom" | "baseline" => Some("baseline"),
                    "absbottom" => Some("bottom"),
                    "texttop" => Some("text-top"),
                    _ => None,
                };
                if let Some(v) = v {
                    write!(css, "vertical-align:{v};").unwrap();
                }
            }
            "table" => {}
            _ if aligns_blocks && matches!(a.as_str(), "center" | "middle") => {
                css.push_str("text-align:-moz-center;");
            }
            _ if aligns_blocks && matches!(a.as_str(), "left" | "right") => {
                write!(css, "text-align:-moz-{a};").unwrap();
            }
            _ if text && maps_attribute(tag, "align") => {
                write!(css, "text-align:{a};").unwrap();
            }
            _ => {}
        }
    }
    if part && let Some(v) = attr("valign").map(|a| a.trim().to_ascii_lowercase()) {
        if matches!(v.as_str(), "top" | "middle" | "bottom" | "baseline") {
            write!(css, "vertical-align:{v};").unwrap();
        }
    }
    // Chrome 157 maps an `<img>`'s `valign` too, as the `vertical-align` it
    // names (Blink's `HTMLImageElement`; no standard does): mail writes
    // `<img valign="bottom">` alone in a line, which Chrome lines up by its
    // bottom, with no strut descent under it (#2475). After `align`, which
    // it overrides there.
    if tag == "img"
        && let Some(v) = attr("valign").map(|a| a.trim().to_ascii_lowercase())
        && matches!(
            v.as_str(),
            "top" | "middle" | "bottom" | "baseline" | "text-top" | "text-bottom" | "sub" | "super"
        )
    {
        write!(css, "vertical-align:{v};").unwrap();
    }
    match tag {
        "td" | "th" if attr("nowrap").is_some() => css.push_str("white-space:nowrap;"),
        "table" => {
            if let Some(v) = attr("cellspacing").and_then(dimension_length) {
                write!(css, "border-spacing:{v}px;").unwrap();
            }
            if let Some(v) = attr("border").map(table_border).filter(|v| *v > 0.0) {
                let c = attr("bordercolor").and_then(color).unwrap_or_default();
                write!(css, "border:{v}px outset {c};").unwrap();
            }
        }
        "img" => {
            if let Some(v) = attr("hspace").and_then(dimension) {
                write!(css, "margin-left:{v};margin-right:{v};").unwrap();
            }
            if let Some(v) = attr("vspace").and_then(dimension) {
                write!(css, "margin-top:{v};margin-bottom:{v};").unwrap();
            }
            if let Some(v) = attr("border").and_then(non_negative_integer) {
                write!(css, "border:{v}px solid;").unwrap();
            }
        }
        "font" => {
            if let Some(c) = attr("color").and_then(color) {
                write!(css, "color:{c};").unwrap();
            }
            if let Some(f) = attr("face") {
                write!(css, "font-family:{};", families(f)).unwrap();
            }
            if let Some(s) = attr("size").and_then(font_size) {
                write!(css, "font-size:{s};").unwrap();
            }
        }
        "body" => {
            if let Some(c) = attr("text").and_then(color) {
                write!(css, "color:{c};").unwrap();
            }
        }
        "hr" => {
            if let Some(c) = attr("color").and_then(color) {
                write!(css, "color:{c};background-color:{c};").unwrap();
            }
            // Chrome: a `size` of 1 or less (an error is 0) drops the
            // bottom border; a larger one is the height, less the two
            // borders.
            if let Some(v) = attr("size") {
                match to_int(v) {
                    n if n <= 1 => css.push_str("border-bottom-width:0;"),
                    n => write!(css, "height:{}px;", n - 2).unwrap(),
                }
            }
        }
        _ => {}
    }
    (!css.is_empty()).then_some(css)
}

/// What a cell's table maps onto it: `cellpadding`, and a thin border when
/// the table has `border`.
pub(crate) fn cell_hints_css(table: &Node) -> Option<String> {
    let mut css = String::new();
    if let Some(v) = table
        .attributes
        .get("cellpadding")
        .and_then(|v| cell_padding(v))
    {
        write!(css, "padding:{v}px;").unwrap();
    }
    if table
        .attributes
        .get("border")
        .is_some_and(|v| table_border(v) > 0.0)
    {
        let c = table.attributes.get("bordercolor").and_then(|v| color(v));
        let c = c.unwrap_or_default();
        write!(css, "border:1px inset {c};").unwrap();
    }
    (!css.is_empty()).then_some(css)
}
