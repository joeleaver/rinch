//! #597 — an element restyled *out of* inline-level content must rejoin its
//! parent's Taffy child list.
//!
//! `mark_inline_descendants` detaches inline-level children from their IFC
//! root's Taffy child list, because Parley lays that content out and the root
//! draws it. That detach is **one-way**: nothing puts a child back when the
//! reason for detaching goes away. An element restyled from inline-level to
//! block-level at runtime ends with no Taffy parent, its parent's Taffy child
//! list empty, and whatever box it had while it was inline — a stale one, or
//! `0x0` if it was never laid out as a box at all.
//!
//! # The oracle: the same end state, declared rather than reached
//!
//! Every fixture here builds one final DOM **twice**:
//!
//! - **restyled** — built inline-level, laid out, then `set_attribute`d into the
//!   final state and laid out again;
//! - **declared** — built in the final state and laid out once.
//!
//! and asserts the two agree. That is a property of the *input* — whether a
//! transition happened — which no candidate implementation controls, so a
//! fixture written on it bites under any of them. It is also font-independent:
//! nothing below asserts a text measurement, only agreement between two
//! documents measured with the same fonts, plus heights that come from declared
//! `height`/`line-height` values.
//!
//! The issue's own table is this oracle in miniature, and the third row is why
//! reading the `display` **value** gets the wrong answer:
//!
//! | case | shape | before this fix |
//! |---|---|---|
//! | declared | `<button style="display: flex">` from the first layout | attached, laid out |
//! | restyled | bare `<button>` → `set_attribute("style", "display: flex")` | **stranded** |
//! | bare | a bare `<button>` left alone | **legitimately** detached — guard rail below |
//!
//! # Two mechanisms, and the fixtures separate them
//!
//! 1. **Nothing re-attaches.** `mark_inline_descendants` keeps no departure
//!    record, so no later pass knows a box is missing from a list.
//! 2. **Nothing notices.** `DisplayValue::to_taffy` is not injective:
//!    `inline`/`block` both map to `taffy::Display::Block`, and
//!    `inline-block`/`flex`/`inline-flex`/`contents` all map to
//!    `taffy::Display::Flex`. `apply_to_taffy` only sets `ifc_dirty` when the
//!    **Taffy** display changes, so `inline-block → flex` — the issue's own
//!    repro — never re-ran the IFC pass at all. (`contents` was already
//!    special-cased on the computed-display crossing itself, #520; the
//!    inline-level crossing is the same shape and was not.)
//!
//! `the_span_and_svg_route` needs only (1): its parent's `block → flex` is a
//! real Taffy display change, so the pass does run, and so does
//! `an_absolutely_positioned_button_rejoins`, where Taffy's `position` field
//! changes. `case_b` needs both.
//!
//! # Fixed points stepped off on purpose
//!
//! - **One child.** A single restyled child is where "re-attach it" and
//!   "rebuild the whole list" agree about order, so
//!   `three_restyled_buttons_come_back_in_document_order` restyles three.
//! - **One transition.** Healing once and never re-detaching passes a
//!   single-transition fixture, so `restyling_back_to_inline_level_detaches_again`
//!   goes there and back.
//! - **The direct parent.** A child whose Taffy list is its DOM parent's is
//!   where "ask the parent" and "walk the flattening" agree, so
//!   `a_button_behind_a_display_contents_wrapper_rejoins` puts a boxless
//!   wrapper in between, and `a_button_between_two_text_runs_rejoins` puts
//!   anonymous block boxes beside it.
//!
//! # Deliberately not covered here
//!
//! The **mirror** case — a block-level child stranded when its *wrapper* is
//! restyled `flex → inline` — is a different defect (it is #513's static
//! block-in-inline defect reached by a transition) and is not fixed by this
//! work. `the_mirror_case_gains_no_new_taffy_violation` is a guard rail only:
//! it passes before and after, and is here so an over-reaching fix is caught,
//! not as coverage of the mirror case.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// Whether Taffy holds this node in some parent's child list.
fn attached(doc: &RinchDocument, id: NodeId) -> bool {
    let Some(t) = doc.tree.get(id.0).and_then(|n| n.taffy_id) else {
        return false;
    };
    doc.tree.taffy.parent(t).is_some()
}

