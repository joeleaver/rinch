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
//!
//! An image has no byte to count: it is an inline box, so its two sides are
//! one flat byte. #1104 draws a caret beside one from its box
//! (`DomDocument::query_inline_box_caret`) and resolves a point on one to the
//! half it is on (`text_query::inline_box_at_point`).

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

/// One paragraph in a 400px editor, Inter 16px / 24px, focused and settled;
/// every image 40x16.
fn page(html: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        // The model keeps an image's `src` and `alt`, not an inline style, so
        // an image's size comes from a sheet: 40x16 wherever one appears.
        let sheet = scope.create_element("style");
        sheet.append_child(&scope.create_text("img { width: 40px; height: 16px; }"));
        root.append_child(&sheet);
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

impl Page {
    /// The painted boxes of the visible selection-highlight rects.
    fn highlights(&self) -> Vec<(f32, f32, f32, f32)> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter(|id| {
                doc.tree.nodes[id.0].computed_style.display
                    != rinch_dom::computed_style::DisplayValue::None
            })
            .map(|id| painted_element_box(&doc.tree, id.0))
            .filter(|b| b.2 > 0.0 && b.3 > 0.0)
            .collect()
    }
}

/// A press in a paragraph holding a hard break or an image lands in it. It
/// lands nowhere: `editor_point` answers `None`.
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

/// An image, 40x16 from the page's sheet ([`page`]).
const IMG: &str = "<img src=\"x.png\" alt=\"\">";

impl Page {
    /// The drawn caret overlay's painted `(x, y)`, with the caret at char `i`.
    fn caret_drawn(&mut self, i: usize) -> (f32, f32) {
        self.handle.set_selection(Selection::cursor(Pos(i + 1)));
        idle(&mut self.app);
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .next()
            .expect("a caret overlay");
        let (x, y, _, _) = painted_element_box(&doc.tree, id.0);
        (x, y)
    }
    /// `EditorHandle::caret_rect` for char `i`: the popup-frame caret.
    fn caret_rect_x(&self, i: usize) -> f32 {
        self.handle.caret_rect(Pos(i + 1)).expect("laid out").x
    }
    /// The painted boxes of the paragraph's images, in document order.
    fn images(&self) -> Vec<(f32, f32, f32, f32)> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("img")
            .into_iter()
            .map(|id| painted_element_box(&doc.tree, id.0))
            .collect()
    }
    /// Press at `from`, drag to `to`, release: the selection's head.
    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) -> usize {
        let button = MouseButton::Left;
        let (x, y) = from;
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        let (x, y) = to;
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        idle(&mut self.app);
        self.head()
    }
}

/// The caret on the far side of an image is drawn after it, and the one before
/// it before it — the overlay and `EditorHandle::caret_rect` alike. It was
/// not: desktop lays an image out as an inline box with no bytes, so the
/// positions on its two sides share one flat byte, and the host drew both
/// before it — the #1025 ambiguity, on desktop (#1104).
#[test]
fn the_caret_beside_an_image_is_drawn_on_its_side() {
    let mut p = page(&format!("<p>alpha{IMG}bravo</p>"));
    let img = p.images()[0];
    let (before, y0) = p.caret_drawn(5);
    let (after, y1) = p.caret_drawn(6);
    assert_eq!(y0, y1, "one line");
    assert!(
        (before - img.0).abs() < 1.5,
        "before the image, at its left edge {}: {before}",
        img.0
    );
    assert!(
        (after - (img.0 + img.2)).abs() < 1.5,
        "after the 40px image, at its right edge {}: {after}",
        img.0 + img.2
    );
    let (b, a) = (p.caret_rect_x(5), p.caret_rect_x(6));
    assert!(a >= b + 39.0, "caret_rect: {b} -> {a}");
}

/// An image at a paragraph's start, and two in a row: the caret before the
/// first is at the paragraph's start, the one between them at the first's
/// right edge, the one after both at the second's. The position before a
/// leading image has no text before it, and the one between two images
/// shares its byte with both sides of both.
#[test]
fn the_carets_around_leading_and_adjacent_images_are_each_on_their_side() {
    let mut p = page(&format!("<p>{IMG}{IMG}bravo</p>"));
    let imgs = p.images();
    assert_eq!(imgs.len(), 2, "positive control: two images");
    assert!(imgs[1].0 >= imgs[0].0 + 39.0, "side by side: {imgs:?}");
    let expect = [imgs[0].0, imgs[0].0 + imgs[0].2, imgs[1].0 + imgs[1].2];
    for (i, want) in expect.into_iter().enumerate() {
        let (x, _) = p.caret_drawn(i);
        assert!(
            (x - want).abs() < 1.5,
            "char {i}: drawn at {x}, want {want}"
        );
        let r = p.caret_rect_x(i);
        assert!((r - x).abs() < 1.5, "char {i}: caret_rect {r} vs drawn {x}");
    }
}

