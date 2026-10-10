//! An app's `data-*` attributes on a node ([`NodeSpec::data_attrs`]).
//!
//! HTML gives every element *custom data attributes* for the page's own use:
//! `data-` followed by a name. A node whose schema opts in keeps any such
//! attribute an app sets on it, as a string, through every edit, copy and
//! paste (HTML), the durable [`DocNode`](crate::serialize) shape, collaboration
//! (one CRDT entry per attribute) and the view, which writes it on the node's
//! host element. The starter kit's `image` opts in, so an app can keep what it
//! needs to find again — an annotation id, a reference into its own store —
//! on the picture itself.
//!
//! Two filters decide which names are kept, and every reader and writer asks
//! [`is_app_data_attr`], which applies both:
//!
//! - the name is a **valid custom data attribute** (HTML: `data-` and at least
//!   one character after it, XML-compatible, no ASCII uppercase —
//!   [`is_data_attr_name`]);
//! - rinch does **not reserve** it ([`is_reserved_data_attr`]): the names its
//!   own view and runtime read and write on elements (`data-pm-*`, `data-rid`,
//!   the `data-on*` event attributes, …). A reserved name pasted in from HTML
//!   is markup rinch wrote, never an app's, and stamping one on a host element
//!   would change what rinch does with it.
//!
//! [`NodeSpec::data_attrs`]: crate::schema::NodeSpec::data_attrs

use crate::model::{AttrValue, Attrs};

/// Name prefixes rinch reserves: the editor view's own markers (`data-pm-*`),
/// every event-handler attribute the runtime dispatches on (`data-onclick`,
/// `data-ondragstart`, …) and anything rinch names after itself.
pub const RESERVED_DATA_ATTR_PREFIXES: &[&str] = &["data-pm-", "data-on", "data-rinch-"];

/// Whole names rinch reserves: attributes its runtime reads on any element
/// (`data-rid` is a click handler's id, `data-nofocus` … `data-backdrop` change
/// focus and pointer handling, `data-viewport*` / `data-render-surface` make a
/// compositing hole, `data-drag-window` drags the window), the state the shell
/// writes on elements, and the task-list markup the HTML reader reads on
/// `<ul>` / `<li>` (TipTap's `data-type` / `data-checked`).
pub const RESERVED_DATA_ATTRS: &[&str] = &[
    "data-rid",
    "data-nofocus",
    "data-disabled",
    "data-trap-focus",
    "data-backdrop",
    "data-scroll-lock-exempt",
    "data-viewport",
    "data-viewport-ready",
    "data-render-surface",
    "data-drag-window",
    "data-focused",
    "data-text-sel",
    "data-preedit",
    "data-type",
    "data-checked",
];

/// Whether `name` is a valid HTML custom data attribute name: `data-`, then
/// at least one character, with no ASCII uppercase letter, and XML-compatible
/// (the XML `Name` production, without `:`).
pub fn is_data_attr_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("data-") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| !c.is_ascii_uppercase() && is_xml_name_char(c))
}

/// XML 1.0 `NameChar`, without `:` (HTML's "XML-compatible").
fn is_xml_name_char(c: char) -> bool {
    matches!(c,
        'a'..='z' | 'A'..='Z' | '_' | '-' | '.' | '0'..='9'
        | '\u{B7}'
        | '\u{C0}'..='\u{D6}'
        | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}'
        | '\u{203F}'..='\u{2040}'
        | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

/// Whether rinch reserves the data attribute `name` for itself
/// ([`RESERVED_DATA_ATTRS`], [`RESERVED_DATA_ATTR_PREFIXES`]).
pub fn is_reserved_data_attr(name: &str) -> bool {
    RESERVED_DATA_ATTRS.contains(&name)
        || RESERVED_DATA_ATTR_PREFIXES
            .iter()
            .any(|p| name.starts_with(p))
}

/// Whether `name` is an attribute an opted-in node keeps for an app: a valid
/// custom data attribute name rinch does not reserve.
pub fn is_app_data_attr(name: &str) -> bool {
    is_data_attr_name(name) && !is_reserved_data_attr(name)
}

/// The app data attributes in `attrs` with a string value, in name order: what
/// an opted-in node carries out to HTML and to its host element.
pub fn app_data_attrs(attrs: &Attrs) -> impl Iterator<Item = (&str, &str)> {
    attrs.iter().filter_map(|(k, v)| match v {
        AttrValue::Str(s) if is_app_data_attr(k) => Some((k, &**s)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_are_htmls() {
        for ok in [
            "data-ref",
            "data-annotation-id",
            "data-1",
            "data-a.b_c",
            "data-é",
            "data-\u{10000}",
            "data--",
        ] {
            assert!(is_data_attr_name(ok), "{ok}");
        }
        for bad in [
            "data-",
            "data",
            "ref",
            "data-Ref",
            "data-a:b",
            "data-a b",
            "data-a/b",
            "data-a=b",
            "data-a\"",
            "data-\u{0}",
            "Data-x",
            "data-×",
        ] {
            assert!(!is_data_attr_name(bad), "{bad}");
        }
    }

    /// The one exclusion list: every name and prefix in it is refused, and a
    /// name that merely starts like a reserved whole name is not.
    #[test]
    fn reserved_names_are_refused_and_only_those() {
        for name in RESERVED_DATA_ATTRS {
            assert!(is_data_attr_name(name), "{name} is a data attribute");
            assert!(!is_app_data_attr(name), "{name}");
        }
        for prefix in RESERVED_DATA_ATTR_PREFIXES {
            let name = format!("{prefix}x");
            assert!(is_data_attr_name(&name), "{name}");
            assert!(!is_app_data_attr(&name), "{name}");
        }
        for name in [
            "data-ridge",
            "data-types",
            "data-checked-at",
            "data-pmx",
            "data-o",
        ] {
            assert!(is_app_data_attr(name), "{name}");
        }
    }

    #[test]
    fn app_data_attrs_takes_string_app_data_attrs_only() {
        let attrs = Attrs::from_iter([
            ("src", AttrValue::from("a.png")),
            ("data-ref", AttrValue::from("r")),
            ("data-n", AttrValue::Int(1)),
            ("data-rid", AttrValue::from("1")),
        ]);
        assert_eq!(
            app_data_attrs(&attrs).collect::<Vec<_>>(),
            [("data-ref", "r")]
        );
    }
}
