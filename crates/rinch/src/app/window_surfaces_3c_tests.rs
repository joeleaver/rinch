//! #2416 stage 3c: a desktop window's paint drives and collects its own
//! window's render surfaces, and a surface's wake redraws the window that
//! shows it. Two apps stand for two windows, each with a `RenderSurface`
//! component and a named viewport (a `GameViewport`'s path) of its own; the
//! steps are the desktop paint's (`paint_software` / `paint_gpu`).
//!
//! The GPU-only collectors (`readback_gpu_textures`,
//! `collect_gpu_surface_frames`, `collect_texture_sources`,
//! `collect_surface_frames_shared` on a GPU compositor) take the same `ids`
//! filter; they run in the `#[ignore]`d GPU fixtures (`gpu_`), on lavapipe.

use super::*;
use crate::render_surface::{
    REDRAW_CALLBACK_TEST_LOCK, RenderSurface, RenderSurfaceHandle, clear_redraw_callback,
    collect_surface_pixels_by_id, collect_viewport_frames_by_name, create_render_surface,
    create_render_surface_with_name, forget_window_surfaces, invoke_window_render_callbacks,
    note_window_surfaces, set_redraw_callback, unregister_render_surface,
};
use rinch_core::Component;
use rinch_core::reactive::Signal;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const VP: (f32, f32) = (400.0, 300.0);

/// One window's app: a 120x60 `RenderSurface` (shown while `show` is true)
/// and a 100x50 `data-viewport` node named `viewport`.
struct Window {
    app: RinchApp,
    surface: RenderSurfaceHandle,
    viewport: RenderSurfaceHandle,
    show: Signal<bool>,
    calls: Rc<Cell<u32>>,
}

fn window(viewport_name: &'static str) -> Window {
    let surface = create_render_surface();
    let viewport = create_render_surface_with_name(viewport_name);
    let calls: Rc<Cell<u32>> = Rc::default();
    {
        let c = calls.clone();
        surface.set_render_callback(move |_w, _x, _y| c.set(c.get() + 1));
    }
    let show = Signal::new(true);
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let wrap = scope.create_element("div");
        wrap.set_attribute("style", "width: 120px; height: 60px;");
        root.append_child(&wrap);
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
        let named = scope.create_element("div");
        named.set_attribute("style", "width: 100px; height: 50px;");
        named.set_attribute("data-viewport", viewport_name);
        root.append_child(&named);
        root
    });
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    Window {
        app,
        surface,
        viewport,
        show,
        calls,
    }
}

fn frame(surface: &RenderSurfaceHandle) {
    surface
        .writer()
        .submit_frame(&[10, 20, 30, 255].repeat(4 * 2), 4, 2);
}

fn done(w: &Window) {
    unregister_render_surface(w.surface.id());
    unregister_render_surface(w.viewport.id());
}

#[test]
fn a_window_s_surfaces_are_the_ones_in_its_document() {
    let mut a = window("viewport-3c-a1");
    let mut b = window("viewport-3c-b1");
    let ids_a = a.app.window_surface_ids(1.0);
    let ids_b = b.app.window_surface_ids(1.0);
    assert!(ids_a.contains(&a.surface.id()) && ids_a.contains(&a.viewport.id()));
    assert!(!ids_a.contains(&b.surface.id()) && !ids_a.contains(&b.viewport.id()));
    assert!(ids_b.contains(&b.surface.id()) && ids_b.contains(&b.viewport.id()));
    assert!(!ids_b.contains(&a.surface.id()) && !ids_b.contains(&a.viewport.id()));
    done(&a);
    done(&b);
}

#[test]
fn a_window_s_paint_runs_only_its_own_render_callbacks() {
    let mut a = window("viewport-3c-a2");
    let mut b = window("viewport-3c-b2");
    let ids_a = a.app.window_surface_ids(1.0);
    let _ = b.app.window_surface_ids(1.0);
    invoke_window_render_callbacks(&ids_a);
    assert_eq!(a.calls.get(), 1);
    assert_eq!(
        b.calls.get(),
        0,
        "the other window's callback is its own paint's"
    );
    done(&a);
    done(&b);
}