/// An image at a paragraph's end, and one right after a hard break: the caret
/// after the first is at its right edge, the one before the second at its left
/// edge on line 2. Neither has a byte of its own to be drawn at: the first's is
/// the paragraph's end, drawn after the `a` before it, and the second's is the
/// start of line 2, which Parley draws before the next character — after the
/// image.
#[test]
fn the_carets_at_a_trailing_image_and_after_a_break_are_each_on_their_side() {
    let mut p = page(&format!("<p>alpha{IMG}</p>"));
    let img = p.images()[0];
    assert!((p.caret_drawn(5).0 - img.0).abs() < 1.5, "before it");
    let (x, _) = p.caret_drawn(6);
    assert!(
        (x - (img.0 + img.2)).abs() < 1.5,
        "after it: {x} vs {img:?}"
    );

    let mut p = page(&format!("<p>ab<br>{IMG}cd</p>"));
    let img = p.images()[0];
    let (x, y) = p.caret_drawn(3);
    assert!(
        img.1 > y,
        "positive control: the image is on line 2, below {y}"
    );
    assert!((x - img.0).abs() < 1.5, "before it: {x} vs {img:?}");
    let (x, _) = p.caret_drawn(4);
    assert!(
        (x - (img.0 + img.2)).abs() < 1.5,
        "after it: {x} vs {img:?}"
    );
}

/// ArrowDown from the caret after an image keeps the image's right edge as
/// its column. The column is the caret's point (`editor_caret_point`), which
/// read the flat byte and so started from the image's left edge.
#[test]
fn arrow_down_from_after_an_image_keeps_its_column() {
    let mut p = page(&format!("<p>alpha{IMG}bravo<br>mmmmmmmmmmmmmmmm</p>"));
    let img = p.images()[0];
    let (_, y0) = p.caret_drawn(6);
    let press = |app: &mut RinchApp, down: bool| {
        let key = KeyCode::ArrowDown;
        let modifiers = Modifiers::default();
        if down {
            app.handle_event(
                PlatformEvent::KeyDown {
                    key,
                    logical_key: None,
                    text: None,
                    modifiers,
                    repeat: KeyRepeat::Fresh,
                },
                VP,
                1.0,
            );
        } else {
            app.handle_event(
                PlatformEvent::KeyUp {
                    key,
                    logical_key: None,
                    modifiers,
                },
                VP,
                1.0,
            );
        }
    };
    press(&mut p.app, true);
    press(&mut p.app, false);
    idle(&mut p.app);
    let head = p.head();
    let (x, y) = p.caret_drawn(head);
    assert!(y > y0, "positive control: moved to line 2, head {head}");
    // `m` is about 14px wide in Inter 16px: the nearest boundary is within 8.
    assert!(
        (x - (img.0 + img.2)).abs() < 8.0,
        "below the image's right edge {}: {x}",
        img.0 + img.2
    );
}

/// An image that starts a soft-wrapped line after text: the caret before it
/// is drawn at its left edge on line 2, the one after it at its right edge.
/// Parley's downstream caret at the image's byte is the leading edge of the
/// character after the image, so the caret before it was drawn after it —
/// the after-a-break shape, reached by a wrap. (Review of PR #1119, F1.)
#[test]
fn the_caret_before_an_image_that_starts_a_wrapped_line_is_before_it() {
    let mut p = page(&format!("<p>mmmmmmmmmmmmmmmmmmmmmmm {IMG}bravo</p>"));
    let img = p.images()[0];
    let (_, by, _, _) = p.block_box();
    assert!(
        img.1 > by + LINE * 0.9,
        "positive control: the image wrapped {img:?}"
    );
    let (x, y) = p.caret_drawn(24);
    assert!(y > by + LINE * 0.9, "before it: on line 2, got y {y}");
    assert!((x - img.0).abs() < 1.5, "before it: {x} vs {img:?}");
    let (x, _) = p.caret_drawn(25);
    assert!(
        (x - (img.0 + img.2)).abs() < 1.5,
        "after it: {x} vs {img:?}"
    );
    let r = p.caret_rect_x(24);
    assert!(
        (r - img.0).abs() < 1.5,
        "caret_rect before it: {r} vs {img:?}"
    );
}