fn ifc_root(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    doc.tree.get(id.0).and_then(|n| n.ifc_root)
}

fn geom(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = &doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

/// The node's index in its Taffy parent's child list, so order is asserted as
/// order rather than inferred from coordinates.
fn taffy_index(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    let t = doc.tree.get(id.0).and_then(|n| n.taffy_id)?;
    let p = doc.tree.taffy.parent(t)?;
    doc.tree
        .taffy
        .children(p)
        .ok()?
        .iter()
        .position(|c| *c == t)
}

fn assert_clean(doc: &RinchDocument, what: &str) {
    let v = doc.taffy_tree_violations();
    assert!(
        v.is_empty(),
        "{what}: Taffy tree inconsistent:\n  {}",
        v.join("\n  ")
    );
}

/// Build `shape` twice and hand back both documents: `.0` reached the final
/// state by a runtime restyle, `.1` declared it from the first layout.
///
/// `shape` is called with a flag saying which document it is building; every
/// fixture uses it to decide whether to declare the final style up front or
/// apply it after the first `resolve_layout`.
fn twins(
    shape: impl Fn(&mut RinchDocument, bool) -> Vec<NodeId>,
) -> (RinchDocument, RinchDocument) {
    let mut restyled = RinchDocument::new();
    shape(&mut restyled, true);
    let mut declared = RinchDocument::new();
    shape(&mut declared, false);
    (restyled, declared)
}

/// Assert the two documents agree about every node the shape handed back.
fn assert_paths_agree(restyled: &RinchDocument, declared: &RinchDocument, ids: &[(&str, NodeId)]) {
    assert_clean(restyled, "restyled");
    assert_clean(declared, "declared");
    for (name, id) in ids {
        assert_eq!(
            attached(restyled, *id),
            attached(declared, *id),
            "{name}: Taffy attachment differs by path (restyled={}, declared={})",
            attached(restyled, *id),
            attached(declared, *id)
        );
        assert_eq!(
            ifc_root(restyled, *id),
            ifc_root(declared, *id),
            "{name}: ifc_root differs by path"
        );
        assert_eq!(
            geom(restyled, *id),
            geom(declared, *id),
            "{name}: box differs by path"
        );
    }
}

// ---------------------------------------------------------------------------
// Case B — the issue's own repro.
// ---------------------------------------------------------------------------

/// A bare `<button>` is `inline-block` by tag, so it is inline content and is
/// legitimately detached. `display: flex` makes it block-level and it must come
/// back. Its stale box is the inline-block shrink-to-fit width; the box it
/// earns is the panel's full width.
#[test]
fn case_b_a_button_restyled_to_flex_rejoins_its_parents_taffy_list() {
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(doc, body, "div", "display: block; width: 160px;");
        let btn = el(
            doc,
            panel,
            "button",
            if restyle { "" } else { "display: flex" },
        );
        let lbl = el(doc, btn, "span", "");
        txt(doc, lbl, "Edit");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(btn, "style", "display: flex");
        }
        doc.resolve_layout(VW, VH);
        vec![panel, btn, lbl]
    });
    let (panel, btn, lbl) = (NodeId(3), NodeId(4), NodeId(5));

    assert!(
        attached(&r, btn),
        "the restyled button has no Taffy parent at all"
    );
    assert_eq!(
        geom(&r, btn).2,
        160.0,
        "the restyled button kept its inline-block shrink-to-fit width"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("btn", btn), ("lbl", lbl)]);
}

