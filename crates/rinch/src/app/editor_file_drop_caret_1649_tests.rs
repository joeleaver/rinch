//! The drop caret of an OS file drag over an editor (#1649): while a picture
//! file is dragged over an editor that would take it, a caret bar shows where
//! it would land, follows the pointer, scrolls a long note in the edge bands,
//! and goes with the drag — on its cancel, its drop, or the first event that
//! proves it is over without either. The drop lands where the caret showed.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::path::PathBuf;
use std::time::Duration;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;

fn send(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    let actions = app.handle_event(event, VP, 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
    actions
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// `html` in a 400px editor, Inter 16px / 24px, `extra` appended to its style.
fn page_with(html: &str, extra: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let extra = extra.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            &format!(
                "width: 400px; font-size: 16px; line-height: 24px; \
                 font-family: sans-serif; {extra}"
            ),
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    page.handle.set_selection(Selection::cursor(Pos(1)));
    page.handle.on_image_input(|_| None);
    for _ in 0..2 {
        send(&mut page.app, PlatformEvent::AboutToWait);
    }
    page
}

fn page() -> Page {
    page_with("<p>alpha bravo</p>", "height: 400px")
}

impl Page {
    /// The window point between chars `i - 1` and `i` of the paragraph.
    fn point_at(&self, i: usize) -> (f64, f64) {
        let (block, byte) = self.handle.caret_address(Pos(i + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, by, _, _) = painted_element_box(&doc.tree, block);
        let (x, _) = doc
            .query_caret_position(block as u64, byte)
            .expect("laid out");
        ((bx + x) as f64, (by + LINE / 2.0) as f64)
    }

    fn node_with(&self, attr: &str) -> Option<usize> {
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.contains_key(attr))
            .map(|(id, _)| id)
    }

    /// The drop caret's painted box, while it is shown.
    fn drop_caret(&self) -> Option<(f32, f32, f32, f32)> {
        let id = self.node_with("data-pm-drop-caret")?;
        let d = self.app.doc.as_ref().unwrap().borrow();
        let node = d.tree.get(id)?;
        if node.computed_style.visibility != rinch_dom::computed_style::VisibilityValue::Visible {
            return None;
        }
        Some(painted_element_box(&d.tree, id))
    }

    fn container(&self) -> usize {
        self.node_with("data-pm-editor").unwrap()
    }

    fn scroll_top(&self) -> f64 {
        self.app.doc.as_ref().unwrap().borrow().tree.nodes[self.container()]
            .scroll_offset
            .1
    }

    fn hover(&mut self, path: PathBuf, (x, y): (f64, f64)) {
        send(
            &mut self.app,
            PlatformEvent::FileHoverEnter {
                path,
                position: (x, y),
            },
        );
    }

    fn drag_to(&mut self, position: (f64, f64)) {
        send(&mut self.app, PlatformEvent::FileDragMoved { position });
    }

    fn indicated(&self) -> Option<Pos> {
        self.app
            .file_drag
            .as_ref()
            .and_then(|drag| drag.caret.as_ref())
            .map(|caret| caret.pos)
    }
}

fn assert_caret_at(p: &Page, point: (f64, f64), what: &str) {
    let (x, y, w, h) = p
        .drop_caret()
        .unwrap_or_else(|| panic!("{what}: no drop caret"));
    assert!(
        (f64::from(x) - point.0).abs() <= 1.0,
        "{what}: the drop caret is at x {x}, the text position at {}",
        point.0
    );
    assert!(
        f64::from(y) <= point.1 && point.1 <= f64::from(y + h),
        "{what}: the drop caret spans {y}..{} and misses the line at {}",
        y + h,
        point.1
    );
    assert_eq!(w, 2.0, "{what}: a caret bar");
}

/// A directory of files to drop, removed when the test ends.
struct Files(PathBuf);

impl Files {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rinch-drop-caret-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Files(dir)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[test]
fn a_picture_hovering_over_the_editor_shows_where_it_would_land_and_follows_the_pointer() {
    let mut p = page();
    let (a, b) = (p.point_at(5), p.point_at(8));
    assert!(p.drop_caret().is_none(), "no drop caret before a drag");

    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    assert_caret_at(&p, a, "hover");
    assert_eq!(p.indicated(), Some(Pos(6)));
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(1)),
        "the selection does not move before the drop"
    );

    p.drag_to(b);
    assert_caret_at(&p, b, "move");
    assert_eq!(p.indicated(), Some(Pos(9)));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));

    // Out of the editor: no caret; back in: the caret again.
    p.drag_to((700.0, 500.0));
    assert!(p.drop_caret().is_none(), "outside the editor");
    p.drag_to(a);
    assert_caret_at(&p, a, "back in");

    send(&mut p.app, PlatformEvent::FileHoverCancelled);
    assert!(p.drop_caret().is_none(), "the cancel hides it");
    assert!(p.app.file_drag.is_none());
}

