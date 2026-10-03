//! #1297: registering a font **after** the first layout invalidates no text
//! measurement — a box already measured against the fallback face keeps
//! that face's metrics, however many layouts run after the registration.
//!
//! `RinchApp::register_font_data` / a late `register_app_font` call (the
//! wasm/embed front door and any `.fonts`/`App::fonts` registration that
//! arrives after the first layout, see CLAUDE.md "App-bundled fonts") only
//! ever bumped `RinchDocument::font_generation` and re-sized the document's
//! *form controls* (`note_fonts_registered`, #1177). General text —
//! `auto`/`max-content` IFC roots, and the `fit-content`/`stretch`
//! inline-block cache #1281 added — kept its fallback-font metrics, because
//! a font registration changes no *computed style* (`font-family` is the
//! same string before and after) and so the cascade's own staleness gate
//! never fires for it, unlike a theme restyle which re-cascades everything.
//!
//! Each test below lays a document out once against a fallback face, then
//! registers the real face and lays out again, and compares the result with
//! a **fresh** document that had the real face registered from the start —
//! the fixed point (`fresh == before`, i.e. the registration did nothing) is
//! exactly the bug, so every assertion samples off it: `before != fresh`
//! (the fallback really did produce a different width) and
//! `after == fresh` (the registration caught up).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const SPACE_GROTESK: &[u8] = include_bytes!("../assets/fonts/SpaceGrotesk-VariableFont_wght.ttf");

fn register_face(doc: &mut RinchDocument, data: &'static [u8], family: &str) {
    use parley::fontique::{Blob, FontInfoOverride};
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(data)),
        Some(FontInfoOverride {
            family_name: Some(family),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
}

fn width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

const TEXT: &str = "The quick brown fox jumps over";
// Inter and Space Grotesk differ enough at 16px to move a 31-char string's
// max-content width by several px (the review of #1302 measured up to 19px
// at 16px between two genuinely different families); either one standing
// in for "the fallback face" and the other for "the registered face" is
// enough to detect a frozen measurement.
const CSS: &str = "font: 16px/20px LateFace, monospace; padding: 0; border: 0";

/// General text, `display: inline-block` with no explicit width (the
/// `auto`/shrink-to-fit case): an IFC root inside an atomic inline, sized at
/// max-content. Before the fix this box kept the fallback face's width
/// forever — nothing ever cleared its `text_layout` or measure cache, since
/// its content, tag, display mode and signature never changed.
#[test]
fn a_late_registered_face_resizes_an_inline_block_paragraph() {
    let fresh = {
        let mut doc = RinchDocument::new();
        register_face(&mut doc, SPACE_GROTESK, "LateFace");
        let body = doc.body();
        let c = doc.create_element("div");
        doc.set_attribute(c, "style", &format!("{CSS}; display: inline-block"));
        doc.append_child(body, c);
        doc.set_text_content(c, TEXT);
        doc.resolve_layout(800.0, 600.0);
        width(&doc, c)
    };

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", &format!("{CSS}; display: inline-block"));
    doc.append_child(body, c);
    doc.set_text_content(c, TEXT);
    doc.resolve_layout(800.0, 600.0);
    let before = width(&doc, c);
    assert_ne!(
        before, fresh,
        "sized from the monospace fallback before LateFace exists"
    );

    register_face(&mut doc, SPACE_GROTESK, "LateFace");
    doc.note_fonts_registered();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width(&doc, c),
        fresh,
        "now sized from LateFace, like `fresh` — a frozen measurement would \
         still read `before`"
    );
}

/// The `fit-content`/`stretch` keyword width cache #1281 added
/// (`NodeTree::keyword_inline_cb_width`) is keyed only on the containing
/// block's width, which a font registration does not move — so a box sized
/// from one of those keywords was skipped as "already measured at this
/// width" and kept the fallback face's width even though the general IFC
/// fix above would have caught a bare `auto` box.
#[test]
fn a_late_registered_face_resizes_a_fit_content_inline_block() {
    let css = |family_css: &str| {
        format!(
            "font: 16px/20px {family_css}; padding: 0; border: 0; \
             display: inline-block; width: fit-content"
        )
    };

    let fresh = {
        let mut doc = RinchDocument::new();
        register_face(&mut doc, SPACE_GROTESK, "LateFace");
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(container, "style", "width: 700px");
        doc.append_child(body, container);
        let c = doc.create_element("div");
        doc.set_attribute(c, "style", &css("LateFace, monospace"));
        doc.append_child(container, c);
        doc.set_text_content(c, TEXT);
        doc.resolve_layout(800.0, 600.0);
        width(&doc, c)
    };

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 700px");
    doc.append_child(body, container);
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", &css("LateFace, monospace"));
    doc.append_child(container, c);
    doc.set_text_content(c, TEXT);
    doc.resolve_layout(800.0, 600.0);
    let before = width(&doc, c);
    assert_ne!(
        before, fresh,
        "sized from the monospace fallback before LateFace exists"
    );

    register_face(&mut doc, SPACE_GROTESK, "LateFace");
    doc.note_fonts_registered();
    // The keyword-width cache is keyed on the containing block's width
    // (unchanged across this layout) — a second `resolve_layout` at the
    // *same* viewport is exactly the shape that would still pass if the
    // cache, rather than the general invalidation, were doing the work. The
    // fix clears the cache outright, so this still catches it.
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width(&doc, c),
        fresh,
        "now sized from LateFace, like `fresh`"
    );
}

/// A form control (already fixed by #1177/#1302's `font_generation`-keyed
/// caches) keeps working alongside the general-text invalidation above —
/// the two paths must not step on each other.
#[test]
fn a_late_registered_face_still_resizes_a_text_input() {
    let css = "font: 16px/20px LateFace, monospace; padding: 0; border: 0";
    let fresh = {
        let mut doc = RinchDocument::new();
        register_face(&mut doc, INTER, "LateFace");
        let body = doc.body();
        let c = doc.create_element("input");
        doc.set_attribute(c, "type", "text");
        doc.set_attribute(c, "size", "20");
        doc.set_attribute(c, "style", css);
        doc.append_child(body, c);
        doc.resolve_layout(800.0, 600.0);
        width(&doc, c)
    };

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element("input");
    doc.set_attribute(c, "type", "text");
    doc.set_attribute(c, "size", "20");
    doc.set_attribute(c, "style", css);
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    let before = width(&doc, c);
    assert_ne!(
        before, fresh,
        "sized from the monospace fallback before LateFace exists"
    );

    register_face(&mut doc, INTER, "LateFace");
    doc.note_fonts_registered();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, c), fresh, "now sized from LateFace, like `fresh`");
}
