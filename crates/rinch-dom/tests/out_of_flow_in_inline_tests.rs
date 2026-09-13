//! #591 — an out-of-flow child of a `display: inline` element is laid out by
//! the inline's block container, like its wrapper-free twin.
//!
//! `<div style="position: relative"><span>text<div style="position: absolute">…</div>tail</span></div>`
//! laid the absolute out **nowhere**: the `<span>` is inline content, so the
//! marking pass removes its Taffy node from the IFC root (Parley lays the text
//! out instead), and the absolute — a Taffy child of the `<span>` — left the
//! tree with it. `0x0`, unreachable from the root, one `D detached` line, and
//! not a pixel of it painted, while the container was 20px tall exactly as
//! Chrome says, so every layout assertion passed on both sides of the defect.
//!
//! The fix is one Taffy edge: the IFC root loop's canonicalization of its
//! out-of-flow children (`setup_inline_formatting_contexts`, #466's measure-leaf
//! branch) now collects the out-of-flow boxes **beneath** a non-atomic inline
//! element too, so the absolute becomes a Taffy child of the root — the block
//! container, or the anonymous block box when the container has mixed content —
//! and Taffy lays it out against exactly the list its twin's absolute is in.
//! Paint and hit testing already reached the box through the stacking hoist;
//! they dropped it only for being `0x0`. CSS 2.1 §9.4.2 is honoured throughout:
//! the inline is **not** split around the box (`is_split_inline` stays false,
//! `ifc_root` stays set) — an out-of-flow box neither breaks an inline
//! formatting context nor forces anonymous-box generation.
//!
//! # The oracle
//!
//! **A bare inline wrapper changes nothing**, measured in Chrome 150 (standards
//! mode, `harnesses/session-2026-09-13/oracle-591.html`): every wrapped shape
//! below renders identically to its wrapper-deleted twin, to two decimals, text
//! rects included. So the fixtures assert twin identity — a property of the
//! *input* no implementation controls — and the control is a shape rinch renders
//! correctly today. Where Chrome's number is also asserted it is because it is a
//! declaration (`line-height: 20px`, a `40x30` box, an inset), never a glyph
//! measurement.
//!
//! Three divergences from Chrome are **pre-existing and hold for the unwrapped
//! twin too**, so they are stated in the fixture that meets them and not
//! pocketed: an absolute that is the *first* child takes its static position
//! after the line (Chrome: before it — the measure leaf goes in at index 0,
//! `ifc_out_of_flow_tests`); a `position: fixed` box with auto insets goes to
//! the viewport origin (Chrome: its static position —
//! `read_layout_results`' fixed arm); an inline-level absolute (`<span
//! style="position: absolute">`) takes a block's static position (Chrome: in the
//! line — Stylo blockifies it). A fourth is the one shape with no wrapper-free
//! twin: a `position: relative` inline is the absolute's containing block per
//! CSS 2.1 §10.1, and rinch resolves the insets against the block container
//! instead. Each is filed — #632 (static position after the line block), #633
//! (`fixed` auto insets), #634 (inline-level absolute), #631 (the `relative`
//! inline containing block); see the fixture docs.
//!
//! # Fixed points these fixtures step off
//!
//! The container at the origin (a stale box and a correct one agree there — so
//! the container is padded, or has a block above it); padding 0 on a restyle
//! (the stale `(0,0)` the wrapper keeps after `block → inline` adds nothing);
//! the pixel oracle at scale 1.0 alone (`the_twin_property_holds_at_scale_two`);
//! and "the container got the right height", which is true before and after
//! the fix for every shape here (#591's whole point).
//!
//! # What a green run here does not establish
//!
//! Vello: the pixel oracle is tiny-skia. Floats: rinch has none. The inside of a
//! re-measured `inline-block` (`<span style="display:inline-block"><span>text<abs/></span></span>`)
//! — a different mechanism, filed as #630.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;
/// One line box: `line-height: 20px` is declared so a line is 20px whatever the
/// font set, and the 40x30 box is never confusable with one.
const LINE: f32 = 20.0;
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px; position: relative";
const BLK: &str = "width: 40px; height: 30px; background: rgb(255, 0, 255)";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn abs_style(extra: &str) -> String {
    format!("{BLK}; position: absolute{extra}")
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

fn size_of(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.width, l.height)
}

/// Where paint puts `id`, relative to `container`'s painted border-box origin —
/// summed through the **box** tree the way `paint` and hit testing do, so a
/// wrapper whose `layout` is not zero would show up here.
fn painted_at(doc: &RinchDocument, id: NodeId, container: NodeId) -> (f32, f32) {
    let (x, y, _) = rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, id.0, 1.0);
    let (cx, cy, _) =
        rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, container.0, 1.0);
    ((x - cx) as f32, (y - cy) as f32)
}

/// The DOM node whose Taffy child list holds `id`'s Taffy node.
fn taffy_parent_dom(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    let t = doc.tree.get(id.0)?.taffy_id?;
    let p = doc.tree.taffy.parent(t)?;
    doc.tree.taffy_map.get(&p).copied()
}

/// Reachable from the document root's Taffy node — the property `D detached`
/// reports the absence of, asked directly.
fn reachable(doc: &RinchDocument, id: NodeId) -> bool {
    let root_taffy = doc.tree.get(doc.tree.root_id).unwrap().taffy_id.unwrap();
    let Some(t) = doc.tree.get(id.0).and_then(|n| n.taffy_id) else {
        return false;
    };
    let mut stack = vec![root_taffy];
    let mut seen = std::collections::HashSet::new();
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        if n == t {
            return true;
        }
        if let Ok(kids) = doc.tree.taffy.children(n) {
            stack.extend(kids);
        }
    }
    false
}

fn assert_consistent(doc: &RinchDocument, what: &str) {
    let t = doc.taffy_tree_violations();
    assert!(
        t.is_empty(),
        "{what}: Taffy tree inconsistent:\n  {}",
        t.join("\n  ")
    );
    let r = doc.run_bookkeeping_violations();
    assert!(
        r.is_empty(),
        "{what}: run bookkeeping inconsistent:\n  {}",
        r.join("\n  ")
    );
    let l = doc.ifc_leaf_invariant_violations();
    assert!(
        l.is_empty(),
        "{what}: IFC leaf invariant violated on dom {l:?}"
    );
}

