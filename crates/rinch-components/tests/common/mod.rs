//! Helpers shared by the `rinch-components` integration tests (#284).
//!
//! Every file under `tests/` is its own crate and pulls this in with
//! `mod common;`, so a file that uses only some of these would warn about the
//! rest — hence the module-wide `dead_code` allowance.
//!
//! Each helper here was a verbatim (or behaviour-identical) copy in two or
//! more test files before it moved. Anything that differed between copies in
//! more than spelling stayed with its file.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_utils::parse_color;
use rinch_core::Signal;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{ClickContext, EventHandlerId, dispatch_input_event, set_click_context};
use rinch_core::reactive::Effect;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

// ------------------------------------------------------------- tree walks

/// Whether `node`'s `class` attribute lists `class` as a whole word.
pub fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

/// The first node in `node`'s subtree (itself included, document order)
/// carrying `class`.
pub fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if has_class(node, class) {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// Every node in `node`'s subtree (itself included, document order) carrying
/// `class`, appended to `out`.
pub fn collect_by_class(node: &NodeHandle, class: &str, out: &mut Vec<NodeHandle>) {
    if has_class(node, class) {
        out.push(node.clone());
    }
    for child in node.children() {
        collect_by_class(&child, class, out);
    }
}

/// The handler id the first `class` element under `root` carries in `attr`
/// (`data-rid`, `data-oninput`, `data-onchange`, ...).
pub fn handler(root: &NodeHandle, class: &str, attr: &str) -> EventHandlerId {
    let node = find_by_class(root, class).expect("element exists");
    EventHandlerId(
        node.get_attribute(attr)
            .expect("element carries a handler id")
            .parse()
            .expect("handler id is numeric"),
    )
}

// ------------------------------------------------------------ icon glyphs

/// Every `d` attribute in `node`'s subtree, in document order.
///
/// This is what tells one rendered Tabler glyph from another: the two icons in
/// a parent-default/child-wins pair are the same `<svg>` box with different
/// path data, so nothing shallower discriminates them.
pub fn glyph(node: &NodeHandle) -> Vec<String> {
    fn walk(node: &NodeHandle, out: &mut Vec<String>) {
        if let Some(d) = node.get_attribute("d") {
            out.push(d);
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

/// The glyph a given icon renders to, rendered on its own for comparison.
pub fn glyph_of(icon: TablerIcon) -> Vec<String> {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let paths = glyph(&render_tabler_icon(
        &mut scope,
        icon,
        TablerIconStyle::Outline,
    ));
    assert!(
        !paths.is_empty(),
        "{icon:?} renders no path data, so it cannot stand for itself in a \
         comparison — pick another icon"
    );
    paths
}

// ------------------------------------------------------------ stylesheets

/// Every component's CSS, as the runtime loads it.
pub fn sheet() -> String {
    rinch_components::styles::generate_all_component_styles()
}

/// Strip `/* ... */` comments before parsing with [`declarations_for`].
///
/// Load-bearing, and verified so by the suites that call it
/// (`color_input_props`, `color_input_dismiss_465`,
/// `hidden_input_containing_block`): with this reduced to the identity their
/// tests fail rather than pass, because the comment introducing a rule is
/// swept into that rule's prelude and stops the selector matching.
pub fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The declarations of every rule in `css` whose selector list contains
/// exactly `selector` (whitespace-normalized), each whitespace-normalized and
/// lowercased.
///
/// Selectors are matched as whole comma-separated entries so `.rinch-switch`
/// does not match `.rinch-switch--disabled` or `.rinch-switch__track`.
/// Comments are NOT stripped here: pass [`strip_comments`]' output, or a
/// comment sitting immediately above a rule is swept into that rule's prelude
/// and stops the selector matching.
pub fn declarations_for(css: &str, selector: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let prelude = rest[..open].rsplit('}').next().unwrap_or("").trim();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        let matches = prelude
            .split(',')
            .any(|s| s.split_whitespace().collect::<Vec<_>>().join(" ") == selector);
        if matches {
            found.extend(
                body.split(';')
                    .map(|d| {
                        d.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase()
                    })
                    .filter(|d| !d.is_empty()),
            );
        }
        rest = &rest[open + close + 1..];
    }
    found
}

// ------------------------------------------------------- colour harnesses

/// What a controlled colour consumer does with each emission it receives.
#[derive(Clone, Copy)]
pub enum Echo {
    /// Writes it back to the bound store verbatim — the controlled-input
    /// shape, and the collaborative store #229 was measured in.
    Back,
    /// The controlled idiom that *reads* the store inside the handler:
    /// `if value != store.get() { store.set(value) }`.
    IfChanged,
    /// A *normalizing* store (#262): every emission is written back
    /// re-spelled by a converter that is not rinch's — the same colour in
    /// another notation, rounded by another rule.
    Normalizing(fn(&str) -> String),
    /// A *transforming* controlled handler (#283): every emission is written
    /// back as this fixed colour — `|v| store.set(snap_to_palette(v))` with a
    /// one-colour palette.
    Snap(&'static str),
    /// Never writes anything back.
    Never,
}

impl Echo {
    /// Do what this consumer does with `value`, emitted by a component bound
    /// to `store`.
    pub fn write_back(self, store: Signal<String>, value: String) {
        match self {
            Echo::Back => store.set(value),
            Echo::IfChanged => {
                if value != store.get() {
                    store.set(value)
                }
            }
            Echo::Normalizing(respell) => store.set(respell(&value)),
            Echo::Snap(colour) => store.set(colour.to_string()),
            Echo::Never => {}
        }
    }
}

/// Record every value `store` ever holds — what a peer on the other end of a
/// controlled binding would receive — returning the log and the effect that
/// keeps it.
pub fn record_store(store: Signal<String>) -> (Rc<RefCell<Vec<String>>>, Effect) {
    let published: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let recorded = published.clone();
    let recorder = Effect::new(move || {
        let value = store.get();
        recorded.borrow_mut().push(value);
    });
    (published, recorder)
}

/// A click at (`px`, `py`) of a 200×200 element at the origin.
pub fn click_at(px: f32, py: f32) {
    set_click_context(ClickContext {
        mouse_x: px * 200.0,
        mouse_y: py * 200.0,
        element_x: 0.0,
        element_y: 0.0,
        element_width: 200.0,
        element_height: 200.0,
        ..Default::default()
    });
}

pub fn hue_of(color: &str) -> f64 {
    parse_color(color).expect("a formatted colour parses").h
}

pub fn sat_of(color: &str) -> f64 {
    parse_color(color).expect("a formatted colour parses").s
}

/// The `key`-prefixed percentage in a thumb's style string, e.g.
/// `percent_of(&style, "left: ")`. Click-derived positions carry f32→f64
/// noise ("left: 40.000000596%"), so callers compare within a tolerance.
pub fn percent_of(style: &str, key: &str) -> f64 {
    let start = style.find(key).expect("style carries the key") + key.len();
    let rest = &style[start..];
    let end = rest.find('%').expect("a % terminates the value");
    rest[..end].trim().parse().expect("the value is numeric")
}

/// The style attribute of the first `class` thumb under `root` — where a
/// picker says a degree of freedom currently sits.
pub fn thumb_style(root: &NodeHandle, class: &str) -> String {
    find_by_class(root, class)
        .expect("thumb exists")
        .get_attribute("style")
        .expect("thumb is positioned")
}

/// One keystroke into `field`, desktop-shaped: the runtime mirrors the typed
/// text into the `value` attribute *before* dispatching `oninput` (`handler`)
/// with it. `dispatch_input_event` alone only invokes the handler — it does
/// not model the field's own text.
pub fn type_desktop(field: &NodeHandle, handler: EventHandlerId, text: &str) {
    field.set_attribute("value", text);
    dispatch_input_event(handler, text.to_string());
}
