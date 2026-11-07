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
        "valign" => matches!(tag, "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"),
        "nowrap" => matches!(tag, "td" | "th"),
        "cellspacing" | "bordercolor" => tag == "table",
        "border" => matches!(tag, "table" | "img"),
        "hspace" | "vspace" => tag == "img",
        "color" => matches!(tag, "font" | "hr"),
        "face" | "size" => tag == "font",
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

/// The HTML Standard's rules for parsing integers: leading whitespace, an
/// optional sign, then digits; whatever follows them is ignored, so `"10px"`
/// is 10 and `"1e1"` is 1. `None` when there is no digit.
fn integer(v: &str) -> Option<i64> {
    let v = v.trim_start_matches(['\t', '\n', '\x0C', '\r', ' ']);
    let (negative, digits) = match v.as_bytes().first()? {
        b'-' => (true, &v[1..]),
        b'+' => (false, &v[1..]),
        _ => (false, v),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let n: i64 = digits[..end].parse().ok()?;
    Some(if negative { -n } else { n })
}

/// The rules for parsing non-negative integers: [`integer`], and an error
/// below zero.
fn non_negative_integer(v: &str) -> Option<f32> {
    integer(v).filter(|n| *n >= 0).map(|n| n as f32)
}

/// `cellpadding` as Chrome reads it: [`integer`], where an error and a
/// negative number are both 0. An empty value maps nothing (the UA's 1px).
fn cell_padding(v: &str) -> Option<f32> {
    (!v.is_empty()).then(|| integer(v).unwrap_or(0).max(0) as f32)
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

/// `<font size>`: 1 to 7, or relative to 3.
fn font_size(v: &str) -> Option<&'static str> {
    let v = v.trim();
    let n: i32 = v.trim_start_matches(['+', '-']).parse().ok()?;
    let n = match v.as_bytes().first()? {
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
            "img" => match a.as_str() {
                "top" | "middle" | "bottom" => write!(css, "vertical-align:{a};").unwrap(),
                _ => {}
            },
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
