//! Performance regression scenarios for the rich-text editor: what one
//! keystroke, one caret move and one toolbar command cost in a 30-paragraph
//! document, and what a focused editor costs while idle.
//!
//! Same contract and harness as `perf_regression_tests.rs` (`perf_expect`):
//! whole frames, every non-timing counter exact, unlisted counters `0`, each
//! scenario with a positive control that it did what it names.
//!
//! The editor's own model work (transactions, the `ViewDesc` diff) has no
//! counter; what is pinned is what it costs the document — the cascades, the
//! Taffy computes, the Parley shapes, the repainted area and the effect runs —
//! which is where a regression in the view shows up.

use super::perf_expect::*;
use super::*;
use rinch_dom::perf::Counter::*;
use rinch_editor_core::{Pos, Selection};

const PARAGRAPHS: usize = 30;

/// "Paragraph number NN" is 19 characters, so paragraph `i`'s text starts at
/// `1 + 21 * i` (one for the doc's opening, two per paragraph boundary).
fn para_start(i: usize) -> usize {
    1 + 21 * i
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// A page: a Bold button (`data-nofocus`, as an editor
/// toolbar is) above one editor holding 30 paragraphs, focused by a real press
/// in the first paragraph and settled.
fn page() -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        let mut html = String::new();
        for i in 0..PARAGRAPHS {
            html.push_str(&format!("<p>Paragraph number {i:02}</p>"));
        }
        handle.load_html(&html);

        let bar = scope.create_element("div");
        bar.set_attribute("style", "height: 30px; display: flex");
        bar.set_attribute("data-nofocus", "");
        let bold = scope.create_element("button");
        bold.set_attribute(
            "style",
            "width: 60px; height: 30px; font-family: sans-serif; font-size: 14px; line-height: 20px",
        );
        let h = handle.clone();
        let id = scope.register_handler(move || {
            h.command("toggleBold");
        });
        bold.set_attribute("data-rid", &id.0.to_string());
        let label = scope.create_text("B");
        bold.append_child(&label);
        bar.append_child(&bold);
        root.append_child(&bar);

        container.set_attribute(
            "style",
            "width: 600px; height: 500px; overflow-y: auto; font-size: 16px; \
             line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let mut page = Page { app, handle };
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(4))
        .expect("the first paragraph has a caret");
    interaction(&mut page.app, |app| click(app, x + 1.0, y + h / 2.0));
    assert!(
        matches!(page.app.focus_target, FocusTarget::Editor(_)),
        "precondition: the editor holds the keyboard"
    );
    settle(&mut page.app);
    page
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, SIZE, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, SIZE, 1.0);
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        SIZE,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        SIZE,
        1.0,
    );
}

/// Put the caret at `pos` outside the measured frame.
fn caret_at(page: &mut Page, pos: usize) {
    page.handle.set_selection(Selection::cursor(Pos(pos)));
    settle(&mut page.app);
}

/// The document's size in positions; typing one character grows it by one.
fn doc_size(page: &Page) -> usize {
    page.handle.doc().content_size()
}

/// A focused editor with its caret blink left out (the blink is the runtime's
/// timed wake, driven by the clock, not by `AboutToWait`): no frame.
#[test]
fn a_focused_editor_idles_for_free_between_blinks() {
    let mut page = page();
    let (redraws, total) = idle_turns(&mut page.app, 20);
    assert_eq!(redraws, 0);
    expect_frame("editor focused, idle", &total, &[]);
}