/// Whether `container` minted anonymous block boxes this pass — i.e. its inline
/// runs are laid out by anonymous-box IFC roots rather than by itself.
trait MixedRoot {
    fn doc_is_mixed_root(&self, container: NodeId) -> bool;
}
impl MixedRoot for RinchDocument {
    fn doc_is_mixed_root(&self, container: NodeId) -> bool {
        !self.tree.get(container.0).unwrap().run_boxes.is_empty()
    }
}

struct Built {
    doc: RinchDocument,
    container: NodeId,
    abs: NodeId,
    /// The inline element the absolute sits in; `None` for the unwrapped twin.
    wrapper: Option<NodeId>,
}

/// Build one shape, `wrapped` in a bare `<span>`/`<a>` or with the wrapper
/// deleted. `tag` is the wrapper's tag. Every shape mirrors a case in the Chrome
/// oracle page named in the module doc.
fn build(shape: &str, wrapped: bool, tag: &str) -> Built {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container_style = match shape {
        "icb_auto" | "icb_insets" => "width: 400px; line-height: 20px; font-size: 16px".to_string(),
        "insets_br" => format!("{CONTAINER}; height: 100px"),
        "padded" => format!("{CONTAINER}; padding: 7px 11px; border: 3px solid rgb(0, 0, 0)"),
        "multiline" => format!("{CONTAINER}; width: 120px"),
        _ => CONTAINER.to_string(),
    };
    if matches!(shape, "icb_auto" | "icb_insets") {
        // Off the fixed point: the container must not sit at the viewport
        // origin, or "resolved against the ICB" and "resolved against the
        // container" give the same number.
        el(&mut doc, body, "div", "height: 50px");
    }
    let container = el(&mut doc, body, "div", &container_style);
    let mut wrapper = None;
    // The host the inline content is appended to.
    let mut host = container;
    if wrapped {
        let w = el(&mut doc, container, tag, "");
        wrapper = Some(w);
        host = w;
    }
    let abs = match shape {
        "middle" | "padded" | "icb_auto" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &abs_style(""));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "first" => {
            let a = el(&mut doc, host, "div", &abs_style(""));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "last" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &abs_style(""));
            txt(&mut doc, a, "block");
            a
        }
        "insets_tl" | "icb_insets" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &abs_style("; top: 0; left: 0"));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "insets_br" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &abs_style("; right: 0; bottom: 0"));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "top_only" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &abs_style("; top: 5px"));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "fixed" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "div", &format!("{BLK}; position: fixed"));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "tail");
            a
        }
        "nested" => {
            // Two inlines deep when wrapped; the unwrapped twin keeps the inner
            // `<b>` on both sides of the absolute, as Chrome's twin page does.
            // The block declares its own `font-weight`: wrapped, its label
            // inherits the `<b>`'s 700 and unwrapped it inherits 400 — true in
            // Chrome too, a property of the markup — and that is the one
            // difference the two markups cannot avoid (measured: band 1 was 828
            // against 805 without it; the same finding as #513's `nested`).
            if wrapped {
                let inner = el(&mut doc, host, "b", "");
                txt(&mut doc, inner, "text");
                let a = el(&mut doc, inner, "div", &abs_style("; font-weight: 400"));
                txt(&mut doc, a, "block");
                txt(&mut doc, inner, "tail");
                a
            } else {
                let before = el(&mut doc, host, "b", "");
                txt(&mut doc, before, "text");
                let a = el(&mut doc, host, "div", &abs_style("; font-weight: 400"));
                txt(&mut doc, a, "block");
                let after = el(&mut doc, host, "b", "");
                txt(&mut doc, after, "tail");
                a
            }
        }
        "contents_between" => {
            // A `display: contents` wrapper between the inline and the absolute.
            let c = el(&mut doc, host, "div", "display: contents");
            txt(&mut doc, c, "text");
            let a = el(&mut doc, c, "div", &abs_style(""));
            txt(&mut doc, a, "block");
            txt(&mut doc, c, "tail");
            a
        }
        "inline_level" => {
            txt(&mut doc, host, "text");
            let a = el(&mut doc, host, "span", &abs_style(""));
            txt(&mut doc, a, "box");
            txt(&mut doc, host, "tail");
            a
        }
        "multiline" => {
            txt(&mut doc, host, "alpha");
            let a = el(&mut doc, host, "div", &abs_style(""));
            txt(&mut doc, a, "block");
            txt(&mut doc, host, "beta gamma delta epsilon zeta");
            a
        }
        "mixed" => {
            // The container has mixed content, so its inline run is laid out by
            // an anonymous block box — which is then the IFC root that has to
            // collect the absolute. `text<span>x<abs/>y</span>tail<blk/>`; the
            // twin is `text x<abs/>y tail<blk/>`.
            if wrapped {
                txt(&mut doc, container, "text ");
                let w = el(&mut doc, container, tag, "");
                wrapper = Some(w);
                txt(&mut doc, w, "x");
                let a = el(&mut doc, w, "div", &abs_style(""));
                txt(&mut doc, a, "block");
                txt(&mut doc, w, "y");
                txt(&mut doc, container, " tail");
                let k = el(&mut doc, container, "div", BLK);
                txt(&mut doc, k, "k");
                a
            } else {
                txt(&mut doc, container, "text x");
                let a = el(&mut doc, container, "div", &abs_style(""));
                txt(&mut doc, a, "block");
                txt(&mut doc, container, "y tail");
                let k = el(&mut doc, container, "div", BLK);
                txt(&mut doc, k, "k");
                a
            }
        }
        other => panic!("unknown shape {other}"),
    };
    // `mixed` appends the wrapper itself, after the leading text.
    if shape == "mixed" && wrapped {
        // remove the empty wrapper `build` added before the match
        let stray = host;
        doc.remove_child(container, stray);
    }
    doc.resolve_layout(VW, VH);
    Built {
        doc,
        container,
        abs,
        wrapper,
    }
}

