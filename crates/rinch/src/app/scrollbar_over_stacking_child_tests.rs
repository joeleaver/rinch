//! A press on a scroller's scrollbar is the scroller's even when the child
//! under the bar is positioned or a stacking context.
//!
//! The shape (found in Pimble's note pane): an `overflow-y: auto;
//! position: relative; z-index: 0` scroller whose in-flow child is the
//! `Editor` — which the editor sheet makes `position: relative; z-index: 0` —
//! as wide as the scroller with a `padding-right`, so the child covers the
//! strip the overlay thumb is drawn in. Two empty siblings beside it. A press
//! on the thumb did not grab it; it went into the editor and placed the caret.
//! Chrome gives a press on a scroller's scrollbar to the scrollbar whatever is
//! under it, positioned, z-indexed or not. #2549 (an editor child) and #2613
//! (a draggable child) covered children that are neither.
#![cfg(feature = "windowing")]
use super::*;
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);
/// The scroller's border box, off the origin on both axes.
const LEFT: f32 = 61.0;
const TOP: f32 = 13.0;
const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 200.0;

const SCROLLER_STYLE: &str = "position: relative; z-index: 0; overflow-y: auto; \
     width: 400px; height: 200px";

/// What sits in the scroller.
#[derive(Clone, Copy, PartialEq)]
enum Child {
    /// The `Editor`, styled as Pimble styles it; the sheet adds
    /// `position: relative; z-index: 0`.
    Editor,
    /// A plain tall `div` with `position: relative; z-index: 3` and a click
    /// handler.
    PositionedDiv,
    /// An inner `overflow-y: auto; position: relative; z-index: 1` scroller,
    /// 300px wide, holding a positioned tall `div`, inside a full-width
    /// `position: relative; z-index: 0` wrapper that covers the outer bar.
    Nested,
    /// The editor as Pimble's pane has it: a flex item of a column-flex
    /// scroller, itself a flex item of an `overflow: hidden` column.
    FlexEditor,
    /// Pimble's whole chain: the flex editor's scroller in an absolute,
    /// `overflow: hidden` pane, inside a `display: contents` slot wrapper
    /// (a 0x0 box), inside a `position: relative; overflow: hidden` parent.
    ContentsChain,
}

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    scroller: u64,
    inner: Option<u64>,
    clicks: Rc<Cell<u32>>,
}

/// The outer scroller, and the inner one when there is one.
type Ids = (u64, Option<u64>);