#[test]
fn an_unfocused_editor_shows_the_drop_caret_too() {
    let mut p = page();
    p.handle.blur();
    send(&mut p.app, PlatformEvent::AboutToWait);
    let a = p.point_at(5);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    send(&mut p.app, PlatformEvent::AboutToWait);
    assert_caret_at(
        &p,
        a,
        "a caret pass hides the caret of a blurred editor, not this",
    );
}

#[test]
fn no_drop_caret_where_the_drop_would_not_be_the_editors() {
    let mut p = page();
    let a = p.point_at(5);

    // Nothing named like a picture.
    p.hover(PathBuf::from("/nowhere/notes.txt"), a);
    assert!(p.drop_caret().is_none(), "a text file");
    send(&mut p.app, PlatformEvent::FileHoverCancelled);

    // Wayland: no path before the drop.
    p.drag_to(a);
    assert!(p.drop_caret().is_none(), "no path known");
    send(&mut p.app, PlatformEvent::FileHoverCancelled);

    // Read-only.
    p.handle.set_read_only(true);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    assert!(p.drop_caret().is_none(), "a read-only editor");
    send(&mut p.app, PlatformEvent::FileHoverCancelled);
    p.handle.set_read_only(false);

    // An editor with no picture callback.
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>alpha bravo</p>"));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px",
        );
        editor
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    send(
        &mut app,
        PlatformEvent::FileHoverEnter {
            path: PathBuf::from("/nowhere/cat.png"),
            position: (20.0, 12.0),
        },
    );
    assert!(
        app.file_drag.as_ref().is_some_and(|d| d.caret.is_none()),
        "no callback, no drop caret"
    );

    // The positive control: the same hover, and the editor takes it.
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    assert_caret_at(&p, a, "control");
}

#[test]
fn the_drop_lands_where_the_drop_caret_was_and_takes_it_away() {
    let files = Files::new("lands");
    let png = files.write("cat.png", PNG_MAGIC);
    let mut p = page();
    let (a, b) = (p.point_at(5), p.point_at(8));
    p.hover(png.clone(), a);
    assert_eq!(p.indicated(), Some(Pos(6)));
    // The drop reports another point than the last hover did: the caret the
    // person saw is where it goes.
    send(
        &mut p.app,
        PlatformEvent::FileDropped {
            paths: vec![png],
            position: b,
        },
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    assert!(p.drop_caret().is_none(), "the drop hides it");
    assert!(p.app.file_drag.is_none());
}

#[test]
fn a_drop_after_the_document_changed_resolves_its_point_again() {
    let files = Files::new("changed");
    let png = files.write("cat.png", PNG_MAGIC);
    let mut p = page();
    let (a, b) = (p.point_at(5), p.point_at(8));
    p.hover(png.clone(), a);
    assert_eq!(p.indicated(), Some(Pos(6)));
    // A peer (here: the app) changed the document after the last hover.
    p.handle.set_selection(Selection::cursor(Pos(12)));
    assert!(p.handle.insert_text("!"));
    send(&mut p.app, PlatformEvent::AboutToWait);
    send(
        &mut p.app,
        PlatformEvent::FileDropped {
            paths: vec![png],
            position: b,
        },
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(9)));
}

