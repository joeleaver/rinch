//! #2694 — a stylesheet's place in the cascade follows the document, whatever
//! moved it.
//!
//! Two gaps after #2690's incremental `sync_style_element_sheets`:
//!
//! 1. **A moved `<style>`.** `insert_before` / `append_child` of a connected
//!    `<style>` (or of an ancestor of one), with no remove first, never marked
//!    the sheets stale: the element was connected before and after, so its
//!    sheet kept its old cascade position.
//! 2. **A layer reorder.** CSS cascade layers are ordered by where each name
//!    first appears across every sheet. A sheet added before another (or
//!    removed, or edited) can change that order, and Stylo's stylesheet
//!    invalidation collects only the added or removed sheet's own rules
//!    (`@layer x { … }` is bucketed by the rules inside it), so the elements
//!    the *other* sheets' layered rules match kept their old cascade. The
//!    same gap was in `load_css`.
//!
//! What these pin is the cascade the spec gives and a browser gives a freshly
//! styled element (`.scratch` twin, Chrome 157 / Playwright Chromium 145 and
//! Firefox 155): layers `b` then `a`, and a new sheet naming `a` inserted
//! first, gives `b` the win — 10px. (Chrome and Firefox both leave an element
//! that was already styled at 20px in that case, until something else
//! restyles it, while a new `.box` element beside it gets 10px; that is their
//! own invalidation gap, the one this fixes in rinch.)
//!
//! Geometry is box widths set by the rules themselves.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter::*;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

const BASE_CSS: &str = "body { margin: 0; } .box { height: 10px; }";

fn width_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

/// `<div class="box">` in `body`, with 50 `.other` divs after it, laid out.
fn laid_out_box() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    let body = doc.body();
    let bx = doc.create_element("div");
    doc.set_attribute(bx, "class", "box");
    doc.append_child(body, bx);
    for _ in 0..50 {
        let d = doc.create_element("div");
        doc.set_attribute(d, "class", "other");
        doc.append_child(body, d);
    }
    doc.resolve_layout(VW, VH);
    (doc, bx)
}

fn style_element(doc: &mut RinchDocument, css: &str) -> NodeId {
    let style = doc.create_element("style");
    let text = doc.create_text(css);
    doc.append_child(style, text);
    style
}

// ── 1. A moved `<style>` ────────────────────────────────────────────────

/// `insert_before` of a connected `<style>` puts its sheet at its new place in
/// the cascade, and only that sheet is re-invalidated: a later sheet with a
/// `:root` rule, whose order relative to the rest did not change, does not
/// make the move a whole-document restyle.
#[test]
fn a_connected_style_moved_with_insert_before_takes_its_new_place() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let s1 = style_element(&mut doc, ".box { width: 100px; }");
    doc.append_child(body, s1);
    let s2 = style_element(&mut doc, ".box { width: 200px; }");
    doc.append_child(body, s2);
    let root = style_element(&mut doc, ":root { --late: 1px; } .nothing { width: 1px; }");
    doc.append_child(body, root);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "precondition: s2 is later");

    doc.insert_before(body, s2, s1);
    doc.tree.perf.reset();
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(width_of(&doc, bx), 100.0, "#2694: s2 now precedes s1");
    assert_eq!(
        s.get(FullRestyles),
        0,
        "only the moved sheet is re-invalidated: `root`'s `:root` rule did not re-run"
    );
    assert_eq!(
        s.get(ElementsCascaded),
        1,
        "the moved sheet's rule: the box"
    );
}

/// `append_child` of a connected `<style>` moves it to the end: its sheet is
/// last again.
#[test]
fn a_connected_style_moved_with_append_child_takes_its_new_place() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let s1 = style_element(&mut doc, ".box { width: 100px; }");
    doc.append_child(body, s1);
    let s2 = style_element(&mut doc, ".box { width: 200px; }");
    doc.append_child(body, s2);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "precondition");

    doc.append_child(body, s1);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 100.0, "#2694: s1 is now the last");

    // Inside one parent, the other way, through `insert_child` too.
    doc.insert_child(body, s1, 0);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "s1 is first again");
}