/// One caret blink: the caret's `display` toggles, and that one small box is
/// repainted (`repainted_px` 320). This is the cost a focused editor pays
/// twice a second.
///
/// **Findings, pinned as they are — #907; a fix must LOWER this number, and its PR updates the pin.** The toggle is a style write, so the blink
/// is a cascade (with a `::before`/`::after` pass: the editor's sheet has such
/// rules) and a Taffy sync, though it takes the paint-only path. Paint builds
/// three stacking sequences and visits 37 nodes for those 320 pixels. And the
/// software painter fills **607 320** clip-mask pixels to repaint 320: one of
/// the two clips is the editor's own `overflow-y: auto` clip, filled over
/// bounds larger than the editor's visible 600x500 box rather than over the
/// damage (every editor scenario below shows the same ~607k, a single
/// keystroke included).
#[test]
fn one_caret_blink_repaints_the_caret() {
    let mut page = page();
    let redraw = page.handle.set_caret_blink(false);
    assert_eq!(redraw, Some(true), "positive control: the caret toggled");
    let s = paint(&mut page.app);
    expect_frame(
        "editor: one caret blink",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (PseudoElementPasses, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 320),
            (SurfacePx, 480000),
            (PaintNodesVisited, 37),
            (StackingOrderBuilds, 3),
            (GlyphCacheHits, 19),
            (ClipMasks, 2),
            (ClipMaskPx, 607320),
            (PaintSurfaceAllocs, 1),
        ],
    );
}

/// Typing one character into a paragraph: one paragraph re-shaped, one region
/// repainted.
///
/// **Finding, pinned as it is — #906; a fix must LOWER this number, and its PR updates the pin:** a keystroke lays out **twice**
/// (`layout_resolves` 2, `taffy_root_computes` 2). The likely second is the
/// caret overlay, repositioned after the first layout — its one Taffy style
/// change is the same one `arrow_right` shows — but that is read off the
/// counts, not traced.
#[test]
fn typing_one_character() {
    let mut page = page();
    let before = doc_size(&page);
    let s = interaction(&mut page.app, |app| key(app, KeyCode::KeyX, Some("x")));
    assert_eq!(doc_size(&page), before + 1, "positive control");
    expect_frame(
        "editor: type one character",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 2),
            (StyleNodesVisited, 2),
            (PseudoElementPasses, 2),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 2),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (LayoutResolves, 2),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 2),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 18678),
            (SurfacePx, 480000),
            (PaintNodesVisited, 37),
            (StackingOrderBuilds, 3),
            (GlyphCacheHits, 19),
            (GlyphCacheMisses, 1),
            (ClipMasks, 2),
            (ClipMaskPx, 627906),
            (PaintSurfaceAllocs, 1),
        ],
    );
}

/// ArrowRight: a selection-only change. Nothing is re-shaped, and the repaint
/// is the caret's old and new rects (608 px).
///
/// **Finding, pinned as it is — #906; a fix must LOWER this number, and its PR updates the pin:** moving the caret is a Taffy style change on
/// the caret overlay (`taffy_style_changes` 1), so a selection-only key runs a
/// **root Taffy compute** over the whole document.
#[test]
fn arrow_right() {
    let mut page = page();
    let from = page.handle.selection();
    let s = interaction(&mut page.app, |app| key(app, KeyCode::ArrowRight, None));
    assert_ne!(page.handle.selection(), from, "positive control");
    expect_frame(
        "editor: ArrowRight",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (PseudoElementPasses, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 608),
            (SurfacePx, 480000),
            (PaintNodesVisited, 37),
            (StackingOrderBuilds, 3),
            (GlyphCacheHits, 19),
            (ClipMasks, 2),
            (ClipMaskPx, 607644),
            (PaintSurfaceAllocs, 1),
        ],
    );
}