#[test]
fn a_window_s_paint_collects_only_its_own_frames_and_leaves_the_other_s_new() {
    let mut a = window("viewport-3c-a3");
    let mut b = window("viewport-3c-b3");
    let ids_a = a.app.window_surface_ids(1.0);
    let ids_b = b.app.window_surface_ids(1.0);
    for s in [&a.surface, &a.viewport, &b.surface, &b.viewport] {
        frame(s);
    }

    let pixels = collect_surface_pixels_by_id(&ids_a);
    assert!(pixels.contains_key(&a.surface.id()));
    assert!(!pixels.contains_key(&b.surface.id()));
    let frames = collect_viewport_frames_by_name(&ids_a);
    assert!(frames.frames.contains_key("viewport-3c-a3"));
    assert!(!frames.frames.contains_key("viewport-3c-b3"));

    // The other window's frames are still new when its own paint comes.
    assert!(
        crate::render_surface::is_surface_dirty_by_id(b.surface.id()),
        "not taken by the first window's paint"
    );
    let frames = collect_viewport_frames_by_name(&ids_b);
    assert!(frames.fresh.contains("viewport-3c-b3"));
    assert!(collect_surface_pixels_by_id(&ids_b).contains_key(&b.surface.id()));
    done(&a);
    done(&b);
}

#[test]
fn a_window_forgets_only_the_sizes_of_surfaces_that_left_it() {
    let mut a = window("viewport-3c-a4");
    let mut b = window("viewport-3c-b4");
    let _ = a.app.window_surface_ids(1.0);
    let _ = b.app.window_surface_ids(1.0);
    assert_eq!(a.surface.layout_size(), (120, 60));
    assert_eq!(b.surface.layout_size(), (120, 60));

    a.show.set(false);
    a.app.resolve_and_repaint(VP.0, VP.1);
    let ids_a = a.app.window_surface_ids(1.0);
    assert!(!ids_a.contains(&a.surface.id()));
    assert_eq!(a.surface.layout_size(), (0, 0), "it left this window");
    assert_eq!(
        b.surface.layout_size(),
        (120, 60),
        "the other window's is kept"
    );
    done(&a);
    done(&b);
}

#[test]
fn a_surface_s_wake_redraws_the_window_that_shows_it() {
    let _lock = REDRAW_CALLBACK_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut a = window("viewport-3c-a5");
    let mut b = window("viewport-3c-b5");
    let ids_a = a.app.window_surface_ids(1.0);
    let ids_b = b.app.window_surface_ids(1.0);
    note_window_surfaces(None, &ids_a);
    note_window_surfaces(Some(705), &ids_b);
    let redraws_a = Arc::new(AtomicUsize::new(0));
    let redraws_b = Arc::new(AtomicUsize::new(0));
    {
        let r = redraws_a.clone();
        set_redraw_callback(
            None,
            Arc::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let r = redraws_b.clone();
        set_redraw_callback(
            Some(705),
            Arc::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        );
    }
    frame(&b.surface);
    assert_eq!(redraws_b.load(Ordering::SeqCst), 1);
    assert_eq!(redraws_a.load(Ordering::SeqCst), 0, "not the main window");
    frame(&a.viewport);
    assert_eq!(redraws_a.load(Ordering::SeqCst), 1);
    assert_eq!(redraws_b.load(Ordering::SeqCst), 1);

    // Hidden (no callback): the wake redraws neither window.
    clear_redraw_callback(Some(705));
    frame(&b.surface);
    assert_eq!(
        (
            redraws_a.load(Ordering::SeqCst),
            redraws_b.load(Ordering::SeqCst)
        ),
        (1, 1),
        "a hidden window's surface wakes nothing"
    );
    // Closed: its surface is shown nowhere, and wakes the main window.
    forget_window_surfaces(Some(705));
    frame(&b.surface);
    assert_eq!(redraws_a.load(Ordering::SeqCst), 2, "shown nowhere: main");

    clear_redraw_callback(None);
    done(&a);
    done(&b);
}