/// Moving an *ancestor* of a `<style>` moves its sheet.
#[test]
fn moving_the_parent_of_a_style_moves_its_sheet() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let wrap = doc.create_element("div");
    let s1 = style_element(&mut doc, ".box { width: 100px; }");
    doc.append_child(wrap, s1);
    doc.append_child(body, wrap);
    let s2 = style_element(&mut doc, ".box { width: 200px; }");
    doc.append_child(body, s2);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 200.0, "precondition");

    doc.append_child(body, wrap);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        100.0,
        "#2694: the wrapper's <style> is now after s2"
    );
}

/// A move that leaves every `<style>` in the same tree order changes no
/// cascade and restyles nothing.
#[test]
fn a_move_that_keeps_style_order_restyles_nothing() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let s1 = style_element(&mut doc, ":root { --a: 1px; } .box { width: 100px; }");
    doc.append_child(body, s1);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 100.0, "precondition");

    // The only `<style>`, moved to the front of body: still the only one.
    let first = doc.tree.get(body.0).unwrap().children[0];
    doc.insert_before(body, s1, NodeId(first));
    doc.tree.perf.reset();
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(width_of(&doc, bx), 100.0);
    assert_eq!(s.get(FullRestyles), 0, "the sheet was not re-added");
}

// ── 2. A layer reorder ──────────────────────────────────────────────────

const APP_LAYERS: &str = "@layer b { .box { width: 10px; } } @layer a { .box { width: 20px; } }";

/// The issue's case: layers `b` then `a`, and a new sheet naming `a` inserted
/// first. `a` now precedes `b`, so `b`'s rule wins: 10px (the spec, and a
/// browser's answer for an element styled after the insert).
#[test]
fn a_style_inserted_first_that_names_a_later_layer_reorders_the_layers() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let app = style_element(&mut doc, APP_LAYERS);
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        20.0,
        "precondition: a is the last layer"
    );

    let s0 = style_element(&mut doc, "@layer a { .unrelated { color: red; } }");
    // From before the insert: inserting a `<style>` styles it at once, and
    // that flushes the sheets.
    doc.tree.perf.reset();
    doc.insert_before(body, s0, app);
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(width_of(&doc, bx), 10.0, "#2694: layer order is a, b");
    assert_eq!(s.get(FullRestyles), 0, "the reorder is not a whole restyle");
    assert_eq!(
        s.get(ElementsCascaded),
        2,
        "the new <style> and the box, which the moved layer's rules match: \
         not the 50 `.other`s"
    );

    // Removed again: the order is b, a.
    doc.remove_child(body, s0);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "#2694: s0 removed, a is last");

    // Edited in: an empty `<style>` before `app` gets the text naming `a`.
    let s0 = style_element(&mut doc, "");
    doc.insert_before(body, s0, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "an empty sheet names no layer");
    doc.set_text_content(s0, "@layer a { }");
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "#2694: text edit reorders too");
}

/// `load_css` sheets cascade before every `<style>`: a late one naming `a`
/// reorders the app's layers the same way.
#[test]
fn load_css_naming_a_later_layer_reorders_the_layers() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let app = style_element(&mut doc, APP_LAYERS);
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "precondition");

    doc.load_css("@layer a { .unrelated { color: red; } }");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        10.0,
        "#2694: load_css sheet precedes app"
    );
}

/// Sub-layers are ordered inside their parent, and anonymous layers by where
/// they appear.
#[test]
fn nested_and_anonymous_layers_reorder_too() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let app = style_element(
        &mut doc,
        "@layer outer { @layer b { .box { width: 10px; } } @layer a { .box { width: 20px; } } }",
    );
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "precondition: outer.a is last");
    let s0 = style_element(&mut doc, "@layer outer.a { }");
    doc.insert_before(body, s0, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "#2694: outer.a, outer.b");
    doc.remove_child(body, s0);
    doc.remove_child(body, app);

    let app = style_element(
        &mut doc,
        "@layer { .box { width: 30px; } } @layer a { .box { width: 40px; } }",
    );
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        40.0,
        "precondition: a follows the anonymous layer"
    );
    let s0 = style_element(&mut doc, "@layer a { }");
    doc.insert_before(body, s0, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        width_of(&doc, bx),
        30.0,
        "#2694: a now precedes the anonymous layer"
    );
}