/// The twin property for one shape: same container height, same box size, same
/// painted position, both laid out by a reachable Taffy node, both trees clean —
/// and the wrapped absolute is a Taffy child of the **root that lays the line
/// out**, which is what was missing.
fn assert_twin(shape: &str, tag: &str) -> (Built, Built) {
    let w = build(shape, true, tag);
    let p = build(shape, false, tag);
    let what = format!("{shape} in <{tag}>");

    assert!(
        reachable(&p.doc, p.abs),
        "{what}: control — the unwrapped absolute is laid out"
    );
    assert!(
        reachable(&w.doc, w.abs),
        "{what}: the wrapped absolute is laid out by nobody (#591)"
    );
    assert_eq!(
        height_of(&w.doc, w.container),
        height_of(&p.doc, p.container),
        "{what}: container height differs from the twin"
    );
    assert_eq!(
        size_of(&w.doc, w.abs),
        size_of(&p.doc, p.abs),
        "{what}: the absolute's size differs from the twin"
    );
    assert_eq!(
        painted_at(&w.doc, w.abs, w.container),
        painted_at(&p.doc, p.abs, p.container),
        "{what}: the absolute is painted somewhere else than in the twin"
    );
    if let Some(wr) = w.wrapper {
        let n = w.doc.tree.get(wr.0).unwrap();
        assert!(
            !n.is_split_inline(),
            "{what}: an out-of-flow box must not split its inline (CSS 2.1 §9.4.2)"
        );
        assert!(
            n.ifc_root.is_some(),
            "{what}: the wrapper is still inline content of its IFC"
        );
        assert_ne!(
            taffy_parent_dom(&w.doc, w.abs),
            Some(wr.0),
            "{what}: the absolute is still a Taffy child of the detached inline"
        );
    }
    assert_consistent(&w.doc, &format!("{what}, wrapped"));
    assert_consistent(&p.doc, &format!("{what}, unwrapped"));
    (w, p)
}

// ── Static shapes ───────────────────────────────────────────────────────────

/// The issue's shape, under a padded and bordered container so that neither
/// coordinate of the answer is zero: Chrome puts the box at `(14, 30)` —
/// `border 3 + padding 11`, `border 3 + padding 7 + one 20px line`.
#[test]
fn an_out_of_flow_child_of_an_inline_is_laid_out_where_its_twin_is() {
    for tag in ["span", "a"] {
        let (w, _p) = assert_twin("padded", tag);
        assert_eq!(
            painted_at(&w.doc, w.abs, w.container),
            (14.0, 30.0),
            "the static position is the content-box origin plus the line (Chrome 14,30)"
        );
        assert_eq!(height_of(&w.doc, w.container), 3.0 + 7.0 + LINE + 7.0 + 3.0);
    }
}

/// Both ends of the run. Chrome places an absolute that is the **first** child at
/// `y = 0` — before the line — and rinch's leaf-first canonicalization puts it
/// after it, at `y = 20`, **for the unwrapped twin as well**
/// (`ifc_out_of_flow_tests::an_auto_inset_absolute_child_keeps_its_below_the_line_static_position`
/// documents the choice). Twin identity is what this pins; the divergence is
/// pre-existing and filed as #632.
#[test]
fn an_absolute_first_or_last_in_the_inline_lands_where_its_twin_does() {
    assert_twin("first", "span");
    assert_twin("last", "span");
}

/// Insets resolve against the **container**, not the inline: the containing
/// block of an absolute whose ancestors are a static inline and a `relative`
/// block is the block (CSS 2.1 §10.1). Chrome: `(0,0)`, `(360,70)`, and for a
/// single `top: 5px` the static x with the inset y, `(0,5)`.
#[test]
fn insets_resolve_against_the_container_not_the_inline() {
    let (w, _) = assert_twin("insets_tl", "span");
    assert_eq!(painted_at(&w.doc, w.abs, w.container), (0.0, 0.0));
    let (w, _) = assert_twin("insets_br", "span");
    assert_eq!(painted_at(&w.doc, w.abs, w.container), (360.0, 70.0));
    let (w, _) = assert_twin("top_only", "span");
    assert_eq!(painted_at(&w.doc, w.abs, w.container), (0.0, 5.0));
}

/// The recursion: two inlines deep, and through a `display: contents` wrapper
/// between the inline and the absolute.
#[test]
fn the_collection_recurses_through_nested_inlines_and_contents_wrappers() {
    assert_twin("nested", "span");
    assert_twin("contents_between", "span");
}

/// The IFC root is an **anonymous block box** when the container has mixed
/// content — and the absolute must still belong to the **container**, not to
/// that box. CSS resolves its insets and percentages against its containing
/// block (the positioned container, or the ICB), and Taffy resolves them against
/// the Taffy parent; so the hoisted box is a unit of the *container*
/// (`collect_run_units`), the anonymous box's run does not include it, and the
/// container's Taffy list does. Kills: hoisting into the IFC root that found it
/// — the anonymous box — which puts `top:0;left:0` at the box's origin `(14,10)`
/// where the twin and Chrome give the container's padding-box origin `(3,3)`
/// (the review of PR 2, finding A; the auto-inset case below is the fixed point
/// where the two answers coincide, which is why the inset cases follow).
#[test]
fn an_absolute_inside_an_inline_inside_an_anonymous_box_is_laid_out() {
    let (w, _) = assert_twin("mixed", "span");
    let wrapper = w.wrapper.unwrap();
    let root = w
        .doc
        .tree
        .get(wrapper.0)
        .unwrap()
        .ifc_root
        .expect("inline content");
    assert!(
        w.doc.tree.get(root).unwrap().is_anonymous_block_box,
        "precondition: the inline's IFC root is the anonymous block box for the run"
    );
    assert_eq!(
        taffy_parent_dom(&w.doc, w.abs),
        Some(w.container.0),
        "the absolute is a Taffy child of the CONTAINER — its containing block — not of the anonymous box that lays the line out"
    );
    assert_eq!(
        painted_at(&w.doc, w.abs, w.container),
        (0.0, LINE),
        "auto insets: below the one line, where Chrome puts it (a fixed point — see the inset fixture)"
    );
}

