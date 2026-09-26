//! Desktop's half of #1025: an inline leaf (hard break, image) in a textblock.
//!
//! Found while checking whether desktop shares the web's leaf ambiguity. It
//! has worse: `is_editor_textblock` reads a paragraph holding a leaf as a
//! container (a leaf carries `data-pm-type`), so no press lands in it at all;
//! and desktop's IFC gives a `<br>` one byte (`"\n"`) where the view's flat
//! byte map gives it none, so every caret after a break is drawn one character
//! early. Both fixed by #1099: a leaf child no longer makes its paragraph a
//! container, and the view's flat byte map gives a `<br>` the byte the host's
//! IFC gives it (`DomDocument::line_break_flat_bytes`).

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One paragraph in a 400px editor, Inter 16px / 24px, focused and settled.
fn page(html: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    fn block(&self) -> usize {
        self.handle.caret_address(Pos(1)).unwrap().0
    }
    fn block_box(&self) -> (f32, f32, f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, self.block())
    }
    fn click(&mut self, x: f32, y: f32) {
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        idle(&mut self.app);
    }
    /// The head as a char offset into the paragraph.
    fn head(&self) -> usize {
        self.handle.selection().head().0 - 1
    }
    /// The layout-local caret for char offset `i`, through the model's caret
    /// address — what the caret overlay is placed from.
    fn local(&self, i: usize) -> (f32, f32) {
        let (block, byte) = self.handle.caret_address(Pos(i + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(block as u64, byte)
            .expect("laid out")
    }
}

/// A press in a paragraph holding a hard break or an image lands in it. It
/// lands nowhere: `editor_point_address` answers `None`.
#[test]
fn a_press_in_a_paragraph_holding_a_leaf_lands_in_it() {
    for html in [
        "<p>alpha bravo<br>charlie delta</p>",
        "<p>alpha bravo<img src=\"x.png\" alt=\"\">charlie delta</p>",
    ] {
        let mut p = page(html);
        let (bx, by, _, _) = p.block_box();
        p.handle.set_selection(Selection::cursor(Pos(1)));
        idle(&mut p.app);
        // The right half of line 1's last glyph, `o` (char 10).
        let (ox, _) = p.local(10);
        let (ex, _) = p.local(11);
        assert!(ex > ox + 4.0, "positive control: {html}: `o` has width");
        p.click(bx + (ox + 3.0 * ex) / 4.0, by + LINE / 2.0);
        assert_eq!(p.head(), 11, "{html}: after `bravo`");
    }
}

/// The caret after a hard break is drawn at the start of line 2, and the one
/// after its first character one character in. Both are drawn one character
/// early: the view gives the `<br>` no byte, desktop's IFC gives it `"\n"`.
#[test]
fn the_caret_after_a_hard_break_is_drawn_after_it() {
    let p = page("<p>alpha bravo<br>charlie delta</p>");
    assert_eq!(
        p.local(11).1,
        0.0,
        "positive control: before the break, line 1"
    );
    assert_eq!(p.local(12), (0.0, LINE), "after the break: line 2's start");
    let (x, y) = p.local(13);
    assert_eq!(y, LINE);
    assert!(x > 4.0, "after `c`: one glyph in, got {x}");
}

/// A press on line 2, after a hard break, lands on the character it hit — the
/// inverse of the caret map. Left half of `c` is before it (char 12, right
/// after the break), right half after it (13). With the break worth no byte in
/// the map, the IFC's byte for `c` (12) read as the char after it. Each press
/// is on a fresh page, so no two read as a double click.
#[test]
fn a_press_after_a_hard_break_lands_on_the_character_hit() {
    const HTML: &str = "<p>alpha bravo<br>charlie delta</p>";
    let press = |frac: f32, line: f32| {
        let mut p = page(HTML);
        let (bx, by, _, _) = p.block_box();
        // `c` runs from line 2's start to the caret after it (char 13).
        let (cx, cy) = p.local(13);
        assert_eq!(cy, LINE, "positive control: after `c` is on line 2");
        assert!(cx > 4.0, "positive control: `c` has width, got {cx}");
        p.click(bx + cx * frac, by + LINE * line);
        p.head()
    };
    assert_eq!(
        press(0.25, 1.5),
        12,
        "left half of `c`: right after the break"
    );
    assert_eq!(press(0.75, 1.5), 13, "right half of `c`: after it");
}

/// The caret on the far side of an image is drawn after it, and the one before
/// it before it. It is not: desktop lays an image out as an inline box with no
/// bytes, so the positions on its two sides share one flat byte, and the host
/// draws both before it — the #1025 ambiguity, on desktop.
#[test]
#[ignore = "#1104: an image's two sides share one flat byte on desktop"]
fn the_caret_beside_an_image_is_drawn_on_its_side() {
    let p =
        page("<p>alpha<img src=\"x.png\" alt=\"\" style=\"width: 40px; height: 16px\">bravo</p>");
    let (before, y0) = p.local(5);
    let (after, y1) = p.local(6);
    assert_eq!(y0, y1, "one line");
    assert!(
        after >= before + 39.0,
        "after the 40px image: {before} -> {after}"
    );
}
