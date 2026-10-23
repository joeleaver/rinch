//! #1866: a `GameViewport` is the base layer. Nothing painted beneath it in
//! paint order shows inside its box, whether or not the box that painted it
//! clips; anything painted after it covers it as usual.
//!
//! Paint clears the viewport's box (its own border-box shape, through its own
//! clip chain and transform) to transparent when it reaches the viewport, and
//! the compositor's layer under the UI shows there. These fixtures paint with
//! the software painter and no active-viewport filter, which is the GPU
//! window's scene (every `data-viewport` clears); the desktop GPU and embed
//! renders of the same rule are the `#[ignore]`d `gpu_game_viewport_base_layer`
//! fixtures in `crates/rinch`.
//!
//! The issue's cases, each read at a pixel inside the viewport (want:
//! transparent) and a pixel of the same ancestor outside it (want: painted):
//! a non-clipping ancestor's background, an earlier sibling under the
//! viewport, a later sibling and a popup over it, the viewport's own
//! `border-radius`, and a viewport moved by a transform inside a clipping box
//! (#1386: the hole used to be cut at the untransformed rect).

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 400;
const H: u32 = 300;

fn painted(html: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    painter
}

fn rgba(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    let px = p.pixels();
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

const GREEN: [u8; 4] = [0, 128, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// The issue's headline: an ancestor that paints a background and does not
/// clip. Its background used to be cut only when the box clipped (#974), so
/// here it covered the game. Two such ancestors, so a clear that reached one
/// level up would still fail.
#[test]
fn a_non_clipping_ancestors_background_is_cleared_under_the_viewport() {
    let p = painted(
        "<div style=\"background: rgb(255,0,0); padding: 10px\">\
         <div style=\"background: rgb(0,128,0); padding: 20px 30px\">\
         <div data-viewport=\"base-a\" style=\"width: 100px; height: 60px\"></div>\
         </div></div>",
    );
    // The viewport is at (40, 30)..(140, 90).
    for (x, y) in [(41, 31), (90, 60), (139, 89)] {
        assert_eq!(rgba(&p, x, y), CLEAR, "({x}, {y}) inside the viewport");
    }
    assert_eq!(rgba(&p, 35, 60), GREEN, "the ancestor beside the viewport");
    assert_eq!(rgba(&p, 5, 60), RED, "the grandparent beside it");
    assert_eq!(rgba(&p, 145, 60), GREEN, "right of the viewport");
}

/// Content painted beneath the viewport in paint order is cleared too, not
/// only ancestor backgrounds: an earlier positioned sibling under it.
#[test]
fn an_earlier_sibling_under_the_viewport_is_cleared() {
    let p = painted(
        "<div style=\"position: relative; width: 300px; height: 200px\">\
         <div style=\"position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
         background: rgb(255,0,0)\"></div>\
         <div data-viewport=\"base-b\" style=\"position: absolute; left: 50px; top: 50px; \
         width: 100px; height: 100px\"></div>\
         </div>",
    );
    assert_eq!(rgba(&p, 100, 75), CLEAR, "the sibling under the viewport");
    assert_eq!(rgba(&p, 25, 75), RED, "the sibling beside it");
    assert_eq!(rgba(&p, 175, 25), RED, "the sibling above it");
}

/// What is painted after the viewport still covers it: a later sibling, and
/// a popup (a `z-index` above it, earlier in the DOM). A clear run at the end
/// of paint, or for the whole layer, would erase both.
#[test]
fn a_later_sibling_and_a_popup_still_cover_the_viewport() {
    let p = painted(
        "<div style=\"position: relative; width: 300px; height: 200px; \
         background: rgb(0,128,0)\">\
         <div style=\"position: absolute; z-index: 5; left: 120px; top: 20px; width: 40px; \
         height: 40px; background: rgb(0,0,255)\"></div>\
         <div data-viewport=\"base-c\" style=\"position: absolute; left: 20px; top: 20px; \
         width: 200px; height: 120px\"></div>\
         <div style=\"position: absolute; left: 40px; top: 100px; width: 40px; height: 20px; \
         background: rgb(255,0,0)\"></div>\
         </div>",
    );
    assert_eq!(
        rgba(&p, 60, 110),
        RED,
        "the later sibling over the viewport"
    );
    assert_eq!(rgba(&p, 140, 40), BLUE, "the popup over the viewport");
    assert_eq!(rgba(&p, 60, 60), CLEAR, "the viewport elsewhere");
    assert_eq!(rgba(&p, 250, 60), GREEN, "the container beside it");
}

/// The clear is the viewport's own shape: a `border-radius` on it leaves the
/// ancestor painted in its corners. Inside a clipping box, where the old hole
/// was already cut, so the corner is what this pins: the old hole was the
/// viewport's square rect and erased the corner.
#[test]
fn the_viewports_own_border_radius_is_respected() {
    let p = painted(
        "<div style=\"position: absolute; left: 50px; top: 50px; width: 200px; height: 150px; \
         overflow: hidden; background: rgb(0,128,0)\">\
         <div data-viewport=\"base-d\" style=\"margin: 25px; width: 150px; height: 100px; \
         border-radius: 40px\"></div>\
         </div>",
    );
    // The viewport is at (75, 75)..(225, 175), corners of radius 40.
    assert_eq!(rgba(&p, 77, 77), GREEN, "outside the top-left arc");
    assert_eq!(rgba(&p, 223, 173), GREEN, "outside the bottom-right arc");
    assert_eq!(rgba(&p, 100, 100), CLEAR, "inside the top-left arc");
    assert_eq!(rgba(&p, 150, 125), CLEAR, "the middle");
    assert_eq!(rgba(&p, 76, 125), CLEAR, "the left edge, between the arcs");
}

/// The clear goes through the viewport's own transform (#1386): a viewport
/// moved 30px right inside a clipping, painted box is cleared where it is
/// drawn, and the strip it moved away from keeps the background. The old hole
/// was cut at the untransformed rect.
#[test]
fn the_clear_follows_the_viewports_transform() {
    let p = painted(
        "<div style=\"position: absolute; left: 50px; top: 50px; width: 250px; height: 150px; \
         overflow: hidden; background: rgb(0,128,0)\">\
         <div data-viewport=\"base-e\" style=\"width: 100px; height: 100px; \
         transform: translate(30px, 0)\"></div>\
         </div>",
    );
    // Untransformed (50, 50)..(150, 150); drawn at (80, 50)..(180, 150).
    assert_eq!(rgba(&p, 65, 100), GREEN, "the strip it moved away from");
    assert_eq!(rgba(&p, 165, 100), CLEAR, "the part it moved into");
    assert_eq!(rgba(&p, 120, 100), CLEAR, "the middle");
}

/// The clear goes through the viewport's clip chain (#1386): a viewport
/// larger than the scroller it is in clears nothing outside the scroller, so
/// the page's background beside the scroller stays.
#[test]
fn the_clear_is_clipped_by_the_viewports_clip_chain() {
    let p = painted(
        "<div style=\"position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
         background: rgb(255,0,0)\">\
         <div style=\"position: absolute; left: 50px; top: 50px; width: 150px; height: 100px; \
         overflow: hidden; background: rgb(0,128,0)\">\
         <div data-viewport=\"base-f\" style=\"width: 300px; height: 200px\"></div>\
         </div></div>",
    );
    assert_eq!(rgba(&p, 100, 100), CLEAR, "inside the scroller");
    assert_eq!(rgba(&p, 250, 100), RED, "right of the scroller");
    assert_eq!(rgba(&p, 100, 200), RED, "below the scroller");
}

/// A `data-viewport` that is not ready (#186) clears nothing: its node
/// paints as a plain element.
#[test]
fn a_viewport_that_is_not_ready_clears_nothing() {
    let p = painted(
        "<div style=\"background: rgb(0,128,0); padding: 20px\">\
         <div data-viewport=\"base-g\" data-viewport-ready=\"false\" \
         style=\"width: 100px; height: 60px\"></div></div>",
    );
    assert_eq!(rgba(&p, 70, 50), GREEN);
}

/// A `visibility: hidden` viewport (one in a closed `Drawer`) clears nothing:
/// a hidden box draws none of its own visuals. The old hole was cut through
/// a clipping ancestor regardless (#854).
#[test]
fn a_hidden_viewport_clears_nothing() {
    let p = painted(
        "<div style=\"background: rgb(0,128,0); padding: 20px; overflow: hidden\">\
         <div data-viewport=\"base-h\" style=\"width: 100px; height: 60px; \
         visibility: hidden\"></div></div>",
    );
    assert_eq!(rgba(&p, 70, 50), GREEN);
}

/// The debug-build warning where the rule cannot hold: an ancestor with a
/// group effect (here `opacity: 0.5`) composites the cleared group over what
/// is beneath it. It names the viewport and the ancestor, fires once per
/// viewport, and does not fire for a plain ancestor.
#[cfg(debug_assertions)]
#[test]
fn a_group_effect_ancestor_warns_once_and_a_plain_one_does_not() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    let plain = "<div id=\"plain\" style=\"background: rgb(0,128,0); padding: 20px\">\
         <div data-viewport=\"warn-plain\" style=\"width: 100px; height: 60px\"></div></div>";
    painted(plain);
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new(),
        "a plain ancestor"
    );

    let faded = "<div id=\"hud\" style=\"opacity: 0.5; background: rgb(0,128,0); padding: 20px\">\
         <div><div data-viewport=\"warn-faded\" style=\"width: 100px; height: 60px\"></div></div>\
         </div>";
    painted(faded);
    let warnings = rinch_dom::paint::take_viewport_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("\"warn-faded\"")
            && warnings[0].contains("div#hud")
            && warnings[0].contains("opacity"),
        "{}",
        warnings[0]
    );
    painted(faded);
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new(),
        "once per viewport"
    );
}