/// Finding A of the PR 2 review, off the fixed point: under a mixed-content
/// container the hoisted absolute resolves **insets and percentages against the
/// container**, as its twin does. With `padding: 7px 11px; border: 3px` the
/// container's padding box starts at `(3,3)`; an anonymous box that held the
/// absolute instead would put `top:0;left:0` at `(14,10)`, `right:0;bottom:0`
/// off by the box's size, `width: 50%` at half the box's width, and the ICB case
/// at the container's origin rather than the viewport's.
#[test]
fn under_an_anonymous_box_root_the_containing_block_is_still_the_container() {
    const PADDED: &str = "width: 400px; line-height: 20px; font-size: 16px; position: relative; \
                          padding: 7px 11px; border: 3px solid rgb(0, 0, 0)";
    // (inset style, what the twin must agree on)
    for (extra, label) in [
        ("; top: 0; left: 0", "top-left insets"),
        ("; right: 0; bottom: 0", "bottom-right insets"),
        ("; top: 0; left: 0; width: 50%", "percentage width"),
    ] {
        let build_mixed = |wrapped: bool| -> (RinchDocument, NodeId, NodeId) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let c = el(&mut doc, body, "div", PADDED);
            let abs = if wrapped {
                txt(&mut doc, c, "text ");
                let w = el(&mut doc, c, "span", "");
                txt(&mut doc, w, "x");
                let a = el(&mut doc, w, "div", &abs_style(extra));
                txt(&mut doc, a, "block");
                txt(&mut doc, w, "y");
                txt(&mut doc, c, " tail");
                a
            } else {
                txt(&mut doc, c, "text x");
                let a = el(&mut doc, c, "div", &abs_style(extra));
                txt(&mut doc, a, "block");
                txt(&mut doc, c, "y tail");
                a
            };
            let k = el(&mut doc, c, "div", BLK);
            txt(&mut doc, k, "k");
            doc.resolve_layout(VW, VH);
            (doc, c, abs)
        };
        let (wd, wc, wa) = build_mixed(true);
        let (pd, pc, pa) = build_mixed(false);
        assert!(
            wd.doc_is_mixed_root(wc),
            "{label}: precondition — the container mints anonymous boxes"
        );
        assert_eq!(
            taffy_parent_dom(&wd, wa),
            Some(wc.0),
            "{label}: the absolute's Taffy parent is the container"
        );
        assert_eq!(
            painted_at(&wd, wa, wc),
            painted_at(&pd, pa, pc),
            "{label}: painted where the twin paints it"
        );
        assert_eq!(
            size_of(&wd, wa),
            size_of(&pd, pa),
            "{label}: same size as the twin"
        );
        if extra == "; top: 0; left: 0" {
            assert_eq!(
                painted_at(&wd, wa, wc),
                (3.0, 3.0),
                "top:0;left:0 against the container's padding box (Chrome 3,3)"
            );
        }
        assert_consistent(&wd, label);
        assert_consistent(&pd, &format!("{label} twin"));
    }
}

/// The ICB half of finding A: container **static**, a block above it, the
/// mixed-content shape. `top:0;left:0` resolves against the viewport — `(0,-50)`
/// container-relative — for both twins; held by the anonymous box it was
/// `(14,-40)`, the #204 patch summing through a parent that was not the Taffy
/// parent.
#[test]
fn under_an_anonymous_box_root_the_icb_case_resolves_against_the_viewport() {
    let build = |wrapped: bool| -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        el(&mut doc, body, "div", "height: 50px");
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px",
        );
        let abs = if wrapped {
            txt(&mut doc, c, "text ");
            let w = el(&mut doc, c, "span", "");
            txt(&mut doc, w, "x");
            let a = el(&mut doc, w, "div", &abs_style("; top: 0; left: 0"));
            txt(&mut doc, a, "block");
            txt(&mut doc, w, "y");
            txt(&mut doc, c, " tail");
            a
        } else {
            txt(&mut doc, c, "text x");
            let a = el(&mut doc, c, "div", &abs_style("; top: 0; left: 0"));
            txt(&mut doc, a, "block");
            txt(&mut doc, c, "y tail");
            a
        };
        let k = el(&mut doc, c, "div", BLK);
        txt(&mut doc, k, "k");
        doc.resolve_layout(VW, VH);
        (doc, c, abs)
    };
    let (wd, wc, wa) = build(true);
    let (pd, pc, pa) = build(false);
    assert_eq!(
        painted_at(&wd, wa, wc),
        (0.0, -50.0),
        "the viewport origin, 50px above the container"
    );
    assert_eq!(painted_at(&wd, wa, wc), painted_at(&pd, pa, pc));
    assert_consistent(&wd, "icb mixed");
}

/// `position: fixed` inside the inline. Both twins go to the viewport origin
/// (`read_layout_results`' fixed arm sends auto insets to 0); Chrome uses the
/// static position `(0, 20)`. Pre-existing for the direct child, filed as #633; what this
/// pins is that the fixed box is laid out at all and agrees with its twin.
#[test]
fn a_fixed_child_of_an_inline_agrees_with_its_twin() {
    let (w, _) = assert_twin("fixed", "span");
    assert_eq!(size_of(&w.doc, w.abs), (40.0, 30.0));
}

/// An **inline-level** absolute (`<span style="position: absolute">`). Stylo
/// blockifies it, so both twins give it a block's static position below the
/// line; Chrome keeps it in the line after `text`. Pre-existing, filed as #634; the hoist
/// is by role, not by tag, which is what this pins.
#[test]
fn an_inline_level_absolute_agrees_with_its_twin() {
    assert_twin("inline_level", "span");
}

/// No positioned ancestor: the initial containing block (#204). Auto insets keep
/// the static position; `top: 0; left: 0` goes to the **viewport** origin, which
/// is 50px above the container here — the block above the container is what
/// makes "ICB" and "container" different answers.
#[test]
fn an_absolute_in_an_inline_with_no_positioned_ancestor_resolves_against_the_icb() {
    let (w, _) = assert_twin("icb_auto", "span");
    assert_eq!(painted_at(&w.doc, w.abs, w.container), (0.0, LINE));
    let (w, _) = assert_twin("icb_insets", "span");
    assert_eq!(
        painted_at(&w.doc, w.abs, w.container),
        (0.0, -50.0),
        "top:0;left:0 against the ICB is the viewport origin, 50px above the container"
    );
}

/// A three-line run. Chrome's static position is after the line containing the
/// preceding content (`y = 20`); rinch's is after the whole run (`y = 60`), for
/// the unwrapped twin as well — the measure leaf is one Taffy child. Twin
/// identity is pinned; the divergence is filed with the abs-first one, #632.
#[test]
fn an_absolute_in_a_multi_line_run_agrees_with_its_twin() {
    let (w, _) = assert_twin("multiline", "span");
    assert_eq!(height_of(&w.doc, w.container), 3.0 * LINE, "three lines");
}

