//! Issue #1650: a `data-onfiledrop` handler reads the drop point and its own
//! element's bounds through `get_click_context()`, in the frame every other
//! handler's `ClickContext` is in (logical window px; the element box as it is
//! painted), so an app can place dropped files where they were dropped.
//!
//! Every fixture drops off the origin and off the element's origin, and first
//! writes a context that is wrong in every field, so a dispatch that sets no
//! context (or the wrong one) cannot pass by agreeing with a default.

use super::*;
use rinch_core::events::{ClickContext, MouseButton, ModifierState, TextHitInfo};
use std::path::PathBuf;

const VP: (u32, u32) = (800, 600);

type Seen = Rc<RefCell<Vec<(Vec<PathBuf>, ClickContext)>>>;

/// A context no correct dispatch can leave behind.
fn poison() {
    rinch_core::events::set_click_context(ClickContext {
        mouse_x: -1.0,
        mouse_y: -1.0,
        element_x: -1.0,
        element_y: -1.0,
        element_width: -1.0,
        element_height: -1.0,
        text_hit: TextHitInfo::default(),
        viewport_width: -1.0,
        viewport_height: -1.0,
        button: MouseButton::Right,
        modifiers: ModifierState::default(),
    });
}

/// Mark `el` as a file-drop target whose handler records the paths and the
/// click context it reads.
fn on_drop(scope: &mut RenderScope, el: &NodeHandle, seen: &Seen) {
    let seen = seen.clone();
    let id = scope.register_file_drop_handler(move |paths| {
        seen.borrow_mut()
            .push((paths, rinch_core::events::get_click_context()));
    });
    el.set_attribute("data-onfiledrop", &id.0.to_string());
}

fn app_of(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(build);
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

fn drop_at(app: &mut RinchApp, position: (f64, f64), window: (u32, u32), scale: f64) {
    poison();
    app.handle_event(
        PlatformEvent::FileDropped {
            paths: vec![PathBuf::from("/tmp/a.txt")],
            position,
        },
        window,
        scale,
    );
}

fn one(seen: &Seen) -> ClickContext {
    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "the handler ran once");
    assert_eq!(seen[0].0, vec![PathBuf::from("/tmp/a.txt")]);
    seen[0].1
}

fn assert_box(ctx: &ClickContext, (x, y, w, h): (f32, f32, f32, f32)) {
    let got = (
        ctx.element_x,
        ctx.element_y,
        ctx.element_width,
        ctx.element_height,
    );
    assert!(
        (got.0 - x).abs() < 0.01
            && (got.1 - y).abs() < 0.01
            && (got.2 - w).abs() < 0.01
            && (got.3 - h).abs() < 0.01,
        "element box {got:?}, wanted {:?}",
        (x, y, w, h)
    );
}

#[test]
fn a_file_drop_handler_reads_the_drop_point_and_its_elements_box() {
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let mut app = app_of(move |scope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 120px; top: 70px; width: 200px; height: 100px",
        );
        on_drop(scope, &target, &s);
        root.append_child(&target);
        root
    });
    drop_at(&mut app, (157.0, 93.0), VP, 1.0);
    let ctx = one(&seen);
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (157.0, 93.0));
    assert_box(&ctx, (120.0, 70.0, 200.0, 100.0));
    assert_eq!((ctx.viewport_width, ctx.viewport_height), (800.0, 600.0));
    assert_eq!(ctx.button, MouseButton::Left);
}

/// The element box is the one it is painted in, as for a click (#203): under
/// `scale(2)` a 100x50 box at (10, 5) in a wrapper at (40, 30) covers
/// (60, 40, 200, 100) on screen.
#[test]
fn the_element_box_is_the_painted_one_under_a_transform() {
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let mut app = app_of(move |scope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let wrap = scope.create_element("div");
        wrap.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 30px; width: 300px; height: 300px; \
             transform: scale(2); transform-origin: 0 0",
        );
        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 10px; top: 5px; width: 100px; height: 50px",
        );
        on_drop(scope, &target, &s);
        wrap.append_child(&target);
        root.append_child(&wrap);
        root
    });
    drop_at(&mut app, (211.0, 117.0), VP, 1.0);
    let ctx = one(&seen);
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (211.0, 117.0));
    assert_box(&ctx, (60.0, 40.0, 200.0, 100.0));
}

/// Under a scroller scrolled by 250, a target 400px down its content is
/// painted at 150.
#[test]
fn the_element_box_follows_a_scrolled_ancestor() {
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let scroller_out: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let out = scroller_out.clone();
    let mut app = app_of(move |scope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 0; width: 300px; height: 200px; \
             overflow-y: auto",
        );
        let spacer = scope.create_element("div");
        spacer.set_attribute("style", "height: 400px");
        let target = scope.create_element("div");
        target.set_attribute("style", "height: 100px");
        on_drop(scope, &target, &s);
        scroller.append_child(&spacer);
        scroller.append_child(&target);
        root.append_child(&scroller);
        *out.borrow_mut() = Some(scroller.clone());
        root
    });
    scroller_out.borrow().as_ref().unwrap().set_scroll_top(250.0);
    app.resolve_and_repaint(800.0, 600.0);
    drop_at(&mut app, (63.0, 171.0), VP, 1.0);
    let ctx = one(&seen);
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (63.0, 171.0));
    assert_box(&ctx, (20.0, 150.0, 300.0, 100.0));
}

/// At a scale factor of 1.5 the drop point the shell hands over is already
/// logical, and so is the viewport the handler reads: the same frame as a
/// click's.
#[test]
fn at_scale_one_and_a_half_the_context_is_logical() {
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let mut app = app_of(move |scope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 120px; top: 70px; width: 200px; height: 100px",
        );
        on_drop(scope, &target, &s);
        root.append_child(&target);
        root
    });
    drop_at(&mut app, (157.0, 93.0), (1200, 900), 1.5);
    let ctx = one(&seen);
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (157.0, 93.0));
    assert_box(&ctx, (120.0, 70.0, 200.0, 100.0));
    assert_eq!((ctx.viewport_width, ctx.viewport_height), (800.0, 600.0));
}

/// The files of a mixed drop the editor does not take reach the app's handler
/// with the same drop point.
#[cfg(feature = "desktop")]
#[test]
fn the_files_an_editor_hands_back_carry_the_drop_point() {
    const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
    let d = std::env::temp_dir().join(format!("rinch-1650-mixed-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let png = d.join("cat.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    let pdf = d.join("report.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();

    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>alpha bravo</p>"));
    handle.on_image_input(|_| None);
    let handle_in = handle.clone();
    let seen: Seen = Rc::default();
    let s = seen.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: absolute; left: 7px; top: 9px; width: 700px; height: 500px",
        );
        on_drop(scope, &root, &s);
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

    poison();
    app.handle_event(
        PlatformEvent::FileDropped {
            paths: vec![pdf.clone(), png],
            position: (44.0, 22.0),
        },
        VP,
        1.0,
    );
    let got = seen.borrow().clone();
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(got.len(), 1, "the app's handler got the PDF");
    assert_eq!(got[0].0, vec![pdf]);
    let ctx = got[0].1;
    assert_eq!((ctx.mouse_x, ctx.mouse_y), (44.0, 22.0));
    assert_box(&ctx, (7.0, 9.0, 700.0, 500.0));
}
