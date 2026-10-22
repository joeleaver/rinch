//! `ondropfiles` on desktop (issue #1771): an OS drop reaches the nearest
//! element carrying a live `data-onfiledrop` (paths) or `data-ondropfiles`
//! (`LocalFile`s) handler, the files carry their name, size and path and read
//! their bytes on demand, and an editor that takes pictures hands the rest of
//! a mixed drop to `ondropfiles` as it does to `onfiledrop`.

use super::*;
use rinch_core::file::LocalFile;
use std::path::PathBuf;

const VP: (u32, u32) = (800, 600);

struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rinch-files-1771-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake};
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Arc::new(Thread(std::thread::current())).into();
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(std::time::Duration::from_millis(50));
    }
}

type Files = Rc<RefCell<Vec<Vec<LocalFile>>>>;
type Paths = Rc<RefCell<Vec<Vec<PathBuf>>>>;

/// A 800x600 root with an `ondropfiles` handler; inside it, at the top, a
/// 800x100 band with an `onfiledrop` (paths) handler, and below it a 800x100
/// band carrying both kinds.
fn app() -> (RinchApp, Files, Paths, Paths) {
    let files: Files = Rc::default();
    let band_paths: Paths = Rc::default();
    let both_paths: Paths = Rc::default();
    let (f, bp, bo) = (files.clone(), band_paths.clone(), both_paths.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let f1 = f.clone();
        let id = scope.register_files_drop_handler(move |files| f1.borrow_mut().push(files));
        root.set_attribute("data-ondropfiles", &id.0.to_string());

        let band = scope.create_element("div");
        band.set_attribute("style", "height: 100px");
        let bp = bp.clone();
        let id = scope.register_file_drop_handler(move |paths| bp.borrow_mut().push(paths));
        band.set_attribute("data-onfiledrop", &id.0.to_string());
        root.append_child(&band);

        let both = scope.create_element("div");
        both.set_attribute("style", "height: 100px");
        let bo = bo.clone();
        let id = scope.register_file_drop_handler(move |paths| bo.borrow_mut().push(paths));
        both.set_attribute("data-onfiledrop", &id.0.to_string());
        let f2 = f.clone();
        let id = scope.register_files_drop_handler(move |files| f2.borrow_mut().push(files));
        both.set_attribute("data-ondropfiles", &id.0.to_string());
        root.append_child(&both);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, files, band_paths, both_paths)
}

fn drop_at(app: &mut RinchApp, paths: Vec<PathBuf>, (x, y): (f64, f64)) {
    app.handle_event(
        PlatformEvent::FileDropped {
            paths,
            position: (x, y),
        },
        VP,
        1.0,
    );
}

#[test]
fn a_drop_hands_ondropfiles_named_sized_files_that_read_on_demand() {
    let dir = Dir::new("read");
    let glb = dir.write("model.glb", b"glTF\x02\x00\x00\x00");
    let txt = dir.write("notes.txt", b"hello");
    let (mut app, files, band, both) = app();
    drop_at(&mut app, vec![glb.clone(), txt.clone()], (400.0, 400.0));
    assert!(band.borrow().is_empty() && both.borrow().is_empty());
    let got = files.borrow().clone();
    assert_eq!(got.len(), 1, "one dispatch");
    let names: Vec<_> = got[0].iter().map(|f| (f.name().to_string(), f.size())).collect();
    assert_eq!(names, vec![("model.glb".into(), 8), ("notes.txt".into(), 5)]);
    assert_eq!(got[0][0].path(), Some(glb.as_path()));
    // Read lazily: a write after the drop is what the read sees.
    std::fs::write(&txt, b"rewritten").unwrap();
    assert_eq!(block_on(got[0][1].bytes()).unwrap(), b"rewritten");
    assert_eq!(block_on(got[0][0].bytes()).unwrap(), b"glTF\x02\x00\x00\x00");
}

#[test]
fn the_nearest_handler_wins_and_a_node_with_both_fires_both() {
    let dir = Dir::new("nearest");
    let a = dir.write("a.bin", b"a");
    let (mut app, files, band, both) = app();
    // The paths-only band is nearer than the root's `ondropfiles`.
    drop_at(&mut app, vec![a.clone()], (400.0, 50.0));
    assert_eq!(*band.borrow(), vec![vec![a.clone()]]);
    assert!(files.borrow().is_empty(), "the root's handler is farther");
    // A node carrying both: paths and files, once each.
    drop_at(&mut app, vec![a.clone()], (400.0, 150.0));
    assert_eq!(*both.borrow(), vec![vec![a.clone()]]);
    assert_eq!(files.borrow().len(), 1);
    assert_eq!(files.borrow()[0][0].name(), "a.bin");
    // A path gone before the drop reaches the paths handler, not as a file.
    let gone = dir.0.join("gone.bin");
    drop_at(&mut app, vec![gone.clone(), a.clone()], (400.0, 150.0));
    assert_eq!(both.borrow()[1], vec![gone, a]);
    assert_eq!(files.borrow()[1].len(), 1);
}

#[test]
fn an_editor_that_takes_pictures_hands_the_rest_to_ondropfiles() {
    let dir = Dir::new("editor");
    let png = dir.write("pic.png", b"\x89PNG\r\n\x1a\n0000");
    let pdf = dir.write("doc.pdf", b"%PDF-1.7");
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>alpha bravo</p>"));
    handle.on_image_input(|_| Some(("app-blob:1".into(), String::new())));
    let files: Files = Rc::default();
    let (h, f) = (handle.clone(), files.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let f = f.clone();
        let id = scope.register_files_drop_handler(move |files| f.borrow_mut().push(files));
        root.set_attribute("data-ondropfiles", &id.0.to_string());
        let editor = h.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px",
        );
        root.append_child(&editor);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    drop_at(&mut app, vec![png, pdf.clone()], (20.0, 12.0));
    let got = files.borrow();
    assert_eq!(got.len(), 1, "the rest of the drop reaches ondropfiles");
    assert_eq!(
        got[0].iter().map(|f| f.path().unwrap().to_path_buf()).collect::<Vec<_>>(),
        vec![pdf]
    );
}