/// Order, and the container's own height. One restyled child is the fixed point
/// where "insert it back" and "rebuild the list" agree; three is not. The
/// heights are declared (`height: 10px`), so 0-versus-30 is not a font question.
#[test]
fn three_restyled_buttons_come_back_in_document_order() {
    const BTN: &str = "display: flex; height: 10px";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(doc, body, "div", "width: 160px;");
        let mut out = vec![panel];
        for label in ["one", "two", "three"] {
            let b = el(doc, panel, "button", if restyle { "" } else { BTN });
            txt(doc, b, label);
            out.push(b);
        }
        if restyle {
            doc.resolve_layout(VW, VH);
            for b in out[1..].iter().copied() {
                doc.set_attribute(b, "style", BTN);
            }
        }
        doc.resolve_layout(VW, VH);
        out
    });
    let (panel, b1, b2, b3) = (NodeId(3), NodeId(4), NodeId(6), NodeId(8));

    assert_eq!(
        (
            taffy_index(&r, b1),
            taffy_index(&r, b2),
            taffy_index(&r, b3)
        ),
        (Some(0), Some(1), Some(2)),
        "the three restyled buttons are not back in document order"
    );
    assert_eq!(
        geom(&r, panel).3,
        30.0,
        "the panel's height collapsed — it is laying out none of its children"
    );
    assert_paths_agree(
        &r,
        &d,
        &[("panel", panel), ("b1", b1), ("b2", b2), ("b3", b3)],
    );
}

// ---------------------------------------------------------------------------
// The `<span>`/`<svg>` route — the parent blockifies its children.
// ---------------------------------------------------------------------------

/// The second route in the issue, and the source of six of #584's sixteen
/// `C orphan` lines: the *parent* becomes a flex container, which blockifies
/// its children from `Inline` to `Block`. Those children were never laid out as
/// boxes while they were inline, so their stale box is `0x0` and the container
/// collapses with them.
///
/// This route needs no `ifc_dirty` repair — `block → flex` is a real Taffy
/// display change — so it isolates the missing re-attach on its own.
#[test]
fn the_span_and_svg_route_a_parent_that_becomes_a_flex_container() {
    const FLEX: &str = "width: 160px; display: flex";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let wrap = el(
            doc,
            body,
            "div",
            if restyle { "width: 160px" } else { FLEX },
        );
        let sp = el(doc, wrap, "span", "height: 12px");
        txt(doc, sp, "Choose");
        let svg = el(doc, wrap, "svg", "width: 16px; height: 16px");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(wrap, "style", FLEX);
        }
        doc.resolve_layout(VW, VH);
        vec![wrap, sp, svg]
    });
    let (wrap, sp, svg) = (NodeId(3), NodeId(4), NodeId(6));

    assert!(
        attached(&r, sp),
        "the blockified <span> has no Taffy parent"
    );
    assert!(
        attached(&r, svg),
        "the blockified <svg> has no Taffy parent"
    );
    assert_eq!(
        geom(&r, wrap).3,
        16.0,
        "the flex container's height collapsed to its unlaid-out children"
    );
    assert_paths_agree(&r, &d, &[("wrap", wrap), ("sp", sp), ("svg", svg)]);
}

/// The third route, which the issue does not name: `display: inline` and
/// `display: block` both map to `taffy::Display::Block`, so a `<span>` restyled
/// straight to `block` never re-ran the IFC pass either. Its `ifc_root` stayed
/// stale, which is why invariant `C` said nothing about it.
#[test]
fn an_inline_span_restyled_to_block_rejoins() {
    const BLK: &str = "display: block; height: 14px";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(doc, body, "div", "width: 160px;");
        let sp = el(doc, panel, "span", if restyle { "" } else { BLK });
        txt(doc, sp, "Edit");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(sp, "style", BLK);
        }
        doc.resolve_layout(VW, VH);
        vec![panel, sp]
    });
    let (panel, sp) = (NodeId(3), NodeId(4));

    assert!(attached(&r, sp), "the restyled <span> has no Taffy parent");
    assert_eq!(
        ifc_root(&r, sp),
        None,
        "the restyled <span> still claims to be inline content of its parent"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("sp", sp)]);
}

// ---------------------------------------------------------------------------
// Off the "its Taffy list is its DOM parent's" fixed point.
// ---------------------------------------------------------------------------