/// Enter in the middle of a paragraph: one block becomes two.
///
/// **Finding, pinned as it is — #905; a fix must LOWER this number, and its PR updates the pin:** every paragraph is re-shaped
/// (`shape_measure_ifc` 31), not the two the split touched. **Not the
/// structural pass's signatures:** with the scoped pass (#895) only the one
/// split block's signature moves (`ifc_signature_changes` 1, was 31 when every
/// structural pass re-signed the whole document), and the 31 shapes are
/// unchanged — so the drops come in through `ifc_measure_invalidations` (92,
/// about three per block), whose source is not traced here. Two
/// layouts, as for a keystroke. The full repaint is legitimate: every block
/// below the caret moves, which is more than half the window.
#[test]
fn enter_splits_a_paragraph() {
    let mut page = page();
    let blocks = page.handle.doc().child_count();
    let s = interaction(&mut page.app, |app| key(app, KeyCode::Enter, None));
    assert_eq!(
        page.handle.doc().child_count(),
        blocks + 1,
        "positive control"
    );
    expect_frame(
        "editor: Enter",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 3),
            (PseudoElementPasses, 3),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 31),
            (ShapeIfcBuild, 31),
            (IfcMeasureInvalidations, 92),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 34),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 32),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullRegionTooLarge, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 38),
            (StackingOrderBuilds, 3),
            (GlyphCacheHits, 324),
            (ClipMasks, 1),
            (ClipMaskPx, 303408),
        ],
    );
}

/// Backspace at the start of the second paragraph joins it to the first.
///
/// **Finding, pinned as it is — #905; a fix must LOWER this number, and its PR updates the pin:** the mirror of `enter_splits_a_paragraph` —
/// the blocks after the join are re-shaped with it (`shape_measure_ifc` 29), and
/// as there, not through signatures (`ifc_signature_changes` 0 since #895, was
/// 29) but through `ifc_measure_invalidations` (88).
#[test]
fn backspace_joins_two_paragraphs() {
    let mut page = page();
    caret_at(&mut page, para_start(1));
    let blocks = page.handle.doc().child_count();
    let s = interaction(&mut page.app, |app| key(app, KeyCode::Backspace, None));
    assert_eq!(
        page.handle.doc().child_count(),
        blocks - 1,
        "positive control"
    );
    expect_frame(
        "editor: Backspace across a block boundary",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 2),
            (StyleNodesVisited, 2),
            (PseudoElementPasses, 2),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 2),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 29),
            (ShapeIfcBuild, 29),
            (IfcMeasureInvalidations, 88),
            (LayoutResolves, 2),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 1),
            (IfcScopeNodes, 31),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 30),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullRegionTooLarge, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 36),
            (StackingOrderBuilds, 3),
            (GlyphCacheHits, 362),
            (ClipMasks, 1),
            (ClipMaskPx, 303408),
        ],
    );
}

/// The toolbar's Bold button over a selected word: a real click on a
/// `data-nofocus` button, then the command.
///
/// **Findings, pinned as they are — #908; a fix must LOWER this number, and its PR updates the pin:** one click runs **ten** hit tests, two
/// whole-document IFC setup passes and two layouts, and builds six stacking
/// sequences. The bold mark re-shapes only its own paragraph.
#[test]
fn a_toolbar_bold_over_a_word() {
    let mut page = page();
    page.handle
        .set_selection(Selection::text(Pos(para_start(0)), Pos(para_start(0) + 9)));
    settle(&mut page.app);
    let s = interaction(&mut page.app, |app| click(app, 20.0, 15.0));
    assert!(page.handle.is_mark_active("bold"), "positive control");
    assert!(
        matches!(page.app.focus_target, FocusTarget::Editor(_)),
        "the toolbar did not take the keyboard"
    );
    expect_frame(
        "editor: toolbar toggleBold",
        &s,
        &[
            (StyleResolves, 4),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 4),
            (PseudoElementPasses, 4),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 5),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (IfcSetupPasses, 2),
            (IfcScopedPasses, 2),
            (IfcScopeContainers, 3),
            (IfcScopeNodes, 37),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 2),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullRegionTooLarge, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 38),
            (StackingOrderBuilds, 6),
            (GlyphCacheHits, 343),
            (ClipMasks, 1),
            (ClipMaskPx, 303408),
            (HitTests, 10),
            (HitTestNodesVisited, 50),
            (HitExtentsComputed, 39),
        ],
    );
}
