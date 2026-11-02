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
        let (x, _) = doc.query_caret_position(block, byte).expect("laid out");
        ((bx + x) as f64, (by + LINE / 2.0) as f64)
    }

    fn node_with(&self, attr: &str) -> Option<u64> {
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

    fn container(&self) -> u64 {
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

    // Read-only.
    p.handle.set_read_only(true);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    assert!(p.drop_caret().is_none(), "a read-only editor");
    send(&mut p.app, PlatformEvent::FileHoverCancelled);
    // Read-only, with no path named (yet, or ever: an unreadable URI list).
    p.drag_to(a);
    assert!(p.drop_caret().is_none(), "a read-only editor, path-less");
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
    // A pointer move hides the caret, and keeps the drag for a drop.
    let mut p = page();
    let a = p.point_at(5);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    send(&mut p.app, PlatformEvent::MouseMove { x: 600.0, y: 500.0 });
    assert!(p.drop_caret().is_none(), "a move hides it");
    assert!(p.app.file_drag.as_ref().is_some_and(|d| d.suspended));
    send(&mut p.app, PlatformEvent::AboutToWait);
    assert!(p.drop_caret().is_none(), "nothing shows it again");
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

    // A pointer move stops it too, though the drag waits for its drop.
    p.hover(PathBuf::from("/nowhere/cat.png"), at);
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    let scrolled = p.scroll_top();
    assert!(scrolled > stopped, "scrolling again");
    send(&mut p.app, PlatformEvent::MouseMove { x: 700.0, y: 500.0 });
    frame_after(&mut p, 30);
    frame_after(&mut p, 30);
    assert_eq!(p.scroll_top(), scrolled, "a move stops it");
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

/// A drag hovers with no path named until its URI list is read (#1962; before
/// it, Wayland named none before the drop: the fork started the hover with an
/// empty list, and only for an offer carrying `text/uri-list`): the editor
/// claims the hover on its own say, and the drop decides.
#[test]
fn a_path_less_hover_shows_the_drop_caret_and_a_picture_drop_lands_there() {
    let files = Files::new("pathless");
    let png = files.write("cat.png", PNG_MAGIC);
    let mut p = page();
    let (a, b) = (p.point_at(5), p.point_at(8));
    p.drag_to(a);
    assert_caret_at(&p, a, "path-less hover");
    assert_eq!(p.indicated(), Some(Pos(6)));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));
    send(
        &mut p.app,
        PlatformEvent::FileDropped {
            paths: vec![png],
            position: b,
        },
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    assert!(p.drop_caret().is_none());
}

#[test]
fn a_path_less_drop_of_no_picture_inserts_nothing_and_hides_the_caret() {
    let files = Files::new("pathless-text");
    let notes = files.write("notes.txt", b"words");
    let mut p = page();
    let a = p.point_at(5);
    p.drag_to(a);
    assert!(p.drop_caret().is_some(), "shown first");
    send(
        &mut p.app,
        PlatformEvent::FileDropped {
            paths: vec![notes],
            position: a,
        },
    );
    assert!(p.drop_caret().is_none(), "the drop hides it");
    assert!(p.app.file_drag.is_none());
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(1)),
        "the editor took nothing: the caret stays"
    );
    assert_eq!(
        rinch_editor_core::serialize::node_to_html(&p.handle.doc()),
        "<p>alpha bravo</p>"
    );
}

/// X11: the source's ungrab at the release sends the window a crossing Enter
/// (a pointer move to the shell) before the XdndDrop (measured with a GTK
/// source, review of #1655). The drop still lands where the caret showed.
#[test]
fn a_pointer_move_before_the_drop_keeps_the_shown_position() {
    let files = Files::new("enter");
    let png = files.write("cat.png", PNG_MAGIC);
    let mut p = page();
    let (a, b) = (p.point_at(5), p.point_at(8));
    p.hover(png.clone(), a);
    assert_eq!(p.indicated(), Some(Pos(6)));
    send(
        &mut p.app,
        PlatformEvent::MouseMove {
            x: b.0 as f32,
            y: b.1 as f32,
        },
    );
    send(
        &mut p.app,
        PlatformEvent::FileDropped {
            paths: vec![png],
            position: b,
        },
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    assert!(p.app.file_drag.is_none());
}

/// A hover after a pointer move is a new drag: the old one's picture does
/// not carry over.
#[test]
fn a_hover_after_a_pointer_move_starts_a_new_drag() {
    let mut p = page();
    let a = p.point_at(5);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    send(&mut p.app, PlatformEvent::MouseMove { x: 600.0, y: 500.0 });
    p.hover(PathBuf::from("/nowhere/notes.txt"), a);
    assert!(p.drop_caret().is_none(), "a text file, in a new drag");
    send(&mut p.app, PlatformEvent::FileHoverCancelled);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    assert_caret_at(&p, a, "control");
}

/// X11/Windows name each path with a `FileHoverEnter` of its own: a picture
/// and a text file dragged together show the caret.
#[test]
fn a_picture_then_a_text_file_in_one_drag_shows_the_caret() {
    let mut p = page();
    let a = p.point_at(5);
    p.hover(PathBuf::from("/nowhere/cat.png"), a);
    p.hover(PathBuf::from("/nowhere/notes.txt"), a);
    assert_caret_at(&p, a, "picture, then text");
}

fn two_editors() -> (
    RinchApp,
    crate::editor::EditorHandle,
    crate::editor::EditorHandle,
) {
    let h1 = crate::editor::create_editor();
    let h2 = crate::editor::create_editor();
    assert!(h1.load_html("<p>alpha bravo</p>"));
    assert!(h2.load_html("<p>charlie delta</p>"));
    let (i1, i2) = (h1.clone(), h2.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; display: flex");
        for h in [&i1, &i2] {
            let e = h.mount(scope);
            e.set_attribute(
                "style",
                "width: 300px; height: 200px; font-size: 16px; line-height: 24px; \
                 font-family: sans-serif",
            );
            root.append_child(&e);
        }
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    h1.on_image_input(|_| None);
    h2.on_image_input(|_| None);
    send(&mut app, PlatformEvent::AboutToWait);
    (app, h1, h2)
}

fn visible_drop_carets(app: &RinchApp) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes.contains_key("data-pm-drop-caret")
                && n.computed_style.visibility
                    == rinch_dom::computed_style::VisibilityValue::Visible
        })
        .count()
}

