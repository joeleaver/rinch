//! #1768: a `RenderSurface` whose node leaves the document while its component
//! lives on — a captured handle hidden by an `if` — is not run by the desktop
//! paint, and a continuous one stops asking for paints until it is shown again.
//!
//! The steps are the desktop paint's (`paint_software` / `paint_gpu`): tell the
//! surfaces their layouts, forgetting the absent ones, then run the callbacks.

use super::*;
use crate::render_surface::{
    REDRAW_CALLBACK_TEST_LOCK, RenderSurface, clear_redraw_callback, create_render_surface,
    invoke_render_callbacks, set_redraw_callback,
};
use rinch_core::Component;
use rinch_core::reactive::Signal;
use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

const VP: (f32, f32) = (400.0, 300.0);

#[test]
fn a_hidden_captured_surface_is_not_run_and_asks_for_no_paint() {
    let _lock = REDRAW_CALLBACK_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let callers: Arc<Mutex<Vec<ThreadId>>> = Arc::default();
    let sink = callers.clone();
    set_redraw_callback(Arc::new(move || {
        sink.lock().unwrap().push(std::thread::current().id());
    }));
    let me = std::thread::current().id();
    let asked = || callers.lock().unwrap().iter().filter(|t| **t == me).count();

    let surface = create_render_surface();
    let calls: Rc<Cell<u32>> = Rc::default();
    {
        let c = calls.clone();
        surface.set_render_callback(move |_w, _x, _y| c.set(c.get() + 1));
    }
    surface.set_continuous(true);
    let show = Signal::new(true);
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrap = scope.create_element("div");
        wrap.set_attribute("style", "width: 120px; height: 60px;");
        root.append_child(&wrap);
        // Built once, outside the branch: the branch only shows and hides it,
        // so the component (and its registration) outlives every hide.
        let node = RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        rinch_core::show::show_dom(
            scope,
            &wrap,
            move || show.get(),
            move |_s: &mut RenderScope| node.clone(),
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        root
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);

    let paint = |app: &RinchApp| {
        app.update_render_surface_layouts(1.0, true);
        invoke_render_callbacks();
    };

    paint(&app);
    assert_eq!(surface.layout_size(), (120, 60));
    assert_eq!(calls.get(), 1, "shown: drawn");
    let before = asked();
    paint(&app);
    assert_eq!(calls.get(), 2);
    assert_eq!(
        asked(),
        before + 1,
        "control: a shown continuous surface asks for the next paint"
    );

    show.set(false);
    app.resolve_and_repaint(VP.0, VP.1);
    let before = asked();
    for _ in 0..3 {
        paint(&app);
    }
    assert_eq!(surface.layout_size(), (0, 0), "hidden: no size");
    assert_eq!(calls.get(), 2, "hidden: never run");
    assert_eq!(asked(), before, "hidden: no paint asked for");

    show.set(true);
    app.resolve_and_repaint(VP.0, VP.1);
    paint(&app);
    assert_eq!(
        surface.layout_size(),
        (120, 60),
        "shown again: its size is back"
    );
    assert_eq!(calls.get(), 3, "and it draws");

    clear_redraw_callback();
}
