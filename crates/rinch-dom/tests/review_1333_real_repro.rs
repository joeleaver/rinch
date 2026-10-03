//! Regression pin added during review of PR #1333 (#653).
//!
//! The PR's first attempt fixed the symptom in `read_layout_results` (skip
//! the generic Taffy read and leave `node.layout` untouched for a nested
//! IFC-member text node) rather than the cause. That guard is only sound if
//! the value it preserves can never have been wrong — and it provably can:
//! whenever a nested member's *previous* box came from a pass where it was
//! briefly a *direct* IFC member (its wrapping span was, for that pass, its
//! own IFC root), the guard freezes that now-stale value forever once the
//! span goes back to being a flowed inline member. No hand-manipulation of
//! the Taffy tree is needed to reach this — it is an ordinary
//! `set_attribute("style", ..)` display toggle:
//!
//! ```text
//! after first layout:        text.layout = 0x0,     ifc_root = Some(container)
//! after display:block:       text.layout = 400x20,  ifc_root = Some(span)      // span is its own IFC root now; text, its direct child, legitimately gets this box
//! after flip back to inline: text.layout = 400x20,  ifc_root = Some(container) // STALE under the read-side guard — pre-fix and the root fix both give 0x0 here
//! ```
//!
//! Fixed at the root instead (`mark_inline_descendants` detaches a member
//! from its *actual current* Taffy parent, via `taffy.parent`, rather than
//! only when it is a direct child of the outermost root), this passes
//! because the text node's Taffy leaf is never left attached to anything
//! reachable in the first place — there is no stale value for any later
//! pass to pick up.

use rinch_core::dom::DomDocument;
use rinch_dom::{LayoutResult, RinchDocument};

#[test]
fn real_repro_display_toggle_reveals_stale_nested_text_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    doc.append_child(body, container);

    let span = doc.create_element("span");
    doc.append_child(container, span);
    let text = doc.create_text("Extra small");
    doc.append_child(span, text);

    doc.resolve_layout(800.0, 600.0);
    println!(
        "after first layout: text.layout = {:?}, ifc_root = {:?}",
        doc.tree.get(text.0).unwrap().layout,
        doc.tree.get(text.0).unwrap().ifc_root
    );
    assert_eq!(
        doc.tree.get(text.0).unwrap().layout,
        LayoutResult::default()
    );

    // Flip span to block: it becomes its own IFC root, and `text` — now its
    // DIRECT child — correctly gets a real, non-zero box.
    doc.set_attribute(span, "style", "display: block");
    doc.resolve_layout(800.0, 600.0);
    println!(
        "after display:block: span.layout = {:?}, text.layout = {:?}, ifc_root = {:?}",
        doc.tree.get(span.0).unwrap().layout,
        doc.tree.get(text.0).unwrap().layout,
        doc.tree.get(text.0).unwrap().ifc_root
    );

    // Flip back to inline: `text` is nested again under `container`'s IFC.
    doc.set_attribute(span, "style", "display: inline");
    doc.resolve_layout(800.0, 600.0);
    println!(
        "after flip back to inline: text.layout = {:?}, ifc_root = {:?}",
        doc.tree.get(text.0).unwrap().layout,
        doc.tree.get(text.0).unwrap().ifc_root
    );
}

#[test]
fn real_repro_assert_correct_behavior() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    doc.append_child(body, container);
    let span = doc.create_element("span");
    doc.append_child(container, span);
    let text = doc.create_text("Extra small");
    doc.append_child(span, text);
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(span, "style", "display: block");
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(span, "style", "display: inline");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(text.0).unwrap().layout,
        LayoutResult::default(),
        "text nested in a span that went block-then-inline-again must not keep a stale box"
    );
}