/// **A `position: relative` inline is the absolute's containing block** (CSS 2.1
/// §10.1) and rinch does not model that: the insets resolve against the block
/// container, so `top: 0; left: 0` lands at the container's `(0, 0)` where
/// Chrome lands at the span's first fragment — `(34.7, 1)` with `lead ` before
/// the span, `(0, 1)` without. Same class as #386 (the containing block is a
/// positioned ancestor that is not the Taffy parent), one level worse because
/// this ancestor has no box in Taffy at all. Filed as #631; this fixture pins that the
/// box is laid out, reachable, painted, and states the number it gets.
#[test]
fn an_absolute_in_a_relative_inline_is_laid_out_and_the_containing_block_is_stated() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, container, "lead ");
    let span = el(&mut doc, container, "span", "position: relative");
    txt(&mut doc, span, "text");
    let abs = el(&mut doc, span, "div", &abs_style("; top: 0; left: 0"));
    txt(&mut doc, abs, "block");
    txt(&mut doc, span, "tail");
    doc.resolve_layout(VW, VH);

    assert!(reachable(&doc, abs), "laid out");
    assert_eq!(size_of(&doc, abs), (40.0, 30.0));
    assert_eq!(
        painted_at(&doc, abs, container),
        (0.0, 0.0),
        "DOCUMENTED DIVERGENCE (#631): resolved against the container; Chrome resolves \
         against the span's first fragment (34.7, 1). When the containing-block \
         #631 lands this assertion is the one to flip."
    );
    assert_consistent(&doc, "relative inline");
}

// ── Transitions ─────────────────────────────────────────────────────────────
//
// Each `resolve_layout` after the first changes the viewport, so the
// `!layout_dirty` early return cannot skip a pass (a mutation sets it anyway;
// this is belt and braces against a fixture that tests nothing).

/// Append an absolute into an inline that has already been laid out. The DOM
/// insert attaches it to the `<span>`'s Taffy list; the next pass must take it.
#[test]
fn an_absolute_appended_into_a_laid_out_inline_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = el(&mut doc, body, "div", CONTAINER);
    let span = el(&mut doc, container, "span", "");
    txt(&mut doc, span, "text");
    txt(&mut doc, span, "tail");
    doc.resolve_layout(VW, VH);
    let abs = el(&mut doc, span, "div", &abs_style(""));
    txt(&mut doc, abs, "block");
    doc.resolve_layout(VW + 1.0, VH);

    let twin = build("middle", false, "span");
    assert!(reachable(&doc, abs));
    assert_eq!(
        painted_at(&doc, abs, container),
        painted_at(&twin.doc, twin.abs, twin.container)
    );
    assert_consistent(&doc, "appended");
}

/// `absolute → static → absolute`, then the container gains padding. The first
/// crossing makes the inline a split inline (#513); the second undoes that and
/// must leave **no stale box**: on the unfixed base the absolute kept the split
/// pass's `(0, 20, 40x30)`, was painted from it, and did not move when the
/// container's padding changed — the padding step is what takes this off the
/// fixed point where a stale box and a correct one coincide.
#[test]
fn an_absolute_restyled_static_and_back_follows_its_container() {
    let mut b = build("middle", true, "span");
    b.doc.set_attribute(b.abs, "style", BLK);
    b.doc.resolve_layout(VW + 1.0, VH);
    assert!(
        b.doc
            .tree
            .get(b.wrapper.unwrap().0)
            .unwrap()
            .is_split_inline(),
        "precondition: a static block inside the inline splits it (#513)"
    );
    assert_eq!(height_of(&b.doc, b.container), LINE + 30.0 + LINE);

    b.doc.set_attribute(b.abs, "style", &abs_style(""));
    b.doc.resolve_layout(VW + 2.0, VH);
    assert_eq!(height_of(&b.doc, b.container), LINE, "out of flow again");
    assert!(reachable(&b.doc, b.abs));
    assert_eq!(painted_at(&b.doc, b.abs, b.container), (0.0, LINE));
    assert_consistent(&b.doc, "static -> absolute");

    b.doc.set_attribute(
        b.container,
        "style",
        &format!("{CONTAINER}; padding-top: 13px; padding-left: 17px"),
    );
    b.doc.resolve_layout(VW + 3.0, VH);
    assert_eq!(
        painted_at(&b.doc, b.abs, b.container),
        (17.0, 13.0 + LINE),
        "the box follows the container's padding — a stale box would not have moved"
    );
    assert_consistent(&b.doc, "after padding");
}

/// Remove the absolute from the inline after it has been hoisted.
///
/// The hoist makes the box's Taffy parent the IFC root while its DOM parent is
/// still the `<span>`. A `remove_child` that detached only from the DOM parent's
/// Taffy list left the node in the root's, and the next pass — with no
/// out-of-flow DOM child left to canonicalize — made the root carry
/// `InlineRoot` on a non-leaf: the #466 leaf invariant, a debug panic, and in
/// release a container collapsing to `0`. `taffy_detach_contribution` now
/// removes a node from the Taffy list that actually holds it.
///
/// **Fail-first is against the hoist commit of this PR**, not against `main`: on
/// `main` the box was never hoisted, so removing it from the `<span>` was
/// already right.
#[test]
fn removing_the_hoisted_absolute_leaves_a_clean_tree() {
    let mut b = build("middle", true, "span");
    assert!(reachable(&b.doc, b.abs), "precondition: hoisted");
    b.doc.remove_child(b.wrapper.unwrap(), b.abs);
    b.doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(height_of(&b.doc, b.container), LINE);
    assert_consistent(&b.doc, "after remove");
    let root_taffy = b.doc.tree.get(b.container.0).unwrap().taffy_id.unwrap();
    assert!(
        b.doc.tree.taffy.children(root_taffy).unwrap().is_empty(),
        "the root's Taffy list no longer names the removed box"
    );
}

/// Move the hoisted absolute from the inline to the container (a DOM
/// `remove_child` + `append_child`). The detach must come off the list that
/// holds it, or the append adds a second edge to the same Taffy node — an
/// `A double-claim` until a canonicalization happens to collapse it.
#[test]
fn moving_the_hoisted_absolute_out_of_the_inline_leaves_one_edge() {
    let mut b = build("middle", true, "span");
    b.doc.remove_child(b.wrapper.unwrap(), b.abs);
    b.doc.append_child(b.container, b.abs);
    // Before the next layout pass: exactly one Taffy parent edge.
    let t = b.doc.tree.get(b.abs.0).unwrap().taffy_id.unwrap();
    let root_taffy = b.doc.tree.get(b.container.0).unwrap().taffy_id.unwrap();
    let count = b
        .doc
        .tree
        .taffy
        .children(root_taffy)
        .unwrap()
        .iter()
        .filter(|&&c| c == t)
        .count();
    assert_eq!(
        count, 1,
        "the moved box is in the container's list exactly once"
    );
    b.doc.resolve_layout(VW + 1.0, VH);
    let twin = build("middle", false, "span");
    assert_eq!(
        painted_at(&b.doc, b.abs, b.container),
        painted_at(&twin.doc, twin.abs, twin.container)
    );
    assert_consistent(&b.doc, "after move");
}

