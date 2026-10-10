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
//! Every reader and writer asks [`is_app_data_attr`] which names are kept:
//!
//! - the name is a **valid custom data attribute** (HTML: `data-` and at least
//!   one character after it, XML-compatible, no ASCII uppercase —
//!   [`is_data_attr_name`]), of at most [`MAX_DATA_ATTR_NAME`] bytes;
//! - rinch does **not reserve** it ([`is_reserved_data_attr`]): the names its
//!   own view, shell and runtime read and write on elements (`data-pm-*`,
//!   `data-rid`, the `data-on…` event attributes, the shell's selection and
//!   caret state, …). A reserved name pasted in from HTML is markup rinch
//!   wrote, never an app's, and stamping one on a host element would change
//!   what rinch does with it (a pasted `data-tcm-item` made a press on the
//!   picture run a text-menu item). `tests/data_attr_ratchet.rs` scans every
//!   crate's sources for `"data-…"` literals and fails on one that is neither
//!   reserved nor on its list of names that are safe to carry.
//!
//! And [`kept_data_attrs`] bounds how many a node keeps: at most
//! [`MAX_DATA_ATTRS`] attributes, each value at most [`MAX_DATA_ATTR_VALUE`]
//! bytes. Past either bound the attribute is **dropped whole** (the first ones
//! are kept), never truncated: a value cut short would be a different value.
//! Every route into the model that validates applies it — the HTML reader,
//! attribute validation (`DocNode` load, `to_doc`) — and so do the HTML writer
//! and the view, so an attribute set past the bound by a raw
//! `SetNodeAttrStep` is neither written out nor shown.
//!
//! [`NodeSpec::data_attrs`]: crate::schema::NodeSpec::data_attrs

use crate::model::{AttrValue, Attrs};

/// The most app data attributes one node keeps. Enough for an app's own ids
/// and flags; a page that stamps dozens (framework scoping attributes, lazy
/// loaders) is not what the document should store or every collaborator sync.
pub const MAX_DATA_ATTRS: usize = 32;

/// The longest value, in UTF-8 bytes, of an app data attribute a node keeps.
/// An id or a short reference fits; a base64 `data-src` placeholder does not.
pub const MAX_DATA_ATTR_VALUE: usize = 1024;

/// The longest name, in bytes, of an app data attribute (`data-` included).
pub const MAX_DATA_ATTR_NAME: usize = 64;

/// Name prefixes rinch reserves: the editor view's own markers (`data-pm-*`),
/// anything rinch names after itself (`data-rinch-*`), and the shell's
/// families of state attributes — the read-only text selection
/// (`data-text-sel*`), an input's caret and selection (`data-cursor-*`,
/// `data-selection-*`) and the text context menu's rows (`data-tcm-*`).
pub const RESERVED_DATA_ATTR_PREFIXES: &[&str] = &[
    "data-pm-",
    "data-rinch-",
    "data-text-sel",
    "data-cursor-",
    "data-selection-",
    "data-tcm-",
];