/// The cancel can go missing (#189's shape): any event the window hears only
/// once the OS drag is over ends it, caret and all.
#[test]
fn a_missing_cancel_does_not_strand_the_drop_caret() {
    let ends: Vec<PlatformEvent> = vec![
        PlatformEvent::MouseMove { x: 600.0, y: 500.0 },
        PlatformEvent::MouseDown {
            x: 600.0,
            y: 500.0,
            button: MouseButton::Left,
        },
        PlatformEvent::WindowFocus(false),
        PlatformEvent::PointerCancel,
    ];
    for end in ends {
        let what = format!("{end:?}");
        let mut p = page();
        let a = p.point_at(5);
        p.hover(PathBuf::from("/nowhere/cat.png"), a);
        assert!(p.drop_caret().is_some(), "{what}: shown first");
        send(&mut p.app, end);
        assert!(p.drop_caret().is_none(), "{what}: still shown");
        assert!(p.app.file_drag.is_none(), "{what}: the drag is still live");
    }
    // Events that come during an OS drag do not end it.
    let mut p = page();
    let a = p.point_at(5);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    send(&mut p.app, PlatformEvent::AboutToWait);
    send(&mut p.app, PlatformEvent::WindowFocus(true));
    assert!(p.drop_caret().is_some(), "a frame or a focus gain ended it");
}

/// Forty lines in a 120px editor that scrolls.
fn long_page() -> Page {
    let html: String = (0..40).map(|i| format!("<p>line {i}</p>")).collect();
    page_with(&html, "height: 120px; overflow-y: auto")
}

fn frame_after(p: &mut Page, ms: u64) -> Vec<AppAction> {
    std::thread::sleep(Duration::from_millis(ms));
    send(&mut p.app, PlatformEvent::AboutToWait)
}

#[test]
fn a_picture_resting_in_the_bottom_band_scrolls_the_note_and_the_caret_follows() {
    let mut p = long_page();
    let (cx, cy, cw, ch) = {
        let d = p.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, p.container())
    };
    let at = (f64::from(cx + cw / 2.0), f64::from(cy + ch - 6.0));
    p.hover(PathBuf::from("/nowhere/cat.png"), at);
    let first = p.indicated().expect("a drop caret");
    assert_eq!(p.scroll_top(), 0.0);
    let mut asked = false;
    for _ in 0..4 {
        asked |= frame_after(&mut p, 30)
            .iter()
            .any(|a| matches!(a, AppAction::RequestRedraw));
    }
    assert!(p.scroll_top() > 0.0, "the note scrolled");
    assert!(asked, "the clock keeps turning");
    let later = p.indicated().expect("still a drop caret");
    assert!(
        later.0 > first.0,
        "the caret follows the text under the pointer"
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));

    send(&mut p.app, PlatformEvent::FileHoverCancelled);
    let stopped = p.scroll_top();
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    assert_eq!(p.scroll_top(), stopped, "the cancel stops it");
}

#[test]
fn a_missing_cancel_does_not_strand_the_scrolling() {
    let mut p = long_page();
    let (cx, cy, cw, ch) = {
        let d = p.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, p.container())
    };
    let at = (f64::from(cx + cw / 2.0), f64::from(cy + ch - 6.0));
    p.hover(PathBuf::from("/nowhere/cat.png"), at);
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    assert!(p.scroll_top() > 0.0, "the note scrolled");
    send(&mut p.app, PlatformEvent::WindowFocus(false));
    let stopped = p.scroll_top();
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    assert_eq!(p.scroll_top(), stopped, "a blur ends the drag");
}

#[test]
fn a_drag_the_editor_would_not_take_does_not_scroll_it() {
    let mut p = long_page();
    let (cx, cy, cw, ch) = {
        let d = p.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, p.container())
    };
    let at = (f64::from(cx + cw / 2.0), f64::from(cy + ch - 6.0));
    p.hover(PathBuf::from("/nowhere/notes.txt"), at);
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    assert_eq!(p.scroll_top(), 0.0);
}