/// The wrapper crosses `block → inline → block` under container padding
/// `13/17`. After the first crossing the wrapper is a flowed inline element
/// and its block pass's `(17, 13, …)` box must add nothing to the absolute's
/// painted position (that zeroing is PR 1's, pinned here at the consumer it was
/// for): the box must paint at `(17, 33)` — Chrome's number and the static
/// twin's — not at `(34, 46)`. The second crossing heals (#597).
#[test]
fn the_wrapper_crossing_block_inline_block_under_padding_keeps_the_box_in_place() {
    const PADDED: &str = "width: 400px; line-height: 20px; font-size: 16px; position: relative; padding-top: 13px; padding-left: 17px";
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = el(&mut doc, body, "div", PADDED);
    let span = el(&mut doc, container, "span", "display: block");
    txt(&mut doc, span, "text");
    let abs = el(&mut doc, span, "div", &abs_style(""));
    txt(&mut doc, abs, "block");
    txt(&mut doc, span, "tail");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        painted_at(&doc, abs, container),
        (17.0, 13.0 + LINE),
        "block wrapper"
    );

    doc.set_attribute(span, "style", "display: inline");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(
        doc.tree.get(span.0).unwrap().layout,
        rinch_dom::node::LayoutResult::default(),
        "a flowed inline element owns no box (PR 1)"
    );
    assert!(reachable(&doc, abs));
    assert_eq!(
        painted_at(&doc, abs, container),
        (17.0, 13.0 + LINE),
        "block -> inline: the box stays where Chrome puts it; (34, 46) is the stale wrapper double-counted"
    );
    assert_consistent(&doc, "block -> inline");

    // The same end state, built fresh, is the control.
    let mut doc2 = RinchDocument::new();
    let body2 = doc2.body();
    let c2 = el(&mut doc2, body2, "div", PADDED);
    let s2 = el(&mut doc2, c2, "span", "");
    txt(&mut doc2, s2, "text");
    let a2 = el(&mut doc2, s2, "div", &abs_style(""));
    txt(&mut doc2, a2, "block");
    txt(&mut doc2, s2, "tail");
    doc2.resolve_layout(VW, VH);
    assert_eq!(painted_at(&doc2, a2, c2), painted_at(&doc, abs, container));

    doc.set_attribute(span, "style", "display: block");
    doc.resolve_layout(VW + 2.0, VH);
    assert_eq!(
        painted_at(&doc, abs, container),
        (17.0, 13.0 + LINE),
        "inline -> block"
    );
    assert_consistent(&doc, "inline -> block");
}

// ── Lifecycle: the hoist condition ends ─────────────────────────────────────
//
// The review of PR 2 found that the hoist had no record-and-restore: when the
// inline element left the document or became an atomic inline, the box's edge
// into the host's Taffy list stayed, and the IFC root became a non-leaf
// `InlineRoot` carrier — the #466 leaf invariant panicked (debug) or the
// container collapsed to 0 (release). All clean on PR 1's base: regressions the
// hoist introduced. `taffy_detach_contribution` now cuts every edge leaving a
// removed subtree, and `rehome_hoisted_out_of_flow` rebuilds the old and new
// host's lists when a box's host changes.

const CPAD: &str = "width: 400px; line-height: 20px; font-size: 16px; position: relative; \
                    padding: 7px 11px; border: 3px solid rgb(0, 0, 0)";

/// Build `lead <span>text<abs/>tail</span>` (or without the lead) under `CPAD`.
fn lifecycle_doc(lead: bool) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CPAD);
    if lead {
        txt(&mut doc, c, "lead ");
    }
    let span = el(&mut doc, c, "span", "");
    txt(&mut doc, span, "text");
    let abs = el(&mut doc, span, "div", &abs_style(""));
    txt(&mut doc, abs, "block");
    txt(&mut doc, span, "tail");
    doc.resolve_layout(VW, VH);
    assert!(reachable(&doc, abs), "precondition: hoisted");
    assert_eq!(
        taffy_parent_dom(&doc, abs),
        Some(c.0),
        "precondition: hosted by the container"
    );
    (doc, c, span, abs)
}

/// T8 of the review: remove the span while the container keeps other inline
/// text. The container stays an IFC root, finds no out-of-flow box, and its
/// Taffy node must be a leaf — with the stale edge it was not, and setup
/// panicked.
#[test]
fn removing_the_inline_while_the_container_keeps_text_leaves_a_clean_tree() {
    let (mut doc, c, span, abs) = lifecycle_doc(true);
    doc.remove_child(c, span);
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(
        height_of(&doc, c),
        3.0 + 7.0 + LINE + 7.0 + 3.0,
        "one line of `lead`"
    );
    assert!(
        !reachable(&doc, abs),
        "the removed subtree's box is laid out by nobody, rightly"
    );
    let root_taffy = doc.tree.get(c.0).unwrap().taffy_id.unwrap();
    assert!(
        doc.tree.taffy.children(root_taffy).unwrap().is_empty(),
        "the container's Taffy list no longer names the removed box"
    );
    assert_consistent(&doc, "span removed, lead kept");
}

/// T9 of the review: remove the span that was the container's only child. The
/// box must leave the container's Taffy list — before the fix it stayed
/// reachable with every validator silent, and became T8 the moment the container
/// got inline content again.
#[test]
fn removing_the_inline_that_was_the_only_child_takes_its_box_with_it() {
    let (mut doc, c, span, abs) = lifecycle_doc(false);
    doc.remove_child(c, span);
    // Before the next pass: the edge is already cut at mutation time. (The
    // root's measure leaf is still in the list — it is Taffy-only and the next
    // pass recreates it — so the assertion is about the box's id, not emptiness.)
    let root_taffy = doc.tree.get(c.0).unwrap().taffy_id.unwrap();
    let abs_taffy = doc.tree.get(abs.0).unwrap().taffy_id.unwrap();
    assert!(
        !doc.tree
            .taffy
            .children(root_taffy)
            .unwrap()
            .contains(&abs_taffy),
        "the container's Taffy list no longer names the removed box, before any pass"
    );
    doc.resolve_layout(VW + 1.0, VH);
    assert!(!reachable(&doc, abs));
    assert_consistent(&doc, "only child removed");
    // …and the container takes text again without incident (this is what T9
    // would have turned into).
    txt(&mut doc, c, "again");
    doc.resolve_layout(VW + 2.0, VH);
    assert_eq!(height_of(&doc, c), 3.0 + 7.0 + LINE + 7.0 + 3.0);
    assert_consistent(&doc, "text after removal");
}

