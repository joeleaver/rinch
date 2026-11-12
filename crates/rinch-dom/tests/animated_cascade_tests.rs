//! An animated `color` or `font-size` goes through the cascade (#777, #787),
//! and a CSS animation's clock starts at the first frame that renders its
//! element (#768).
//!
//! A transition or animation used to write its sample into the node's own
//! `ComputedStyle` only. Stylo had already resolved the node's `em` lengths
//! and every descendant's inherited `color` / `font-size` against the
//! *unanimated* value, so a `transition: color` on a button whose label is a
//! child span snapped the label to the end colour, and an animated
//! `font-size` moved neither the node's own `em` width nor its children's
//! text.
//!
//! # The oracle
//!
//! Chrome 157, headless (`--dump-dom`): every animation paused, then seeked
//! with `currentTime`, then `getComputedStyle` read.
//!
//! | case | at 500ms of 1000ms linear |
//! |---|---|
//! | `.p { color: rgb(0,0,0); transition: color }` → `rgb(200,100,0)`, child `<span>` | parent and span `rgb(100, 50, 0)` (250ms: `rgb(50, 25, 0)`) |
//! | `font-size: 16px; width: 6em; animation: f` with `from { font-size: 32px } to { font-size: 48px }` | `40px`, width `240px`; child span `40px`; child `width: 2em` → `80px` |
//! | `font-size: 16px; width: 6em; transition: font-size` → `32px` | `24px`, width `144px`; child span `24px`; child `width: 2em` → `48px` |
//!
//! #768 is the Web Animations rule rather than a measurement: a CSS
//! animation is *pending* until the first frame its element is rendered in,
//! and its start time is that frame's, so the first frame shows it at t = 0.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const T0: f64 = 1_000_000.0;

type Rgb = (u8, u8, u8);

fn colour_of(doc: &RinchDocument, id: NodeId) -> Rgb {
    let c = doc
        .tree
        .get(id.0)
        .unwrap()
        .computed_style
        .color
        .expect("a cascaded element has a colour")
        .to_rgba8();
    (c.r, c.g, c.b)
}

fn font_size_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.font_size
}

fn width_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

/// One frame as the shell runs it: the ticks, then the layout.
fn frame(doc: &mut RinchDocument, at_ms: f64) {
    doc.set_animation_clock_ms(Some(at_ms));
    doc.tick_transitions();
    doc.tick_animations();
    doc.resolve_layout(800.0, 600.0);
}

fn new_doc(css: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.set_animation_clock_ms(Some(T0));
    doc.load_css(&format!(
        "html, body {{ margin: 0; }} body {{ font-size: 16px; line-height: 20px; }} {css}"
    ));
    doc
}

fn child(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str, text: &str) -> NodeId {
    let el = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(el, "class", class);
    }
    let t = doc.create_text(text);
    doc.append_child(el, t);
    doc.append_child(parent, el);
    el
}

// ---------------------------------------------------------------------------
// #777 — a colour transition reaches the children that inherit it
// ---------------------------------------------------------------------------

fn colour_transition() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = new_doc(
        ".p { color: rgb(0, 0, 0); transition: color 1000ms linear; }
         .p.on { color: rgb(200, 100, 0); }",
    );
    let body = doc.body();
    let p = doc.create_element("button");
    doc.set_attribute(p, "class", "p");
    doc.append_child(body, p);
    let label = child(&mut doc, p, "span", "", "label");
    doc.resolve_layout(800.0, 600.0);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        colour_of(&doc, label),
        (0, 0, 0),
        "positive control: start colour"
    );

    doc.set_attribute(p, "class", "p on");
    frame(&mut doc, T0);
    assert!(
        doc.tree.active_transitions.contains_key(&p.0),
        "positive control: the class change starts a colour transition"
    );
    (doc, p, label)
}

/// The frame the transition starts in already shows the child at the start
/// colour — not one frame of the end colour first.
#[test]
fn a_child_inherits_the_start_colour_on_the_frame_a_transition_starts() {
    let (doc, p, label) = colour_transition();
    assert_eq!(colour_of(&doc, p), (0, 0, 0));
    assert_eq!(colour_of(&doc, label), (0, 0, 0));
}

#[test]
fn a_child_inherits_the_animated_colour_of_a_transition() {
    let (mut doc, p, label) = colour_transition();
    frame(&mut doc, T0 + 250.0);
    assert_eq!(colour_of(&doc, p), (50, 25, 0));
    assert_eq!(
        colour_of(&doc, label),
        (50, 25, 0),
        "Chrome: rgb(50, 25, 0)"
    );
    frame(&mut doc, T0 + 500.0);
    assert_eq!(colour_of(&doc, p), (100, 50, 0));
    assert_eq!(
        colour_of(&doc, label),
        (100, 50, 0),
        "Chrome: rgb(100, 50, 0)"
    );
}

/// The end of the run hands the child the cascaded end colour, and nothing
/// is left overriding the cascade.
#[test]
fn a_child_lands_on_the_end_colour_when_the_transition_ends() {
    let (mut doc, p, label) = colour_transition();
    frame(&mut doc, T0 + 500.0);
    frame(&mut doc, T0 + 1500.0);
    assert!(
        !doc.tree.active_transitions.contains_key(&p.0),
        "{:?}",
        doc.tree.active_transitions.get(&p.0)
    );
    assert_eq!(colour_of(&doc, p), (200, 100, 0));
    assert_eq!(colour_of(&doc, label), (200, 100, 0));
}

