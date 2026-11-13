//! #2666: the editor's selection highlight is painted above the backgrounds of
//! the text it highlights and below its glyphs, as Chrome paints a selection.
//!
//! The highlight divs used to be `z-index: -1` in the editor's stacking
//! context, which paints before every in-flow background there: a `pre`, an
//! inline `code` or `mark`, or a `th` with an opaque background hid them
//! completely. Each fixture renders the same document with the needle selected
//! and with a collapsed caret, and compares the band across the needle's line:
//! the wash must change its background pixels and must leave its glyphs' ink as
//! dark as it was (a wash drawn over the text would tint them).

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

const STYLE: &str = "width: 500px; height: 400px; font-size: 16px; line-height: 24px; \
                     font-family: sans-serif";

fn page(html: &'static str, extra_css: &'static str) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        if !extra_css.is_empty() {
            let style = scope.create_element("style");
            style.append_child(&scope.create_text(extra_css));
            root.append_child(&style);
        }
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(html);
        container.set_attribute("style", STYLE);
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().unwrap();
    handle.focus();
    idle(&mut app);
    Page { app, handle }
}

/// Let the editor draw its overlays for the selection it has now.
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

/// The model range of the first `needle` in a text node.
fn find(p: &Page, needle: &str) -> (usize, usize) {
    let doc = p.handle.doc();
    let mut hit = None;
    doc.nodes_between(0, doc.content_size(), &mut |node, pos, _| {
        if hit.is_none()
            && let Some(text) = node.text()
            && let Some(byte) = text.find(needle)
        {
            let start = pos + text[..byte].chars().count();
            hit = Some((start, start + needle.chars().count()));
        }
        hit.is_none()
    });
    hit.unwrap_or_else(|| panic!("`{needle}` is in the document"))
}

fn frame(app: &mut RinchApp) -> Vec<u8> {
    idle(app);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    app.build_pixels(1.0, VP, false).0.to_vec()
}

fn px(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * VP.0 + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn luma(p: [u8; 4]) -> u32 {
    p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114
}

/// The needle's band: its x extent, inset by a pixel, over the middle of its
/// line (inside an inline `code`'s background, which is shorter than the line).
fn band(p: &Page, (a, b): (usize, usize)) -> (u32, u32, u32, u32) {
    let (x0, y, h) = p.app.editor_caret_point(&p.handle, Pos(a)).unwrap();
    let (x1, _, _) = p.app.editor_caret_point(&p.handle, Pos(b)).unwrap();
    (
        (x0 + 1.0).ceil() as u32,
        (y + h * 0.35).ceil() as u32,
        (x1 - 1.0).floor() as u32,
        (y + h * 0.65).floor() as u32,
    )
}

struct Seen {
    /// Pixels of the band the selection changed.
    changed: usize,
    total: usize,
    /// The darkest pixel of the band, unselected and selected.
    ink: (u32, u32),
    /// A changed pixel's colour, selected, for the message.
    sample: Option<[u8; 4]>,
}

fn look(html: &'static str, extra_css: &'static str, needle: &str) -> Seen {
    let mut p = page(html, extra_css);
    let range = find(&p, needle);
    p.handle.set_selection(Selection::cursor(Pos(range.0)));
    let plain = frame(&mut p.app);
    p.handle
        .set_selection(Selection::text(Pos(range.0), Pos(range.1)));
    let selected = frame(&mut p.app);
    let (x0, y0, x1, y1) = band(&p, range);
    assert!(x1 > x0 + 4 && y1 > y0, "a band to look at: {x0},{y0}-{x1},{y1}");
    let (mut changed, mut total, mut sample) = (0, 0, None);
    let (mut ink_plain, mut ink_sel) = (u32::MAX, u32::MAX);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (a, b) = (px(&plain, x, y), px(&selected, x, y));
            total += 1;
            if (0..3).any(|k| (a[k] as i32 - b[k] as i32).abs() > 8) {
                changed += 1;
                sample.get_or_insert(b);
            }
            ink_plain = ink_plain.min(luma(a));
            ink_sel = ink_sel.min(luma(b));
        }
    }
    Seen {
        changed,
        total,
        ink: (ink_plain, ink_sel),
        sample,
    }
}

/// The wash shows over at least a third of the band (the rest is glyph ink),
/// and the glyphs' darkest ink is unchanged: they are drawn over it.
fn assert_washed_under_text(what: &str, s: &Seen) {
    eprintln!(
        "{what}: {} of {} band pixels changed, sample {:?}, darkest ink {} -> {}",
        s.changed, s.total, s.sample, s.ink.0, s.ink.1
    );
    assert!(
        s.changed * 3 >= s.total,
        "{what}: the selection highlight is not visible ({} of {} band pixels changed)",
        s.changed,
        s.total
    );
    let (plain, sel) = s.ink;
    assert!(
        sel.abs_diff(plain) <= 3 * 1000,
        "{what}: the glyphs' ink changed under the selection ({plain} -> {sel}): \
         the highlight is drawn over the text"
    );
}

/// Positive control: a paragraph with no background showed its selection
/// before the fix too.
#[test]
fn a_selection_in_a_plain_paragraph_is_washed_under_its_text() {
    let s = look("<p>abc def ghi</p>", "", "def");
    assert_washed_under_text("p", &s);
}

#[test]
fn a_selection_in_a_code_block_shows_over_its_background() {
    let s = look("<p>above</p><pre><code>abc def ghi</code></pre>", "", "def");
    assert_washed_under_text("pre", &s);
}

#[test]
fn a_selection_in_inline_code_shows_over_its_background() {
    let s = look("<p>x <code>abc def ghi</code> y</p>", "", "def");
    assert_washed_under_text("code", &s);
}

#[test]
fn a_selection_in_a_mark_shows_over_its_background() {
    let s = look("<p>x <mark>abc def ghi</mark> y</p>", "", "def");
    assert_washed_under_text("mark", &s);
}

#[test]
fn a_selection_in_a_header_cell_shows_over_its_background() {
    let s = look(
        "<table><tr><th><p>abc def ghi</p></th><th><p>z</p></th></tr>\
         <tr><td><p>one</p></td><td><p>two</p></td></tr></table>",
        "",
        "def",
    );
    assert_washed_under_text("th", &s);
}

/// An app's opaque `pre` background (a dark theme's) hides nothing either.
#[test]
fn a_selection_shows_over_an_apps_opaque_dark_code_block() {
    let s = look(
        "<p>above</p><pre><code>abc def ghi</code></pre>",
        "[data-pm-editor] pre { background: #161b22; color: #f0f6fc; }",
        "def",
    );
    eprintln!(
        "dark pre: {} of {} changed, sample {:?}",
        s.changed, s.total, s.sample
    );
    assert!(
        s.changed * 3 >= s.total,
        "dark pre: the selection highlight is not visible"
    );
}