/// A drag onto an image on line 2 lands on the half it ends on: the hit test
/// has to find the line the point is on before the box on it. (Review of PR
/// #1119, F3: nothing pinned an image below line 1.)
#[test]
fn a_drag_onto_an_image_on_line_two_lands_on_its_half() {
    for (frac, want) in [(0.25, 3), (0.75, 4)] {
        let mut p = page(&format!("<p>ab<br>{IMG}cd</p>"));
        let (bx, by, _, _) = p.block_box();
        let i = p.images()[0];
        assert!(
            i.1 > by + LINE * 0.9,
            "positive control: image on line 2 {i:?}"
        );
        let (fx, _) = p.local(0);
        let (nx, _) = p.local(1);
        let from = (bx + (fx + nx) / 2.0, by + LINE / 2.0);
        let head = p.drag(from, (i.0 + i.2 * frac, i.1 + i.3 / 2.0));
        assert_eq!(head, want, "frac {frac}");
    }
}

/// A drag that ends over an image's right half puts the head after it, over
/// its left half before it. Parley's hit test steps over an inline box to the
/// next character's byte, which is the byte of both of the image's sides, so
/// every drag onto an image ended before it — and onto a leading image, after.
#[test]
fn a_drag_onto_an_image_lands_on_the_half_it_ends_on() {
    let drag = |html: &str, from: usize, img: usize, frac: f32| {
        let mut p = page(html);
        let (bx, by, _, _) = p.block_box();
        let (fx, _) = p.local(from);
        let (nx, _) = p.local(from + 1);
        let i = p.images()[img];
        let start = (bx + (fx + nx) / 2.0 - 0.5, by + LINE / 2.0);
        p.drag(start, (i.0 + i.2 * frac, i.1 + i.3 / 2.0))
    };
    let one = format!("<p>alpha{IMG}bravo</p>");
    assert_eq!(drag(&one, 1, 0, 0.25), 5, "left half: before the image");
    assert_eq!(drag(&one, 1, 0, 0.75), 6, "right half: after it");
    let two = format!("<p>{IMG}{IMG}bravo</p>");
    assert_eq!(drag(&two, 3, 0, 0.25), 0, "first image, left half");
    assert_eq!(drag(&two, 3, 0, 0.75), 1, "first image, right half");
    assert_eq!(drag(&two, 3, 1, 0.25), 1, "second image, left half");
    assert_eq!(drag(&two, 3, 1, 0.75), 2, "second image, right half");
}

/// A selection spanning a break highlights the tail of line 1 and the head of
/// line 2, and the line-2 rect ends where the caret after `ch` is drawn. The
/// highlight maps its range through the same byte map as the caret; counting
/// the break as no byte there would end the rect one glyph early. (From the
/// review of PR #1105.)
#[test]
fn a_selection_across_a_hard_break_highlights_both_lines() {
    let mut p = page("<p>alpha bravo<br>charlie delta</p>");
    let (bx, by, _, _) = p.block_box();
    let (x14, y14) = p.local(14); // after `ch`
    assert_eq!(y14, LINE, "positive control: after `ch` is on line 2");
    p.handle.set_selection(Selection::text(Pos(10), Pos(15)));
    idle(&mut p.app);
    let hl = p.highlights();
    assert_eq!(hl.len(), 2, "one rect per line: {hl:?}");
    let l2 = hl.iter().find(|r| r.1 > by + LINE / 2.0).unwrap();
    assert!(
        ((l2.0 + l2.2) - (bx + x14)).abs() < 2.5,
        "line-2 rect ends after `ch`: {l2:?} vs {}",
        bx + x14
    );
}

/// Two breaks in a row: a caret on each line's start, and a press on the empty
/// middle line lands between them. The middle line's only byte is the second
/// break's own `"\n"`, which no text run answers first — the map's leaf branch
/// has to read it as the position before that break. (From the review of PR
/// #1105.)
#[test]
fn consecutive_hard_breaks_each_start_a_line() {
    const HTML: &str = "<p>ab<br><br>cd</p>";
    let p = page(HTML);
    assert_eq!(p.local(2).1, 0.0, "before the first break: line 1");
    assert_eq!(p.local(3), (0.0, LINE), "between the breaks: line 2");
    assert_eq!(p.local(4), (0.0, 2.0 * LINE), "after both: line 3");
    let (x, y) = p.local(5);
    assert_eq!(y, 2.0 * LINE);
    assert!(x > 4.0, "after `c`: one glyph in, got {x}");
    let press = |line: f32| {
        let mut p = page(HTML);
        let (bx, by, _, _) = p.block_box();
        p.click(bx + 100.0, by + line * LINE);
        p.head()
    };
    assert_eq!(press(0.5), 2, "past line 1's end: before the first break");
    assert_eq!(press(1.5), 3, "the empty line 2: between the breaks");
    assert_eq!(press(2.5), 6, "past line 3's end: the paragraph end");
}