fn rig(child: Child) -> Rig {
    let ids: Rc<Cell<Option<Ids>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let clicks = Rc::new(Cell::new(0u32));
    let clicks_in = clicks.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..60)
        .map(|i| format!("<p>paragraph {i} of the note</p>"))
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "box-sizing: border-box; width: 800px; height: 600px; padding: 13px 0 0 61px",
        );
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", SCROLLER_STYLE);
        let mut inner = None;
        let mut outer = None;
        let mut chain_pane: Option<NodeHandle> = None;
        match child {
            Child::Editor => {
                let editor = handle_in.mount(scope);
                editor.set_attribute("style", "width: 100%; padding-right: 34px");
                scroller.append_child(&editor);
            }
            Child::PositionedDiv => {
                let tall = scope.create_element("div");
                tall.set_attribute(
                    "style",
                    "position: relative; z-index: 3; height: 1400px; background: #eef",
                );
                let hid = scope.register_handler({
                    let clicks = clicks_in.clone();
                    move || clicks.set(clicks.get() + 1)
                });
                tall.set_attribute("data-rid", &hid.0.to_string());
                scroller.append_child(&tall);
            }
            Child::FlexEditor => {
                let pane = scope.create_element("div");
                pane.set_attribute(
                    "style",
                    "display: flex; flex-direction: column; overflow: hidden; width: 400px; \
                     height: 200px",
                );
                scroller.set_attribute(
                    "style",
                    "flex: 1; display: flex; flex-direction: column; overflow-y: auto; \
                     position: relative; z-index: 0",
                );
                let editor = handle_in.mount(scope);
                editor.set_attribute(
                    "style",
                    "flex: 1 1 auto; border: none; border-radius: 0; padding-right: 34px",
                );
                scroller.append_child(&editor);
                outer = Some(pane);
            }
            Child::ContentsChain => {
                let panes = scope.create_element("div");
                panes.set_attribute(
                    "style",
                    "position: relative; overflow: hidden; width: 400px; height: 200px",
                );
                let slot = scope.create_element("div");
                slot.set_attribute("style", "display: contents");
                let pane = scope.create_element("div");
                pane.set_attribute(
                    "style",
                    "position: absolute; left: 0px; top: 0px; width: 100%; height: 100%; \
                     overflow: hidden; display: flex; flex-direction: column",
                );
                scroller.set_attribute(
                    "style",
                    "flex: 1; display: flex; flex-direction: column; overflow-y: auto; \
                     position: relative; z-index: 0",
                );
                let editor = handle_in.mount(scope);
                editor.set_attribute(
                    "style",
                    "flex: 1 1 auto; border: none; border-radius: 0; padding-right: 34px",
                );
                scroller.append_child(&editor);
                slot.append_child(&pane);
                panes.append_child(&slot);
                // `pane` takes the scroller below; `outer` holds the top.
                outer = Some(panes);
                chain_pane = Some(pane);
            }
            Child::Nested => {
                let wrapper = scope.create_element("div");
                wrapper.set_attribute(
                    "style",
                    "position: relative; z-index: 0; height: 1400px; padding-top: 160px",
                );
                let inner_scroller = scope.create_element("div");
                inner_scroller.set_attribute(
                    "style",
                    "position: relative; z-index: 1; overflow-y: auto; width: 300px; \
                     height: 150px",
                );
                let inner_tall = scope.create_element("div");
                inner_tall.set_attribute("style", "position: relative; z-index: 2; height: 900px");
                inner_scroller.append_child(&inner_tall);
                wrapper.append_child(&inner_scroller);
                scroller.append_child(&wrapper);
                inner = Some(inner_scroller.node_id().0);
            }
        }
        // The two empty siblings Pimble's pane has.
        for _ in 0..2 {
            scroller.append_child(&scope.create_element("div"));
        }
        match outer {
            Some(top) => {
                chain_pane.as_ref().unwrap_or(&top).append_child(&scroller);
                root.append_child(&top);
            }
            None => root.append_child(&scroller),
        }
        ids_in.set(Some((scroller.node_id().0, inner)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (scroller, inner) = ids.get().unwrap();
    // Scroll part-way, so a thumb press that jumped would show.
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: LEFT + 100.0,
            y: TOP + 100.0,
            delta_x: 0.0,
            delta_y: -120.0,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
    handle.set_selection(Selection::cursor(Pos(3)));
    Rig {
        app,
        handle,
        scroller,
        inner,
        clicks,
    }
}

fn scroll(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn track(app: &RinchApp, id: u64) -> rinch_dom::paint::scrollbar::ScrollbarTrack {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
        .vertical
        .expect("a vertical bar")
}

/// The window point `along` px down the outer bar, centred on its thickness.
fn on_bar(along: f64) -> (f32, f32) {
    (
        LEFT + WIDTH - (MARGIN + THICKNESS / 2.0) as f32,
        TOP + along as f32,
    )
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

fn release(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

/// Grab the outer thumb in its middle, drag it 60px down, release: the
/// scroller scrolls, by the drag's share of the range, and nothing under the
/// bar saw the press.
fn thumb_drag_scrolls(child: Child) {
    let mut r = rig(child);
    let start = scroll(&r.app, r.scroller);
    assert!(
        start > 0.0,
        "premise: the wheel scrolled the scroller part-way"
    );
    let t = track(&r.app, r.scroller);
    let grab = on_bar(t.thumb_start(start) + t.thumb_len / 2.0);
    let caret = r.handle.selection();

    press(&mut r.app, grab);
    let drag = r.app.scrollbar_drag.as_ref().map(|d| d.node_id);
    assert_eq!(
        drag,
        Some(r.scroller),
        "the press grabs the scroller's thumb"
    );
    assert_eq!(
        scroll(&r.app, r.scroller),
        start,
        "a grab moves nothing by itself"
    );

    let to = (grab.0, grab.1 + 60.0);
    move_to(&mut r.app, to);
    r.app.resolve_and_repaint(800.0, 600.0);
    let moved = scroll(&r.app, r.scroller);
    let expected = t.scroll_for_drag(start, 60.0);
    assert!(
        (moved - expected).abs() <= 1.0,
        "the drag scrolled by its share of the range: {start} -> {moved}, expected {expected}"
    );
    release(&mut r.app, to);
    assert!(r.app.scrollbar_drag.is_none(), "the release ends the drag");
    assert_eq!(
        r.handle.selection(),
        caret,
        "no caret was placed in the text"
    );
    assert_eq!(r.clicks.get(), 0, "the child under the bar got no click");
}

/// A press on the outer track below the thumb jumps toward it.
fn track_press_jumps(child: Child) {
    let mut r = rig(child);
    let start = scroll(&r.app, r.scroller);
    let t = track(&r.app, r.scroller);
    let below = t.thumb_start(start) + t.thumb_len + 30.0;
    assert!(
        below < t.track_start + t.track_len,
        "premise: there is track below the thumb"
    );
    let caret = r.handle.selection();

    press(&mut r.app, on_bar(below));
    release(&mut r.app, on_bar(below));
    r.app.resolve_and_repaint(800.0, 600.0);
    assert!(
        scroll(&r.app, r.scroller) > start,
        "the track press scrolled toward it"
    );
    assert_eq!(
        r.handle.selection(),
        caret,
        "no caret was placed in the text"
    );
    assert_eq!(r.clicks.get(), 0, "the child under the bar got no click");
}

#[test]
fn the_thumb_over_a_stacking_editor_child_is_grabbed_and_dragged() {
    thumb_drag_scrolls(Child::Editor);
}

#[test]
fn a_track_press_over_a_stacking_editor_child_jumps() {
    track_press_jumps(Child::Editor);
}

#[test]
fn the_thumb_over_a_flex_editor_child_is_grabbed_and_dragged() {
    thumb_drag_scrolls(Child::FlexEditor);
}

#[test]
fn a_track_press_over_a_flex_editor_child_jumps() {
    track_press_jumps(Child::FlexEditor);
}

#[test]
fn the_thumb_of_a_scroller_under_a_display_contents_slot_is_grabbed_and_dragged() {
    thumb_drag_scrolls(Child::ContentsChain);
}

#[test]
fn a_track_press_under_a_display_contents_slot_jumps() {
    track_press_jumps(Child::ContentsChain);
}

#[test]
fn a_press_beside_the_strip_under_a_display_contents_slot_reaches_the_editor() {
    let mut r = rig(Child::ContentsChain);
    let caret = r.handle.selection();
    let x = LEFT + WIDTH - rinch_dom::paint::scrollbar::HIT_THICKNESS as f32 - 2.0;
    press(&mut r.app, (x, TOP + 100.0));
    release(&mut r.app, (x, TOP + 100.0));
    assert!(r.app.scrollbar_drag.is_none(), "control: beside the strip");
    assert_ne!(r.handle.selection(), caret, "control: the caret moved");
}

#[test]
fn the_thumb_over_a_positioned_div_child_is_grabbed_and_dragged() {
    thumb_drag_scrolls(Child::PositionedDiv);
}

#[test]
fn a_track_press_over_a_positioned_div_child_jumps() {
    track_press_jumps(Child::PositionedDiv);
}

#[test]
fn the_outer_thumb_over_a_stacking_wrapper_of_a_nested_scroller_is_grabbed() {
    thumb_drag_scrolls(Child::Nested);
}

#[test]
fn a_track_press_on_the_outer_bar_over_a_nested_scroller_jumps() {
    track_press_jumps(Child::Nested);
}

/// The inner scroller's own bar is still the inner scroller's: the press
/// lands on the deepest bar under it, not the outer one.
#[test]
fn the_inner_scrollers_thumb_is_the_inner_scrollers() {
    let mut r = rig(Child::Nested);
    let inner = r.inner.unwrap();
    let outer_start = scroll(&r.app, r.scroller);
    let inner_start = scroll(&r.app, inner);
    let t = track(&r.app, inner);
    // The inner scroller's border box: 300 wide at the outer's content
    // origin, 160px down the wrapper, moved up by the outer's scroll.
    let x = LEFT + 300.0 - (MARGIN + THICKNESS / 2.0) as f32;
    let y =
        TOP + 160.0 - outer_start as f32 + (t.thumb_start(inner_start) + t.thumb_len / 2.0) as f32;
    assert!(
        y > TOP && y < TOP + HEIGHT,
        "premise: the inner thumb is in view at {y}"
    );
    press(&mut r.app, (x, y));
    let drag = r.app.scrollbar_drag.as_ref().map(|d| d.node_id);
    assert_eq!(
        drag,
        Some(inner),
        "the press grabs the inner scroller's thumb"
    );
    move_to(&mut r.app, (x, y + 30.0));
    r.app.resolve_and_repaint(800.0, 600.0);
    release(&mut r.app, (x, y + 30.0));
    assert!(
        scroll(&r.app, inner) > inner_start,
        "the inner scroller scrolled"
    );
    assert_eq!(scroll(&r.app, r.scroller), outer_start, "the outer did not");
}

/// Just left of the hit strip the press is the child's: the editor places the
/// caret, the positioned div gets its click.
#[test]
fn a_press_just_outside_the_strip_still_reaches_the_editor() {
    let mut r = rig(Child::Editor);
    let caret = r.handle.selection();
    let x = LEFT + WIDTH - rinch_dom::paint::scrollbar::HIT_THICKNESS as f32 - 2.0;
    press(&mut r.app, (x, TOP + 100.0));
    release(&mut r.app, (x, TOP + 100.0));
    assert!(
        r.app.scrollbar_drag.is_none(),
        "control: left of the strip is not the scrollbar"
    );
    assert_ne!(
        r.handle.selection(),
        caret,
        "control: a press beside the strip moves the caret"
    );
}

#[test]
fn a_press_just_outside_the_strip_still_reaches_a_positioned_div() {
    let mut r = rig(Child::PositionedDiv);
    let x = LEFT + WIDTH - rinch_dom::paint::scrollbar::HIT_THICKNESS as f32 - 2.0;
    press(&mut r.app, (x, TOP + 100.0));
    release(&mut r.app, (x, TOP + 100.0));
    assert!(
        r.app.scrollbar_drag.is_none(),
        "control: left of the strip is not the scrollbar"
    );
    assert_eq!(r.clicks.get(), 1, "control: the div got the click");
}

/// A scroller standing past its parent's box: `parent` is the parent's style,
/// the scroller is 400x200 with a tall positioned child. Answers the app and
/// the scroller.
fn overflowing_rig(parent: &'static str) -> (RinchApp, u64) {
    overflowing_rig_with(parent, SCROLLER_STYLE)
}

fn overflowing_rig_with(parent: &'static str, scroller_style: &'static str) -> (RinchApp, u64) {
    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "box-sizing: border-box; width: 800px; height: 600px; padding: 13px 0 0 61px",
        );
        let narrow = scope.create_element("div");
        narrow.set_attribute("style", parent);
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", scroller_style);
        let tall = scope.create_element("div");
        tall.set_attribute("style", "position: relative; z-index: 0; height: 1400px");
        scroller.append_child(&tall);
        narrow.append_child(&scroller);
        root.append_child(&narrow);
        ids_in.set(Some(scroller.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = ids.get().unwrap();
    (app, id)
}

fn bar_at(app: &RinchApp, (x, y): (f32, f32)) -> Option<u64> {
    let d = app.doc.as_ref().unwrap().borrow();
    super::hit_testing::find_scrollbar_hit(&d.tree, x, y).map(|h| h.node_id)
}

/// The scroller is wider than its parent, so its bar is drawn right of the
/// parent's box: the thumb is grabbed and dragged there, and a track press
/// there jumps.
#[test]
fn the_bar_of_a_scroller_wider_than_its_parent_is_grabbed() {
    let (mut app, id) = overflowing_rig("width: 300px; height: 200px");
    let t = track(&app, id);
    let grab = on_bar(t.thumb_start(0.0) + t.thumb_len / 2.0);
    assert!(
        grab.0 > LEFT + 300.0,
        "premise: the bar is right of the parent's box"
    );
    assert_eq!(
        bar_at(&app, grab),
        Some(id),
        "the strip there is the scroller's"
    );
    press(&mut app, grab);
    assert_eq!(app.scrollbar_drag.as_ref().map(|d| d.node_id), Some(id));
    move_to(&mut app, (grab.0, grab.1 + 40.0));
    release(&mut app, (grab.0, grab.1 + 40.0));
    app.resolve_and_repaint(800.0, 600.0);
    let expected = t.scroll_for_drag(0.0, 40.0);
    assert!(
        (scroll(&app, id) - expected).abs() <= 1.0,
        "the drag scrolled: {} vs {expected}",
        scroll(&app, id)
    );

    let below = t.thumb_start(scroll(&app, id)) + t.thumb_len + 30.0;
    let before = scroll(&app, id);
    press(&mut app, on_bar(below));
    release(&mut app, on_bar(below));
    app.resolve_and_repaint(800.0, 600.0);
    assert!(
        scroll(&app, id) > before,
        "the track press right of the parent jumps"
    );
}

/// The scroller is taller than its parent: the lower part of its bar, below
/// the parent's box, is the scroller's too.
#[test]
fn the_bar_of_a_scroller_taller_than_its_parent_is_grabbed_below_it() {
    let (app, id) = overflowing_rig("width: 400px; height: 90px");
    let p = on_bar(150.0);
    assert!(p.1 > TOP + 90.0, "premise: below the parent's box");
    assert_eq!(bar_at(&app, p), Some(id));
}

/// Control: where an ancestor clips, the bar past its clip is not painted and
/// not grabbed.
#[test]
fn a_bar_clipped_by_an_ancestor_is_not_grabbed_past_the_clip() {
    let (app, id) = overflowing_rig("width: 300px; height: 200px; overflow: hidden");
    let t = track(&app, id);
    assert_eq!(bar_at(&app, on_bar(t.thumb_start(0.0) + 5.0)), None);
}

/// An absolutely positioned scroller standing outside its small positioned
/// parent: the parent's flow extent leaves it out, and its bar is still the
/// scroller's.
#[test]
fn the_bar_of_an_absolute_scroller_outside_its_parent_is_grabbed() {
    let (app, id) = overflowing_rig_with(
        "position: relative; width: 50px; height: 40px",
        "position: absolute; left: 200px; top: 0px; z-index: 0; overflow-y: auto; \
         width: 400px; height: 200px",
    );
    let x = LEFT + 200.0 + WIDTH - (MARGIN + THICKNESS / 2.0) as f32;
    let t = track(&app, id);
    let y = TOP + (t.thumb_start(0.0) + t.thumb_len / 2.0) as f32;
    assert_eq!(bar_at(&app, (x, y)), Some(id));
}

/// The wheel's geometric fallback (the hit is an overlay in another branch,
/// nothing on its own chain overflows) shares the scrollbar search's gate: a
/// scroller under a `display: contents` slot is found through the slot's
/// 0x0 box.
#[test]
fn the_wheels_fallback_finds_a_scroller_under_a_display_contents_slot() {
    let ids: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "box-sizing: border-box; width: 800px; height: 600px; padding: 13px 0 0 61px",
        );
        let panes = scope.create_element("div");
        panes.set_attribute(
            "style",
            "position: relative; overflow: hidden; width: 400px; height: 200px",
        );
        let slot = scope.create_element("div");
        slot.set_attribute("style", "display: contents");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 100%; height: 100%; \
             overflow-y: auto",
        );
        let tall = scope.create_element("div");
        tall.set_attribute("style", "height: 1400px");
        scroller.append_child(&tall);
        slot.append_child(&scroller);
        // An overlay over the scroller, in another branch, scrolling nothing.
        let overlay = scope.create_element("div");
        overlay.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 200px; height: 100px",
        );
        panes.append_child(&slot);
        panes.append_child(&overlay);
        root.append_child(&panes);
        ids_in.set(Some(scroller.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = ids.get().unwrap();
    {
        let d = app.doc.as_ref().unwrap().borrow();
        assert_eq!(
            super::hit_testing::find_scroll_container_at_point(&d.tree, LEFT + 50.0, TOP + 50.0),
            Some(id)
        );
    }
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: LEFT + 50.0,
            y: TOP + 50.0,
            delta_x: 0.0,
            delta_y: -120.0,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
    assert!(
        scroll(&app, id) > 0.0,
        "the wheel over the overlay scrolled it"
    );
}