/// A colour *animation* is inherited the same way.
#[test]
fn a_child_inherits_the_animated_colour_of_an_animation() {
    let mut doc = new_doc(
        ".a { color: rgb(0, 0, 0); animation: c 1000ms linear; }
         @keyframes c { from { color: rgb(0, 0, 0); } to { color: rgb(200, 100, 0); } }",
    );
    let body = doc.body();
    let a = doc.create_element("div");
    doc.set_attribute(a, "class", "a");
    doc.append_child(body, a);
    let label = child(&mut doc, a, "span", "", "label");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.active_animations.contains_key(&a.0),
        "positive control"
    );
    frame(&mut doc, T0 + 500.0);
    assert_eq!(colour_of(&doc, a), (100, 50, 0));
    assert_eq!(colour_of(&doc, label), (100, 50, 0));
}

// ---------------------------------------------------------------------------
// #787 — an animated font-size reaches the node's own `em` and its children
// ---------------------------------------------------------------------------

fn font_size_animation() -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = new_doc(
        ".tb { font-size: 16px; line-height: 1; width: 6em; animation: f 1000ms linear; }
         .w2 { display: block; width: 2em; }
         @keyframes f { from { font-size: 32px; } to { font-size: 48px; } }",
    );
    let body = doc.body();
    let tb = doc.create_element("div");
    doc.set_attribute(tb, "class", "tb");
    doc.append_child(body, tb);
    let span = child(&mut doc, tb, "span", "", "x");
    let w2 = child(&mut doc, tb, "span", "w2", "y");
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.active_animations.contains_key(&tb.0),
        "positive control"
    );
    (doc, tb, span, w2)
}

#[test]
fn an_animated_font_size_resolves_the_nodes_own_em_width() {
    let (mut doc, tb, _, _) = font_size_animation();
    // The first frame: the `from` stop, 32px, and 6em of it.
    assert_eq!(font_size_of(&doc, tb), 32.0);
    assert_eq!(width_of(&doc, tb), 192.0, "6em of the animated 32px");
    frame(&mut doc, T0 + 500.0);
    assert_eq!(font_size_of(&doc, tb), 40.0);
    assert_eq!(width_of(&doc, tb), 240.0, "Chrome: 240px");
}

#[test]
fn an_animated_font_size_is_inherited() {
    let (mut doc, _, span, w2) = font_size_animation();
    frame(&mut doc, T0 + 500.0);
    assert_eq!(font_size_of(&doc, span), 40.0, "Chrome: 40px");
    assert_eq!(font_size_of(&doc, w2), 40.0);
    assert_eq!(width_of(&doc, w2), 80.0, "Chrome: 2em of 40px = 80px");
}

#[test]
fn a_font_size_transition_reaches_em_and_children() {
    let mut doc = new_doc(
        ".ft { font-size: 16px; line-height: 1; width: 6em; transition: font-size 1000ms linear; }
         .ft.on { font-size: 32px; }
         .w2 { display: block; width: 2em; }",
    );
    let body = doc.body();
    let ft = doc.create_element("div");
    doc.set_attribute(ft, "class", "ft");
    doc.append_child(body, ft);
    let span = child(&mut doc, ft, "span", "", "x");
    let w2 = child(&mut doc, ft, "span", "w2", "y");
    doc.resolve_layout(800.0, 600.0);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);

    doc.set_attribute(ft, "class", "ft on");
    frame(&mut doc, T0);
    assert!(
        doc.tree.active_transitions.contains_key(&ft.0),
        "positive control"
    );
    // The start frame keeps the before-change size everywhere.
    assert_eq!(width_of(&doc, ft), 96.0);
    assert_eq!(font_size_of(&doc, span), 16.0);

    frame(&mut doc, T0 + 500.0);
    assert_eq!(font_size_of(&doc, ft), 24.0);
    assert_eq!(width_of(&doc, ft), 144.0, "Chrome: 144px");
    assert_eq!(font_size_of(&doc, span), 24.0, "Chrome: 24px");
    assert_eq!(width_of(&doc, w2), 48.0, "Chrome: 48px");

    frame(&mut doc, T0 + 1500.0);
    assert_eq!(width_of(&doc, ft), 192.0);
    assert_eq!(font_size_of(&doc, span), 32.0);
    assert_eq!(width_of(&doc, w2), 64.0);
}

// ---------------------------------------------------------------------------
// #768 — the clock starts at the first frame that renders the element
// ---------------------------------------------------------------------------

#[test]
fn an_animation_starts_at_the_first_frame_that_renders_its_element() {
    let mut doc = new_doc(
        ".g { height: 10px; animation: g 1000ms linear; }
         @keyframes g { from { width: 100px; } to { width: 200px; } }",
    );
    doc.resolve_layout(800.0, 600.0);
    let body = doc.body();
    let g = doc.create_element("div");
    doc.set_attribute(g, "class", "g");
    doc.append_child(body, g);
    // The component goes on building for 300ms before the frame that shows
    // the element.
    doc.set_animation_clock_ms(Some(T0 + 300.0));
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.active_animations.contains_key(&g.0),
        "positive control"
    );
    assert_eq!(width_of(&doc, g), 100.0, "the first frame shows t = 0");

    frame(&mut doc, T0 + 300.0 + 500.0);
    assert_eq!(width_of(&doc, g), 150.0, "500ms after the first frame");
}