/// A sheet added after the others that names their layers in the same order
/// moves nothing: no extra element is restyled.
#[test]
fn a_sheet_that_keeps_the_layer_order_invalidates_only_itself() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let app = style_element(&mut doc, APP_LAYERS);
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    let late = style_element(&mut doc, "@layer b { .unrelated { color: red; } }");
    doc.tree.perf.reset();
    doc.append_child(body, late);
    doc.resolve_layout(VW, VH);
    let s = doc.tree.perf.end_frame();
    assert_eq!(width_of(&doc, bx), 20.0);
    assert_eq!(
        s.get(ElementsCascaded),
        1,
        "the new <style> alone: nothing reordered, its rule matches nothing"
    );
}

/// Review of #2699, repro 1: a resize flips an `@media` that declares a
/// layer, which reorders the layers with no sheet change; the saved order
/// must follow, or a sheet change that restores the old order is missed.
#[test]
fn a_media_flip_refreshes_the_saved_layer_order() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let s0 = style_element(
        &mut doc,
        "@media (max-width: 500px) { @layer a { .unrelated { color: red; } } }",
    );
    doc.append_child(body, s0);
    let app = style_element(
        &mut doc,
        "@layer b { .box { width: 20px; } } @layer a { .box { width: 10px; } }",
    );
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "precondition, wide: b, a");
    doc.resolve_layout(400.0, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "precondition, narrow: a, b");

    doc.remove_child(body, s0);
    doc.resolve_layout(400.0, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "s0 removed at 400px: b, a again");
}

/// Repro 2: every layer starts inside an `@media` that does not apply, so
/// the saved order is empty; a resize brings them in, and a sheet inserted
/// later that reorders them must still be seen.
#[test]
fn layers_a_resize_brought_in_reorder_on_a_later_insert() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let app = style_element(
        &mut doc,
        "@media (max-width: 500px) { @layer b { .box { width: 20px; } } @layer a { .box { width: 10px; } } }",
    );
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), VW, "precondition: no rule applies");
    doc.resolve_layout(400.0, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "precondition, narrow: b, a");

    let s0 = style_element(&mut doc, "@layer a { }");
    doc.insert_before(body, s0, app);
    doc.resolve_layout(400.0, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "s0 inserted: a, b");
}

/// The same through a device-pixel-ratio change, the other door to a media
/// flip (`set_device_pixel_ratio`).
#[test]
fn a_resolution_flip_refreshes_the_saved_layer_order() {
    let (mut doc, bx) = laid_out_box();
    let body = doc.body();
    let s0 = style_element(
        &mut doc,
        "@media (min-resolution: 2dppx) { @layer a { .unrelated { color: red; } } }",
    );
    doc.append_child(body, s0);
    let app = style_element(
        &mut doc,
        "@layer b { .box { width: 20px; } } @layer a { .box { width: 10px; } }",
    );
    doc.append_child(body, app);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "precondition, 1x: b, a");
    doc.set_device_pixel_ratio(2.0);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 20.0, "precondition, 2x: a, b");

    doc.remove_child(body, s0);
    doc.resolve_layout(VW, VH);
    assert_eq!(width_of(&doc, bx), 10.0, "s0 removed at 2x: b, a again");
}

// ── 3. Random edits, checked against tree order after every frame ──────