/// The other two group effects rinch paints, and the viewport's own opacity.
#[cfg(debug_assertions)]
#[test]
fn filter_mask_and_own_opacity_warn() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    for (name, outer, own, what) in [
        ("warn-filter", "filter: brightness(0.5)", "", "filter"),
        (
            "warn-mask",
            "mask-image: linear-gradient(#000, transparent)",
            "",
            "mask-image",
        ),
        ("warn-own", "", "opacity: 0.8", "opacity"),
    ] {
        painted(&format!(
            "<div class=\"panel\" style=\"{outer}; width: 200px; height: 100px\">\
             <div data-viewport=\"{name}\" style=\"width: 100px; height: 60px; {own}\"></div>\
             </div>"
        ));
        let warnings = rinch_dom::paint::take_viewport_warnings();
        assert_eq!(warnings.len(), 1, "{name}: {warnings:?}");
        assert!(
            warnings[0].contains(&format!("\"{name}\"")) && warnings[0].contains(what),
            "{name}: {}",
            warnings[0]
        );
    }
}

/// A transform other than a translation on the viewport or above it: the
/// clear is rotated, the compositor's layer is not.
#[cfg(debug_assertions)]
#[test]
fn a_rotated_ancestor_warns_and_a_translated_one_does_not() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    painted(
        "<div style=\"transform: translate(10px, 5px)\">\
         <div data-viewport=\"warn-moved\" style=\"width: 100px; height: 60px\"></div></div>",
    );
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new(),
        "a translation"
    );
    painted(
        "<div class=\"tilt\" style=\"transform: rotate(10deg)\">\
         <div data-viewport=\"warn-rotated\" style=\"width: 100px; height: 60px\"></div></div>",
    );
    let warnings = rinch_dom::paint::take_viewport_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("\"warn-rotated\"")
            && warnings[0].contains("div.tilt")
            && warnings[0].contains("transform"),
        "{}",
        warnings[0]
    );
}