/// Whole names rinch reserves:
///
/// - the event attributes the runtime dispatches on (`data-rid`, and each
///   `data-on…` name `rsx!` writes — the exact names, so `data-one` or
///   `data-online` stay an app's);
/// - what the runtime reads on any element: focus and pointer handling
///   (`data-nofocus`, `data-disabled`, `data-trap-focus`, `data-backdrop`,
///   `data-scroll-lock-exempt`, `data-drag-window`), compositing holes
///   (`data-viewport`, `data-viewport-ready`, `data-render-surface`,
///   `data-video-player`), a press's block (`data-block-index`);
/// - state the shell writes on elements (`data-focused`, `data-preedit`, the
///   native `<select>` popup's `data-nsel-opt` / `data-selected` /
///   `data-highlighted`), and `rsx!`'s and components' runtime markers
///   (`data-fragment`, `data-theme-provider`, `data-rid-reactive`,
///   `data-user-rid`);
/// - the task-list markup the HTML reader reads on `<ul>` / `<li>` (TipTap's
///   `data-type` / `data-checked`).
pub const RESERVED_DATA_ATTRS: &[&str] = &[
    "data-rid",
    "data-oninput",
    "data-onchange",
    "data-onscroll",
    "data-onsubmit",
    "data-onfiledrop",
    "data-onfiledragenter",
    "data-onfiledragleave",
    "data-ondragstart",
    "data-ondragmove",
    "data-ondragend",
    "data-ondrop",
    "data-ondragenter",
    "data-ondragover",
    "data-ondragleave",
    "data-oncontextmenu",
    "data-onenter",
    "data-onleave",
    "data-onmousedown",
    "data-onmouseup",
    "data-onmousemove",
    "data-nofocus",
    "data-disabled",
    "data-trap-focus",
    "data-backdrop",
    "data-scroll-lock-exempt",
    "data-drag-window",
    "data-viewport",
    "data-viewport-ready",
    "data-render-surface",
    "data-video-player",
    "data-block-index",
    "data-focused",
    "data-preedit",
    "data-nsel-opt",
    "data-selected",
    "data-highlighted",
    "data-fragment",
    "data-theme-provider",
    "data-rid-reactive",
    "data-user-rid",
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
/// custom data attribute name of at most [`MAX_DATA_ATTR_NAME`] bytes that
/// rinch does not reserve.
pub fn is_app_data_attr(name: &str) -> bool {
    name.len() <= MAX_DATA_ATTR_NAME && is_data_attr_name(name) && !is_reserved_data_attr(name)
}

/// The app data attributes of `pairs` a node keeps, in the order given: each
/// [`is_app_data_attr`] name the **first** time it appears (HTML keeps the
/// first of two attributes with one name), with a value of at most
/// [`MAX_DATA_ATTR_VALUE`] bytes, at most [`MAX_DATA_ATTRS`] of them. Anything
/// else is dropped whole.
pub fn kept_data_attrs<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let mut seen: Vec<&'a str> = Vec::new();
    pairs
        .into_iter()
        .filter(move |(name, value)| {
            if value.len() > MAX_DATA_ATTR_VALUE || !is_app_data_attr(name) || seen.contains(name) {
                return false;
            }
            seen.push(name);
            true
        })
        .take(MAX_DATA_ATTRS)
}

/// The app data attributes in `attrs` a node keeps ([`kept_data_attrs`] over
/// its string-valued attributes, in name order): what an opted-in node carries
/// out to HTML and to its host element.
pub fn app_data_attrs(attrs: &Attrs) -> impl Iterator<Item = (&str, &str)> {
    kept_data_attrs(attrs.iter().filter_map(|(k, v)| match v {
        AttrValue::Str(s) => Some((k, &**s)),
        _ => None,
    }))
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

    /// The bounds drop whole attributes, the first ones are kept, and a
    /// repeated name keeps its first value.
    #[test]
    fn kept_data_attrs_bounds_count_value_and_name() {
        let names: Vec<String> = (0..40).map(|i| format!("data-k{i:02}")).collect();
        let kept: Vec<&str> = kept_data_attrs(names.iter().map(|n| (n.as_str(), "v")))
            .map(|(n, _)| n)
            .collect();
        assert_eq!(kept.len(), MAX_DATA_ATTRS);
        assert_eq!(kept[0], "data-k00");
        assert_eq!(kept[MAX_DATA_ATTRS - 1], "data-k31");

        let at = "x".repeat(MAX_DATA_ATTR_VALUE);
        let past = "x".repeat(MAX_DATA_ATTR_VALUE + 1);
        let long_name = format!("data-{}", "n".repeat(MAX_DATA_ATTR_NAME - 4));
        let kept: Vec<(&str, &str)> = kept_data_attrs([
            ("data-a", "first"),
            ("data-a", "second"),
            ("data-at", at.as_str()),
            ("data-past", past.as_str()),
            (long_name.as_str(), "v"),
            ("data-rid", "1"),
        ])
        .collect();
        assert_eq!(kept, [("data-a", "first"), ("data-at", at.as_str())]);
        // A name of exactly the bound is kept.
        let at_name = format!("data-{}", "n".repeat(MAX_DATA_ATTR_NAME - 5));
        assert!(is_app_data_attr(&at_name));
        assert!(!is_app_data_attr(&long_name));
    }
}
