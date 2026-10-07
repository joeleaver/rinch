//! Review of PR #1433: file drops the PR's own fixtures do not sample. Each
//! asserts what an app would expect; the review report says which are red.

use super::event_dispatch::read_dropped_image;
use super::*;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;
use std::path::PathBuf;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    app_drops: Rc<RefCell<Vec<Vec<PathBuf>>>>,
}

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
    handle.set_selection(Selection::cursor(Pos(1)));
    Page {
        app,
        handle,
        app_drops,
    }
}

fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rinch-rv1433-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Page {
    fn drop_files(&mut self, paths: Vec<PathBuf>, position: (f64, f64)) {
        self.app
            .handle_event(PlatformEvent::FileDropped { paths, position }, VP, 1.0);
    }
}

/// D1. A mixed drop: the picture is the editor's. The other file was the app's
/// `onfiledrop` handler's before this PR; nobody gets it now.
#[test]
fn d1_the_other_files_of_a_mixed_drop_still_reach_the_apps_handler() {
    let d = dir("mixed");
    let png = d.join("cat.png");
    std::fs::write(&png, PNG_MAGIC).unwrap();
    let pdf = d.join("report.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    let mut p = page();
    p.handle.on_image_input(|_| None);
    p.drop_files(vec![pdf.clone(), png], (20.0, 12.0));
    let got = p.app_drops.borrow().clone();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(got, vec![vec![pdf]], "the PDF went to nobody");
}

/// D2. A file that is only NAMED like a picture: the editor claims the drop by
/// the extension, offers nothing (the bytes are not an image), and the app's
/// handler never hears of it. The drop vanishes.
#[test]
fn d2_a_file_only_named_like_a_picture_is_not_swallowed() {
    let d = dir("fake");
    let fake = d.join("notes.png");
    std::fs::write(&fake, b"just some words").unwrap();
    assert!(read_dropped_image(&fake).is_none(), "control: not an image");
    let mut p = page();
    let offered = Rc::new(Cell::new(0));
    let offered_in = offered.clone();
    p.handle.on_image_input(move |_| {
        offered_in.set(offered_in.get() + 1);
        None
    });
    p.drop_files(vec![fake.clone()], (20.0, 12.0));
    std::thread::sleep(std::time::Duration::from_millis(200));
    rinch_core::drain_main_callbacks();
    let got = p.app_drops.borrow().clone();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(offered.get(), 0, "control: nothing was offered");
    assert_eq!(
        got,
        vec![vec![fake]],
        "and the app's handler got nothing either"
    );
}

/// D3. A drop on the blank area under the last line lands in the editor (as a
/// press there places the caret), not in the app's handler.
#[test]
fn d3_a_drop_below_the_last_line_is_the_editors() {
    let d = dir("below");
    let png = d.join("cat.png");
    std::fs::write(&png, PNG_MAGIC).unwrap();
    let mut p = page();
    p.handle.on_image_input(|_| None);
    p.drop_files(vec![png], (200.0, 300.0));
    let got = p.app_drops.borrow().len();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(got, 0);
    assert_eq!(
        node_to_html(&p.handle.doc()),
        "<p>alpha bravo</p>",
        "control"
    );
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(12)));
}

/// D4. A drop the editor takes gives it the keyboard, as it does in the browser
/// (#1436) and as a press at that point would.
#[test]
fn d4_a_drop_the_editor_takes_focuses_it() {
    let d = dir("focus");
    let png = d.join("cat.png");
    std::fs::write(&png, PNG_MAGIC).unwrap();
    let mut p = page();
    p.handle.on_image_input(|_| None);
    assert!(!p.app.has_focused_contenteditable(), "control: not focused");
    p.drop_files(vec![png], (20.0, 12.0));
    for _ in 0..3 {
        p.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    }
    let _ = std::fs::remove_dir_all(&d);
    assert!(p.app.has_focused_contenteditable());
}

/// D5 (probe, `--ignored --nocapture`, run under heavy.sh). A file that is only
/// named like a picture is read WHOLE before its first twelve bytes are looked
/// at, and there is no size limit: the cost of a refusal grows with the file.
#[test]
#[ignore]
fn d5_probe_a_fake_picture_is_read_whole_before_it_is_refused() {
    let d = dir("big");
    for mb in [32u64, 64, 128, 256] {
        let fake = d.join(format!("video-{mb}.png"));
        let f = std::fs::File::create(&fake).unwrap();
        f.set_len(mb << 20).unwrap(); // sparse: zeros, not an image
        drop(f);
        let t = std::time::Instant::now();
        let got = read_dropped_image(&fake);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "D5 {mb:4} MB not-an-image .png: refused={} after {ms:.0} ms",
            got.is_none()
        );
        let _ = std::fs::remove_file(&fake);
        if ms > 2000.0 {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// D6. A file over the size limit is not the editor's, however it is named
/// and whatever its first bytes are: it goes to the app's handler with the
/// rest, and a picture dropped with it is still taken. (Sparse: the file
/// occupies no disk, and only its length and first bytes are looked at.)
#[test]
fn d6_a_picture_over_the_size_limit_is_left_to_the_apps_handler() {
    use super::event_dispatch::{MAX_DROPPED_IMAGE_BYTES, dropped_file_is_a_picture};
    use std::io::Write;
    let d = dir("huge");
    let huge = d.join("film.png");
    let mut f = std::fs::File::create(&huge).unwrap();
    f.write_all(PNG_MAGIC).unwrap();
    f.set_len(MAX_DROPPED_IMAGE_BYTES + 1).unwrap();
    drop(f);
    let at_limit = d.join("big.png");
    let mut f = std::fs::File::create(&at_limit).unwrap();
    f.write_all(PNG_MAGIC).unwrap();
    f.set_len(MAX_DROPPED_IMAGE_BYTES).unwrap();
    drop(f);
    assert!(dropped_file_is_a_picture(&at_limit), "control: at the limit");
    assert!(!dropped_file_is_a_picture(&huge));
    assert!(read_dropped_image(&huge).is_none());

    let cat = d.join("cat.png");
    std::fs::write(&cat, PNG_MAGIC).unwrap();
    let mut p = page();
    p.handle.on_image_input(|_| None);
    let claim = p
        .app
        .claim_editor_file_drop(20.0, 12.0, &[huge.clone(), cat.clone()])
        .expect("the small picture is the editor's");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(claim.images, vec![cat]);
    assert_eq!(claim.rest, vec![huge]);
}

/// D7. Something that is not a regular file (a directory named like a
/// picture) is not opened and not claimed.
#[test]
fn d7_a_directory_named_like_a_picture_is_not_a_picture() {
    use super::event_dispatch::dropped_file_is_a_picture;
    let d = dir("dir");
    let folder = d.join("album.png");
    std::fs::create_dir_all(&folder).unwrap();
    let got = dropped_file_is_a_picture(&folder);
    let mut p = page();
    p.handle.on_image_input(|_| None);
    p.drop_files(vec![folder.clone()], (20.0, 12.0));
    let drops = p.app_drops.borrow().clone();
    let _ = std::fs::remove_dir_all(&d);
    assert!(!got);
    assert_eq!(drops, vec![vec![folder]]);
}