/// A `display: contents` wrapper generates no box, so the button's Taffy id
/// lives in the *panel's* list, not the wrapper's — the shape every rsx
/// `if`/`match` emits. A repair that asks the DOM parent for the list to
/// rebuild hands the boxes to a Taffy node nothing lays out.
#[test]
fn a_button_behind_a_display_contents_wrapper_rejoins() {
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(doc, body, "div", "width: 160px;");
        let wrap = el(doc, panel, "div", "display: contents");
        let btn = el(
            doc,
            wrap,
            "button",
            if restyle {
                ""
            } else {
                "display: flex; height: 18px"
            },
        );
        txt(doc, btn, "Edit");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(btn, "style", "display: flex; height: 18px");
        }
        doc.resolve_layout(VW, VH);
        vec![panel, wrap, btn]
    });
    let (panel, btn) = (NodeId(3), NodeId(5));

    assert!(
        attached(&r, btn),
        "the restyled button behind a contents wrapper has no Taffy parent"
    );
    assert_eq!(
        doc_taffy_parent_dom(&r, btn),
        Some(panel.0),
        "the button's box went to the boxless wrapper instead of the panel"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("btn", btn)]);
}

/// Text on both sides, so the restyle turns the panel into mixed content and
/// anonymous block boxes are minted beside the button. The button's Taffy
/// siblings are then boxes that are not elements at all.
#[test]
fn a_button_between_two_text_runs_rejoins() {
    const BTN: &str = "display: flex; height: 18px";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(doc, body, "div", "width: 160px; line-height: 20px");
        txt(doc, panel, "before");
        let btn = el(doc, panel, "button", if restyle { "" } else { BTN });
        txt(doc, btn, "Edit");
        txt(doc, panel, "after");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(btn, "style", BTN);
        }
        doc.resolve_layout(VW, VH);
        vec![panel, btn]
    });
    let (panel, btn) = (NodeId(3), NodeId(5));

    assert!(
        attached(&r, btn),
        "the restyled button between two text runs has no Taffy parent"
    );
    assert_eq!(
        taffy_index(&r, btn),
        Some(1),
        "the button did not land between the two anonymous block boxes"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("btn", btn)]);
}

/// Back and forth. A repair that heals once — or that heals *after* the marking
/// pass, so the re-detach never happens again — passes a one-transition
/// fixture and fails this one. The final state is inline-level again, so the
/// button must be detached and marked, exactly like the bare twin.
#[test]
fn restyling_back_to_inline_level_detaches_again() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = el(&mut doc, body, "div", "width: 160px;");
    let btn = el(&mut doc, panel, "button", "");
    txt(&mut doc, btn, "Edit");
    doc.resolve_layout(VW, VH);

    doc.set_attribute(btn, "style", "display: flex");
    doc.resolve_layout(VW, VH);
    assert!(attached(&doc, btn), "round 1: the button never came back");

    doc.set_attribute(btn, "style", "display: inline-block");
    doc.resolve_layout(VW, VH);
    assert!(
        !attached(&doc, btn),
        "round 2: an inline-block child is still in its IFC root's Taffy list"
    );
    assert_eq!(
        ifc_root(&doc, btn),
        Some(panel.0),
        "round 2: the button is detached but no IFC claims it"
    );

    doc.set_attribute(btn, "style", "display: flex");
    doc.resolve_layout(VW, VH);
    assert!(attached(&doc, btn), "round 3: the button never came back");
    assert_clean(&doc, "after three transitions");
}