/// #2690's review stress, as a test: random `<style>` adds, removes, moves
/// (of a `<style>` and of a wrapper holding some), text changes, identical
/// text, `load_css` sheets naming layers, and resizes across an `@media`
/// some rules sit in (review of #2699: a media flip reorders layers with no
/// sheet change). After every frame the box's width must be the one the
/// connected sheets give in tree order, with layers ordered by first
/// appearance among the rules in effect and unlayered rules last.
#[test]
fn random_style_edits_keep_tree_order_winners() {
    const SEEDS: u64 = 40;
    const FRAMES: usize = 300;
    for seed in 0..SEEDS {
        run_stress(seed, FRAMES);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum It {
    Box,
    Wrap,
    S(usize),
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A sheet's rule: `None` for an empty `<style>`; else its layer (`None`
/// unlayered), its width (`None`: a rule for `.unrelated`, which only
/// declares the layer — its own invalidation does not reach the box, so it
/// does not hide a missed reorder), and its `Media`.
type Rule = Option<(Option<char>, Option<u32>, Media)>;

const NARROW: f32 = 400.0;

/// Where a rule applies: at both widths, or inside an `@media` for one.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Media {
    Any,
    Narrow,
    Wide,
}

impl Media {
    fn random(rng: &mut Rng) -> Self {
        [Media::Any, Media::Narrow, Media::Wide][rng.below(3)]
    }
    fn applies(self, vw: f32) -> bool {
        match self {
            Media::Any => true,
            Media::Narrow => vw <= 500.0,
            Media::Wide => vw > 500.0,
        }
    }
    fn wrap(self, css: String) -> String {
        match self {
            Media::Any => css,
            Media::Narrow => format!("@media (max-width: 500px) {{ {css} }}"),
            Media::Wide => format!("@media (min-width: 501px) {{ {css} }}"),
        }
    }
}

fn css_of(rule: Rule) -> String {
    let Some((layer, w, media)) = rule else {
        return String::new();
    };
    let inner = match w {
        Some(w) => format!(".box {{ width: {w}px; }}"),
        None => ".unrelated { color: red; }".to_string(),
    };
    let css = match layer {
        None => inner,
        Some(l) => format!("@layer {l} {{ {inner} }}"),
    };
    media.wrap(css)
}

struct Stress {
    doc: RinchDocument,
    bx: NodeId,
    wrap: NodeId,
    body_items: Vec<It>,
    wrap_items: Vec<It>,
    styles: Vec<(NodeId, Rule)>,
    /// Layers named by `load_css` sheets, in load order (they precede every
    /// `<style>`), each with its `Media`.
    head_layers: Vec<(char, Media)>,
    next_width: u32,
    /// The viewport width: `VW` or `NARROW`.
    vw: f32,
}

impl Stress {
    fn new() -> Self {
        let mut doc = RinchDocument::new();
        doc.load_css(BASE_CSS);
        let body = doc.body();
        let bx = doc.create_element("div");
        doc.set_attribute(bx, "class", "box");
        doc.append_child(body, bx);
        let wrap = doc.create_element("div");
        doc.append_child(body, wrap);
        doc.resolve_layout(VW, VH);
        Stress {
            doc,
            bx,
            wrap,
            body_items: vec![It::Box, It::Wrap],
            wrap_items: Vec::new(),
            styles: Vec::new(),
            head_layers: Vec::new(),
            next_width: 100,
            vw: VW,
        }
    }

    fn node(&self, it: It) -> NodeId {
        match it {
            It::Box => self.bx,
            It::Wrap => self.wrap,
            It::S(k) => self.styles[k].0,
        }
    }

    fn random_rule(&mut self, rng: &mut Rng) -> Rule {
        if rng.below(6) == 0 {
            return None;
        }
        self.next_width += 1;
        let layer = match rng.below(6) {
            0 => None,
            1 => Some('c'),
            n => Some(['a', 'b'][n % 2]),
        };
        let width = (rng.below(2) != 0).then_some(self.next_width);
        Some((layer, width, Media::random(rng)))
    }

    /// Take `it` out of the model, if it is in it.
    fn unlink(&mut self, it: It) {
        self.body_items.retain(|&x| x != it);
        self.wrap_items.retain(|&x| x != it);
    }

    fn is_connected(&self, it: It) -> bool {
        self.body_items.contains(&it) || self.wrap_items.contains(&it)
    }

    /// Put `it` (a `<style>` or the wrapper) at a random place: `body` or,
    /// for a `<style>`, the wrapper. A move when it is connected.
    fn place(&mut self, it: It, rng: &mut Rng, log: &mut String) {
        self.unlink(it);
        let in_wrap = it != It::Wrap && rng.below(3) == 0;
        let (parent, list) = if in_wrap {
            (self.wrap, &mut self.wrap_items)
        } else {
            (self.doc.body(), &mut self.body_items)
        };
        let at = rng.below(list.len() + 1);
        let reference = list.get(at).copied();
        list.insert(at, it);
        let child = self.node(it);
        match reference {
            Some(r) => {
                let r = self.node(r);
                self.doc.insert_before(parent, child, r);
            }
            None => self.doc.append_child(parent, child),
        }
        log.push_str(&format!(
            " place({it:?} in {} at {at})",
            if in_wrap { "wrap" } else { "body" }
        ));
    }

    fn step(&mut self, rng: &mut Rng, log: &mut String) {
        let n = self.styles.len();
        match rng.below(11) {
            // Add a new `<style>`.
            0 | 1 if n < 8 => {
                let rule = self.random_rule(rng);
                let s = style_element(&mut self.doc, &css_of(rule));
                self.styles.push((s, rule));
                log.push_str(&format!(" new(S({n}) {rule:?})"));
                self.place(It::S(n), rng, log);
            }
            // Remove a connected one.
            2 if n > 0 => {
                let k = rng.below(n);
                if self.is_connected(It::S(k)) {
                    let parent = if self.wrap_items.contains(&It::S(k)) {
                        self.wrap
                    } else {
                        self.doc.body()
                    };
                    self.unlink(It::S(k));
                    let s = self.styles[k].0;
                    self.doc.remove_child(parent, s);
                    log.push_str(&format!(" remove(S({k}))"));
                }
            }
            // Move (or re-insert) one.
            3 | 4 if n > 0 => {
                let k = rng.below(n);
                self.place(It::S(k), rng, log);
            }
            // Edit its text.
            5 if n > 0 => {
                let k = rng.below(n);
                let rule = self.random_rule(rng);
                self.styles[k].1 = rule;
                let s = self.styles[k].0;
                self.doc.set_text_content(s, &css_of(rule));
                log.push_str(&format!(" text(S({k}) {rule:?})"));
            }
            // The same text again.
            6 if n > 0 => {
                let k = rng.below(n);
                let (s, rule) = self.styles[k];
                self.doc.set_text_content(s, &css_of(rule));
                log.push_str(&format!(" same_text(S({k}))"));
            }
            // Move the wrapper, with the `<style>`s inside it.
            7 => self.place(It::Wrap, rng, log),
            // A `load_css` sheet naming a layer, before every `<style>`.
            8 if self.head_layers.len() < 3 => {
                let l = ['a', 'b', 'c'][rng.below(3)];
                let media = Media::random(rng);
                let css = media.wrap(format!("@layer {l} {{ .unrelated {{ color: red; }} }}"));
                self.doc.load_css(&css);
                self.head_layers.push((l, media));
                log.push_str(&format!(" load_css({css})"));
            }
            // Resize across the `@media` boundary.
            9 | 10 => {
                self.vw = if self.vw == VW { NARROW } else { VW };
                log.push_str(&format!(" resize({})", self.vw));
            }
            _ => {}
        }
    }

    /// The box's width the connected sheets give, in tree order.
    fn expected_width(&self) -> f32 {
        let mut rules = Vec::new();
        for &it in &self.body_items {
            match it {
                It::S(k) => rules.extend(self.styles[k].1),
                It::Wrap => {
                    for &inner in &self.wrap_items {
                        if let It::S(k) = inner {
                            rules.extend(self.styles[k].1);
                        }
                    }
                }
                It::Box => {}
            }
        }
        let rules: Vec<(Option<char>, Option<u32>)> = rules
            .into_iter()
            .filter(|&(_, _, m)| m.applies(self.vw))
            .map(|(l, w, _)| (l, w))
            .collect();
        if let Some((_, Some(w))) = rules.iter().rev().find(|(l, w)| l.is_none() && w.is_some()) {
            return *w as f32;
        }
        let mut order: Vec<char> = Vec::new();
        for l in self
            .head_layers
            .iter()
            .filter(|&&(_, m)| m.applies(self.vw))
            .map(|&(l, _)| l)
            .chain(rules.iter().filter_map(|(l, _)| *l))
        {
            if !order.contains(&l) {
                order.push(l);
            }
        }
        // The last layer in order that has a `.box` rule; its last rule.
        for l in order.iter().rev() {
            if let Some((_, Some(w))) = rules
                .iter()
                .rev()
                .find(|(rl, w)| *rl == Some(*l) && w.is_some())
            {
                return *w as f32;
            }
        }
        self.vw
    }
}

fn run_stress(seed: u64, frames: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut st = Stress::new();
    let mut history = String::new();
    for frame in 0..frames {
        let mut log = String::new();
        for _ in 0..=rng.below(3) {
            st.step(&mut rng, &mut log);
        }
        st.doc.resolve_layout(st.vw, VH);
        history.push_str(&format!("\n  frame {frame}:{log}"));
        let got = width_of(&st.doc, st.bx);
        let want = st.expected_width();
        if got != want {
            let tail: Vec<&str> = history.lines().rev().take(12).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            panic!(
                "seed {seed} frame {frame}: box width {got}, tree order gives {want}\n\
                 body {:?} wrap {:?}\n styles {:?}\n head layers {:?}\n last frames:{}",
                st.body_items,
                st.wrap_items,
                st.styles.iter().map(|s| s.1).collect::<Vec<_>>(),
                st.head_layers,
                tail.join("\n")
            );
        }
    }
}
