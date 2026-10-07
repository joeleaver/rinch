//! Image files dropped on an editor, end to end on desktop: the
//! `PlatformEvent::FileDropped` a window receives, through the hit test, the
//! read off the UI thread and the app's `EditorHandle::on_image_input`
//! callback, to the image node at the drop point.
//!
//! The editor only takes the drop when its app asked to be offered pictures.
//! Otherwise, and for a drop that holds no image file, the app's own
//! `onfiledrop` handler gets it, exactly as before.

use super::event_dispatch::{offer_dropped_images, read_dropped_image};
use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;
use std::path::PathBuf;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

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

/// What the app was offered: bytes, media type, file name.
type Offered = Rc<RefCell<Vec<(Vec<u8>, String, Option<String>)>>>;

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    /// What the app's own `onfiledrop` handler (on the element around the
    /// editor) was given.
    app_drops: Rc<RefCell<Vec<Vec<PathBuf>>>>,
}

/// `alpha bravo` in a 400px editor, Inter 16px / 24px, inside an element with
/// an `onfiledrop` handler of the app's own.
fn page() -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>alpha bravo</p>"));
    let handle_in = handle.clone();
    let app_drops: Rc<RefCell<Vec<Vec<PathBuf>>>> = Rc::default();
    let drops_in = app_drops.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let drops = drops_in.clone();
        let id = scope.register_file_drop_handler(move |paths| drops.borrow_mut().push(paths));
        root.set_attribute("data-onfiledrop", &id.0.to_string());
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
    let mut page = Page {
        app,
        handle,
        app_drops,
    };
    page.handle.focus();
    page.handle.set_selection(Selection::cursor(Pos(1)));
    idle(&mut page.app);
    page
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

    fn drop_files(&mut self, paths: Vec<PathBuf>, position: (f64, f64)) {
        self.app
            .handle_event(PlatformEvent::FileDropped { paths, position }, VP, 1.0);
    }

    /// A drop the editor takes, start to finish on this thread: the claim the
    /// `FileDropped` handler makes (hit test, caret, anchor), then the read
    /// and the offers its worker thread and parked completion make. Done by
    /// hand because that completion comes back through the process-wide
    /// main-thread queue, which any other test's host may drain first.
    /// `meanwhile` runs between the two halves, as typing does while the files
    /// are read. Answers how many images were inserted.
    fn drop_on_editor(
        &mut self,
        paths: Vec<PathBuf>,
        position: (f64, f64),
        meanwhile: impl FnOnce(&crate::editor::EditorHandle),
    ) -> usize {
        let claim = self
            .app
            .claim_editor_file_drop(position.0 as f32, position.1 as f32, &paths)
            .expect("the editor takes this drop");
        let (handle, anchor, images) = (claim.handle, claim.anchor, claim.images);
        meanwhile(&handle);
        let files = images
            .iter()
            .filter_map(|p| read_dropped_image(p))
            .collect();
        let inserted = offer_dropped_images(&handle, &anchor, files);
        idle(&mut self.app);
        inserted
    }

    fn html(&self) -> String {
        node_to_html(&self.handle.doc())
    }
}

/// A directory of files to drop, removed when the test ends.
struct Files(PathBuf);

impl Files {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rinch-drop-{test}-{}", std::process::id()));
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

#[test]
fn an_image_file_dropped_on_an_editor_that_takes_pictures_lands_at_the_drop_point() {
    let files = Files::new("lands");
    let mut bytes = PNG_MAGIC.to_vec();
    bytes.extend_from_slice(b"pixels");
    let png = files.write("cat.png", &bytes);
    let notes = files.write("notes.txt", b"words");

    let mut p = page();
    let offered: Offered = Rc::default();
    let log = offered.clone();
    p.handle.on_image_input(move |input| {
        assert_eq!(input.source, crate::editor::ImageInputSource::Drop);
        log.borrow_mut().push((input.bytes, input.mime, input.name));
        Some(("app-blob:cat".to_string(), "a cat".to_string()))
    });

    // Dropped between `alpha` and the space, with the caret somewhere else;
    // the person types at the start of the line while the file is read.
    let at = p.point_at(5);
    let inserted = p.drop_on_editor(vec![notes, png], at, |handle| {
        assert_eq!(
            handle.selection(),
            Selection::cursor(Pos(6)),
            "the caret goes to the drop point at once, before the file is read"
        );
        handle.set_selection(Selection::cursor(Pos(1)));
        assert!(handle.insert_text("X"));
    });
    assert_eq!(inserted, 1);

    assert_eq!(
        *offered.borrow(),
        vec![(bytes, "image/png".to_string(), Some("cat.png".to_string()))],
        "the image file, and not the text file dropped with it"
    );
    assert_eq!(
        p.html(),
        r#"<p>Xalpha<img alt="a cat" src="app-blob:cat"> bravo</p>"#
    );
}

/// The window event itself: a drop the editor takes moves its caret there and
/// does not reach the app's own handler. (What the editor then does with the
/// files is the test above.)
#[test]
fn a_file_drop_event_the_editor_takes_does_not_reach_the_apps_handler() {
    let files = Files::new("event");
    let png = files.write("cat.png", PNG_MAGIC);
    let mut p = page();
    p.handle.on_image_input(|_| None);
    let at = p.point_at(5);
    p.drop_files(vec![png], at);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    assert!(p.app_drops.borrow().is_empty());
}

#[test]
fn a_drop_the_editor_does_not_take_is_the_apps_file_drop_handlers_as_before() {
    let files = Files::new("falls-through");
    let png = files.write("cat.png", PNG_MAGIC);
    let notes = files.write("notes.txt", b"words");

    // No callback: an image file dropped on the editor is the app's.
    let mut p = page();
    let at = p.point_at(5);
    p.drop_files(vec![png.clone()], at);
    assert_eq!(*p.app_drops.borrow(), vec![vec![png.clone()]]);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(1)),
        "caret unmoved"
    );

    // A callback, but nothing dropped is an image.
    let calls = Rc::new(Cell::new(0));
    let calls_in = calls.clone();
    p.handle.on_image_input(move |_| {
        calls_in.set(calls_in.get() + 1);
        None
    });
    p.drop_files(vec![notes.clone()], at);
    assert_eq!(p.app_drops.borrow().len(), 2);
    assert_eq!(p.app_drops.borrow()[1], vec![notes]);

    // A callback and an image, but dropped outside the editor.
    p.drop_files(vec![png.clone()], (700.0, 500.0));
    assert_eq!(p.app_drops.borrow().len(), 3);

    // A callback and an image on the editor, but the editor is read-only.
    p.handle.set_read_only(true);
    p.drop_files(vec![png], at);
    assert_eq!(p.app_drops.borrow().len(), 4);

    assert_eq!(calls.get(), 0, "the callback was offered none of them");
    assert_eq!(p.html(), "<p>alpha bravo</p>");
}