/// `position: absolute` is the other way out of inline content: Stylo
/// blockifies every out-of-flow box, so the button stops being inline-level
/// without its `display` ever being written. The marking pass leaves an
/// out-of-flow child attached on purpose (#289, #406) — so this one has to come
/// back too, or an absolute box has no route to the screen at all.
///
/// This route needs no `ifc_dirty` repair: Taffy's `position` field does change,
/// so the pass already ran. It isolates the heal's `OutOfFlow` arm.
#[test]
fn an_absolutely_positioned_button_rejoins() {
    const ABS: &str = "position: absolute; left: 10px; top: 20px; width: 30px; height: 40px";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(
            doc,
            body,
            "div",
            "position: relative; width: 160px; height: 100px",
        );
        let btn = el(doc, panel, "button", if restyle { "" } else { ABS });
        txt(doc, btn, "Edit");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(btn, "style", ABS);
        }
        doc.resolve_layout(VW, VH);
        vec![panel, btn]
    });
    let (panel, btn) = (NodeId(3), NodeId(4));

    assert!(
        attached(&r, btn),
        "the absolutely positioned button has no Taffy parent"
    );
    assert_eq!(
        geom(&r, btn),
        (10.0, 20.0, 30.0, 40.0),
        "the absolute box kept its inline-block geometry"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("btn", btn)]);
}

/// The issue's recorded "loose thread", and the half of it this work does
/// settle. `<span style="display: block">text<div/>tail</span>` restyled to
/// `display: inline` used to flag nothing while the identical end state written
/// as `<a>text<div/>tail</a>` was stranded — "same markup, same computed styles,
/// different outcome by path".
///
/// The path was the whole of it: `block` and `inline` both map to
/// `taffy::Display::Block`, so the restyle re-ran no IFC pass and the `<span>`
/// stayed a Taffy block child. rinch was laying out and painting the *previous*
/// style. With the crossing trigger in place both paths reach the same IFC
/// structure, which is what this fixture asserts.
///
/// **It is not asserted that they render the same, because they do not.** Both
/// now hit #513 — block-level content inside an inline element, and everything
/// after it, is dropped — and the restyled twin additionally keeps the stale
/// boxes its block layout left behind, which is why only the structural fields
/// are compared here. Converging on #513 is a *behaviour change*: that shape
/// used to render its block child by accident, and no longer does. See the PR.
#[test]
fn the_block_to_inline_path_now_agrees_with_the_static_twin() {
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let c = el(doc, body, "div", "width: 400px; line-height: 20px");
        // The declared twin uses `<a>`, whose UA display *is* inline, so it
        // reaches the final state with no `display` declaration at all.
        let sp = el(
            doc,
            c,
            if restyle { "span" } else { "a" },
            if restyle { "display: block" } else { "" },
        );
        txt(doc, sp, "text");
        let blk = el(doc, sp, "div", "width: 40px; height: 30px");
        txt(doc, sp, "tail");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(sp, "style", "display: inline");
        }
        doc.resolve_layout(VW, VH);
        vec![c, sp, blk]
    });
    let (c, sp) = (NodeId(3), NodeId(4));

    assert_eq!(
        attached(&r, sp),
        attached(&d, sp),
        "the wrapper's Taffy attachment still depends on the path"
    );
    assert_eq!(
        ifc_root(&r, sp),
        ifc_root(&d, sp),
        "the wrapper's ifc_root still depends on the path"
    );
    assert_eq!(
        geom(&r, c).3,
        geom(&d, c).3,
        "the container's height still depends on the path"
    );
    assert_clean(&r, "restyled");
    assert_clean(&d, "declared");
}

/// The shape that says **where** the heal has to run: the owner of a healed
/// node is a live IFC root, carrying an `InlineRoot` measure leaf.
///
/// `text + absolute` mints no anonymous block box (#406), so the container is
/// still an IFC root after the button goes out of flow — and a root whose only
/// attached child is out-of-flow gets a Taffy-only measure leaf at index 0,
/// because Taffy consults a measure function on a childless node only (#466).
/// Running the heal **after** the marking loop rebuilds that root's list from
/// the DOM, which puts the detached text back and throws the measure leaf away;
/// running it before leaves both passes to do their own work in order.
///
/// Kills: moving `reattach_departed_ifc_children` after the root loop.
#[test]
fn a_healed_out_of_flow_child_leaves_its_roots_measure_leaf_alone() {
    const ABS: &str = "position: absolute; left: 5px; top: 6px; width: 30px; height: 40px";
    let (r, d) = twins(|doc, restyle| {
        let body = doc.body();
        let panel = el(
            doc,
            body,
            "div",
            "position: relative; width: 160px; line-height: 20px",
        );
        txt(doc, panel, "hello");
        let btn = el(doc, panel, "button", if restyle { "" } else { ABS });
        txt(doc, btn, "Edit");
        if restyle {
            doc.resolve_layout(VW, VH);
            doc.set_attribute(btn, "style", ABS);
        }
        doc.resolve_layout(VW, VH);
        vec![panel, btn]
    });
    let (panel, text, btn) = (NodeId(3), NodeId(4), NodeId(5));

    assert!(
        r.tree.ifc_measure_leaves.contains_key(&panel.0),
        "the root lost its measure leaf, so its inline content is measured by nobody"
    );
    assert!(
        !attached(&r, text),
        "the root's inline text was re-attached as a Taffy child"
    );
    assert_paths_agree(&r, &d, &[("panel", panel), ("text", text), ("btn", btn)]);
}