/// T12 / T18 of the review and their `inline-grid` sibling: the span becomes an
/// **atomic** inline. Its interior is a Taffy root of its own now, so the box
/// belongs to it — the host change must move the edge from the container to the
/// span, or the container carries `InlineRoot` on a non-leaf and setup panics.
/// Then back to `inline`, which must re-hoist.
#[test]
fn the_inline_becoming_an_atomic_inline_takes_its_box_back_and_gives_it_up_again() {
    for display in ["inline-block", "inline-flex", "inline-grid"] {
        let (mut doc, c, span, abs) = lifecycle_doc(false);
        doc.set_attribute(
            span,
            "style",
            &format!("display: {display}; position: relative"),
        );
        doc.resolve_layout(VW + 1.0, VH);
        assert_eq!(
            taffy_parent_dom(&doc, abs),
            Some(span.0),
            "{display}: the box is the atomic inline's own child now"
        );
        assert_eq!(
            doc.tree.get(abs.0).unwrap().hoisted_out_of_flow_to,
            None,
            "{display}: no longer hoisted"
        );
        assert_eq!(
            size_of(&doc, abs),
            (40.0, 30.0),
            "{display}: laid out inside it"
        );
        assert_consistent(&doc, &format!("span -> {display}"));

        doc.set_attribute(span, "style", "");
        doc.resolve_layout(VW + 2.0, VH);
        assert_eq!(
            taffy_parent_dom(&doc, abs),
            Some(c.0),
            "{display} -> inline: hoisted to the container again"
        );
        assert_eq!(
            painted_at(&doc, abs, c),
            (14.0, 30.0),
            "{display} -> inline: Chrome's 14,30"
        );
        assert_consistent(&doc, &format!("{display} -> inline"));
    }
}

/// T21 of the review, the negative control: the span goes `display: none` while
/// the container keeps text. The `none` crossing already rebuilt the span's list
/// before this PR (T10 was clean), so this must stay clean and paint nothing of
/// the hidden subtree.
#[test]
fn the_inline_going_display_none_beside_text_stays_clean() {
    let (mut doc, c, span, abs) = lifecycle_doc(true);
    doc.set_attribute(span, "style", "display: none");
    doc.resolve_layout(VW + 1.0, VH);
    assert_eq!(height_of(&doc, c), 3.0 + 7.0 + LINE + 7.0 + 3.0);
    assert!(
        !reachable(&doc, abs),
        "a hidden subtree's box is laid out by nobody"
    );
    assert_consistent(&doc, "span -> none");
    doc.set_attribute(span, "style", "");
    doc.resolve_layout(VW + 2.0, VH);
    assert_eq!(taffy_parent_dom(&doc, abs), Some(c.0));
    assert_consistent(&doc, "none -> inline");
}

/// P14e of the re-review — the **two-departures** shape §3.2′ had only
/// forecast: the span leaves the inline flow (`display: block`) **and** its
/// hoisted box is removed, in one frame, in both orders; then the box is
/// re-added and the span returns to `inline`, again in one frame. The end state
/// must be the twin's `(14,30)`, and every intermediate tree clean. Each leg
/// crosses two of the three locks at once (the subtree-edge detach, the rehome
/// pass, the block/inline crossing's own rebuild), which is where an
/// order-dependent interaction between them would show.
#[test]
fn two_departures_in_one_frame_in_either_order_end_where_the_twin_does() {
    for abs_first in [false, true] {
        let (mut doc, c, span, abs) = lifecycle_doc(false);
        let label = if abs_first {
            "abs removed, then span -> block"
        } else {
            "span -> block, then abs removed"
        };
        if abs_first {
            doc.remove_child(span, abs);
            doc.set_attribute(span, "style", "display: block");
        } else {
            doc.set_attribute(span, "style", "display: block");
            doc.remove_child(span, abs);
        }
        doc.resolve_layout(VW + 1.0, VH);
        assert!(
            !reachable(&doc, abs),
            "{label}: the removed box is laid out by nobody"
        );
        assert_eq!(
            height_of(&doc, c),
            3.0 + 7.0 + LINE + 7.0 + 3.0,
            "{label}: one line of text in a block span"
        );
        assert_consistent(&doc, label);

        // The way back, also two departures in one frame.
        doc.append_child(span, abs);
        doc.set_attribute(span, "style", "");
        doc.resolve_layout(VW + 2.0, VH);
        assert_eq!(
            taffy_parent_dom(&doc, abs),
            Some(c.0),
            "{label}: re-added and re-hoisted to the container"
        );
        assert_eq!(
            painted_at(&doc, abs, c),
            (14.0, 30.0),
            "{label}: the end state is the twin's (Chrome 14,30)"
        );
        assert_consistent(&doc, &format!("{label}, then back"));
    }
}

// ── The atomic-inline boundary (review mutant c) ─────────────────────────────