/// From one editor to another: one bar, in the editor under the pointer; a
/// drop back in the first resolves its point there (review of #1655).
#[test]
fn moving_between_two_editors_moves_the_caret() {
    let files = Files::new("two");
    let png = files.write("cat.png", PNG_MAGIC);
    let (mut app, h1, h2) = two_editors();
    send(
        &mut app,
        PlatformEvent::FileHoverEnter {
            path: png.clone(),
            position: (40.0, 12.0),
        },
    );
    assert_eq!(visible_drop_carets(&app), 1);
    send(
        &mut app,
        PlatformEvent::FileDragMoved {
            position: (340.0, 12.0),
        },
    );
    assert_eq!(
        visible_drop_carets(&app),
        1,
        "only one bar after moving to the second editor"
    );
    send(
        &mut app,
        PlatformEvent::FileDropped {
            paths: vec![png],
            position: (40.0, 12.0),
        },
    );
    assert_eq!(visible_drop_carets(&app), 0);
    assert_ne!(
        h1.selection(),
        Selection::cursor(Pos(1)),
        "the first editor took it"
    );
    assert_eq!(
        h2.selection(),
        Selection::cursor(Pos(1)),
        "the second did not"
    );
}

/// Feed `os_file_drag`'s steps to the app as the runtime does (#1962): each
/// fetch answered with a fresh serial, recorded per transfer.
fn feed(
    p: &mut Page,
    drag: &mut crate::shell::os_file_drag::OsFileDrag,
    steps: Vec<crate::shell::os_file_drag::DragStep>,
    serials: &mut Vec<(i64, winit::event_loop::AsyncRequestSerial)>,
) {
    use crate::shell::os_file_drag::DragStep;
    let mut queue: std::collections::VecDeque<DragStep> = steps.into();
    while let Some(step) = queue.pop_front() {
        match step {
            DragStep::Fetch(id) => {
                let serial = winit::event_loop::AsyncRequestSerial::get();
                serials.push((id.into_raw(), serial));
                queue.extend(drag.fetch_started(id, Ok(serial)));
            }
            DragStep::Emit(event) => {
                send(&mut p.app, event);
            }
            DragStep::Accept(_) | DragStep::Warn(_) => {}
        }
    }
}

/// A drag entering while an earlier drop still waits for its paths (#1962)
/// starts with clean state, and the earlier drop's late paths neither land
/// at the new drag's caret nor end it: the app holds one drag
/// (`file_drag`), so the entry ends the waiting drop first.
#[test]
fn a_new_drag_after_a_pending_drop_starts_clean_and_keeps_its_state() {
    use crate::shell::os_file_drag::OsFileDrag;
    use winit::data_transfer::DataTransferId;
    let (a_id, b_id) = (DataTransferId::from_raw(1), DataTransferId::from_raw(2));
    let serial_of = |serials: &[(i64, _)], id: DataTransferId| {
        serials
            .iter()
            .rev()
            .find(|(i, _)| *i == id.into_raw())
            .unwrap()
            .1
    };
    let mut p = page();
    let a = p.point_at(5);
    let mut d = OsFileDrag::default();
    let mut serials = Vec::new();

    // Drag A hovers path-less over the editor, which claims it, and drops
    // before its URI list comes.
    let steps = d.entered(a_id, Some(a), true);
    feed(&mut p, &mut d, steps, &mut serials);
    assert_caret_at(&p, a, "drag A, path-less");
    // Whatever state A left in the app's one drag slot (here its picture
    // claim) must not reach B.
    p.app.file_drag.as_mut().unwrap().picture = true;
    let steps = d.dropped(a_id, std::time::Instant::now());
    feed(&mut p, &mut d, steps, &mut serials);
    assert!(p.app.file_drag.is_some(), "A's drop waits for its paths");

    // Drag B enters and names a PDF: no picture, so no drop caret.
    let steps = d.entered(b_id, Some(a), true);
    feed(&mut p, &mut d, steps, &mut serials);
    let steps = d.received(
        b_id,
        serial_of(&serials, b_id),
        Some(vec!["file:///nowhere/notes.pdf".into()]),
    );
    feed(&mut p, &mut d, steps, &mut serials);
    let drag = p.app.file_drag.as_ref().expect("B is live");
    assert!(!drag.picture, "B inherited A's picture claim");
    assert!(drag.paths_known);
    assert!(p.drop_caret().is_none(), "a PDF shows no drop caret");

    // A's paths come late: ignored. B is still live and path-named.
    let steps = d.received(
        a_id,
        serial_of(&serials, a_id),
        Some(vec!["file:///nowhere/cat.png".into()]),
    );
    assert!(steps.is_empty(), "{steps:?}");
    let drag = p.app.file_drag.as_ref().expect("B is still live");
    assert!(drag.paths_known);
}