// ---------------------------------------------------------------------------
// Guard rails. These pass before and after; they are here so an over-reaching
// repair is caught, not as coverage of the defect.
// ---------------------------------------------------------------------------

/// The bare row of the issue's table: inline content **should** be detached,
/// and a repair that re-attaches everything it ever detached breaks the IFC.
#[test]
fn a_bare_button_stays_legitimately_detached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = el(&mut doc, body, "div", "width: 160px;");
    let btn = el(&mut doc, panel, "button", "");
    txt(&mut doc, btn, "Edit");
    doc.resolve_layout(VW, VH);

    assert!(!attached(&doc, btn), "a bare button is inline content");
    assert_eq!(ifc_root(&doc, btn), Some(panel.0));
    assert_clean(&doc, "bare");
}

/// A `display: none` child of an IFC root is detached too, and for a different
/// reason — it generates no box and one attached child would make the root's
/// measure structurally unreachable (#466, #487). It must **stay** detached
/// across an IFC pass triggered by a sibling.
///
/// Kills: a repair whose heal gate reads "was detached" instead of "is no
/// longer detachable", which re-attaches the hidden child and puts a Taffy
/// child back under an `InlineRoot` carrier.
#[test]
fn a_hidden_child_of_an_ifc_root_stays_detached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = el(&mut doc, body, "div", "width: 160px; line-height: 20px");
    txt(&mut doc, panel, "visible text");
    let hidden = el(&mut doc, panel, "div", "display: none; height: 40px");
    let sib = el(&mut doc, body, "span", "");
    txt(&mut doc, sib, "sibling");
    doc.resolve_layout(VW, VH);
    assert!(!attached(&doc, hidden), "a hidden child generates no box");

    // Any restyle that re-runs the IFC pass; the hidden child is untouched.
    doc.set_attribute(sib, "style", "display: block");
    doc.resolve_layout(VW, VH);

    assert!(
        !attached(&doc, hidden),
        "the hidden child was re-attached to its IFC root"
    );
    assert_eq!(
        geom(&doc, panel).3,
        20.0,
        "the hidden child took part in its parent's height"
    );
    assert_clean(&doc, "after a sibling restyle");
}

/// The **mirror** case, which this work does not fix: a wrapper restyled
/// `flex → inline` strands its block child (#513's static block-in-inline
/// defect, reached by a transition). Guard rail only — it passes before and
/// after, and exists so a repair that reaches into the mirror direction is
/// caught rather than shipped unmeasured.
#[test]
fn the_mirror_case_gains_no_new_taffy_violation() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = el(&mut doc, body, "div", "display: flex; width: 160px;");
    let kid = el(&mut doc, wrap, "div", "width: 40px; height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(attached(&doc, kid));

    doc.set_attribute(wrap, "style", "display: inline; width: 160px;");
    doc.resolve_layout(VW, VH);
    assert_clean(&doc, "mirror case");
}

/// The DOM node whose Taffy node holds `id`'s Taffy node.
fn doc_taffy_parent_dom(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    let t = doc.tree.get(id.0).and_then(|n| n.taffy_id)?;
    let p = doc.tree.taffy.parent(t)?;
    doc.tree.taffy_map.get(&p).copied()
}