/// An absolute inside an **atomic** inline inside a span belongs to the atomic
/// inline, not to the container: its interior is a Taffy root of its own, and
/// `DropdownMenu`'s panels live under exactly such an `inline-block` root. The
/// mutant that walks into atomic inlines too hoists them all to the outer
/// container and was killed only by `popup_backdrop_hit_tests`; this pins the
/// `DisplayMode::Inline` guard where #591's own suite can see it. All three
/// atomic displays, under `CPAD`, the atomic inline `position: relative` so the
/// insets resolve against it — and `top:0;left:0` lands at its origin, not the
/// container's `(3,3)`.
#[test]
fn an_absolute_inside_an_atomic_inline_inside_a_span_belongs_to_the_atomic_inline() {
    for display in ["inline-block", "inline-flex", "inline-grid"] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CPAD);
        let span = el(&mut doc, c, "span", "");
        txt(&mut doc, span, "text ");
        let atomic = el(
            &mut doc,
            span,
            "span",
            &format!("display: {display}; position: relative; width: 100px; height: 20px"),
        );
        let abs = el(&mut doc, atomic, "div", &abs_style("; top: 0; left: 0"));
        txt(&mut doc, abs, "block");
        txt(&mut doc, span, " tail");
        doc.resolve_layout(VW, VH);

        assert_eq!(
            doc.tree.get(abs.0).unwrap().hoisted_out_of_flow_to,
            None,
            "{display}: not hoisted — the atomic inline resets the walk"
        );
        assert_eq!(
            taffy_parent_dom(&doc, abs),
            Some(atomic.0),
            "{display}: the atomic inline is the Taffy parent"
        );
        let (ax, ay, _) =
            rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, atomic.0, 1.0);
        let (bx, by, _) =
            rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, abs.0, 1.0);
        assert_eq!(
            (bx - ax, by - ay),
            (0.0, 0.0),
            "{display}: painted at the atomic inline's origin, not the container's"
        );
        assert_ne!(
            painted_at(&doc, abs, c),
            (3.0, 3.0),
            "{display}: …which is not the container's padding-box origin"
        );
        assert_consistent(&doc, display);
    }
}

// ── Layer bounds (review finding C) ─────────────────────────────────────────

/// Under `opacity: 0.5` the container paints into a layer whose bounds
/// `layer_bounds` computes by walking the box tree; tiny-skia ignores the
/// bounds and Vello clips to them. The walk skips an IFC-flowed child and does
/// not descend into it, so a box reached only *through* the span was outside
/// the layer: wrapped `400x40` against the twin's `400x60`, the lower 20px of
/// the box gone on the GPU. As a unit of the container the box is a box-tree
/// child the walk sees directly, and the bounds agree.
#[test]
fn the_opacity_layer_bounds_include_the_hoisted_box() {
    const OPAQUE: &str =
        "width: 400px; line-height: 20px; font-size: 16px; position: relative; opacity: 0.5";
    let build = |wrapped: bool| -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", OPAQUE);
        let host = if wrapped {
            el(&mut doc, c, "span", "")
        } else {
            c
        };
        txt(&mut doc, host, "text");
        let a = el(&mut doc, host, "div", &abs_style(""));
        txt(&mut doc, a, "block");
        txt(&mut doc, host, "tail");
        doc.resolve_layout(VW, VH);
        (doc, c)
    };
    let (wd, wc) = build(true);
    let (pd, pc) = build(false);
    let bounds = |doc: &RinchDocument, c: NodeId| {
        let (x, y, _) =
            rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, c.0, 1.0);
        rinch_dom::paint::opacity_layer_bounds(&doc.tree, c.0, 1.0, x, y)
    };
    let (wb, pb) = (bounds(&wd, wc), bounds(&pd, pc));
    assert_eq!(wb, pb, "the wrapped layer is as tall as the twin's");
    assert!(
        (pb.y1 - pb.y0) >= LINE as f64 + 30.0,
        "control: the twin's layer reaches the bottom of the box, got {pb:?}"
    );
}

// ── Pixels ──────────────────────────────────────────────────────────────────

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn pixels_at(doc: &mut RinchDocument, scale: f32) -> Vec<[u8; 4]> {
        let (w, h) = ((VW * scale) as u32, (VH * scale) as u32);
        let mut painter = TinySkiaPainter::new(w, h);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale.into(),
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        painter.pixels().as_chunks::<4>().0.to_vec()
    }

    /// Bounding box of the block's `rgb(255, 0, 255)`, in device pixels.
    fn magenta_bbox(doc: &mut RinchDocument, scale: f32) -> Option<(usize, usize, usize, usize)> {
        let px = pixels_at(doc, scale);
        let w = (VW * scale) as usize;
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
        for (i, p) in px.iter().enumerate() {
            if p[3] > 0 && p[0] > 200 && p[1] < 60 && p[2] > 200 {
                let (x, y) = (i % w, i / w);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                n += 1;
            }
        }
        (n > 0).then_some((x0, y0, x1 + 1, y1 + 1))
    }

    /// Ink per 20px band, six bands deep, at `scale`.
    fn band_profile(doc: &mut RinchDocument, scale: f32) -> Vec<usize> {
        let px = pixels_at(doc, scale);
        let w = (VW * scale) as usize;
        let band_h = (LINE * scale) as usize;
        (0..6)
            .map(|b| {
                (b * band_h..(b + 1) * band_h)
                    .flat_map(|y| px[y * w..(y + 1) * w].iter())
                    .filter(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
                    .count()
            })
            .collect()
    }

    /// The issue's own oracle: ink per band, wrapped vs unwrapped. Before the
    /// fix `[348, 0, 0, …]` against `[348, 805, 400, …]` — the absolute's 40x30
    /// simply gone. (`block_in_inline_tests` carries the same comparison for its
    /// `abs` shape; this one is under the padded container, off the origin.)
    #[test]
    fn the_wrapper_changes_no_pixel() {
        for shape in ["middle", "padded", "insets_br", "nested", "mixed"] {
            let mut w = build(shape, true, "span");
            let mut p = build(shape, false, "span");
            let (wb, pb) = (band_profile(&mut w.doc, 1.0), band_profile(&mut p.doc, 1.0));
            assert!(
                magenta_bbox(&mut p.doc, 1.0).is_some(),
                "{shape}: control draws the block at all"
            );
            assert_eq!(
                wb, pb,
                "{shape}: a bare inline wrapper must change no pixel"
            );
            assert_eq!(
                magenta_bbox(&mut w.doc, 1.0),
                magenta_bbox(&mut p.doc, 1.0),
                "{shape}: the block is drawn at the same place"
            );
        }
    }

    /// Every other pixel assertion in the workspace runs at 1.0, where an offset
    /// hoisted out of a `* scale` is invisible.
    #[test]
    fn the_twin_property_holds_at_scale_two() {
        let mut w = build("padded", true, "span");
        let mut p = build("padded", false, "span");
        assert_eq!(band_profile(&mut w.doc, 2.0), band_profile(&mut p.doc, 2.0));
        let bb = magenta_bbox(&mut w.doc, 2.0).expect("drawn");
        assert_eq!(bb, magenta_bbox(&mut p.doc, 2.0).unwrap());
        // 40x30 at scale 2 is 80x60 device pixels, minus the label's ink.
        assert_eq!((bb.2 - bb.0, bb.3 - bb.1), (80, 60));
    }
}
