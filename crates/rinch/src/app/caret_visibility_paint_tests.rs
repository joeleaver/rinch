//! The caret is hidden with `visibility: hidden` now, not
//! `display: none`. Its box stays in layout, so what keeps it off the screen is
//! paint honouring `visibility` — on a full frame AND on the dirty-region frame
//! a running app paints next. Local oracle: the caret's exact colour
//! (`#1a73e8`, opaque) appears nowhere else in this frame.
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (400, 200);

fn caret_px(px: &[u8]) -> usize {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255 && p[0] == 26 && p[1] == 115 && p[2] == 232)
        .count()
}

fn frame(app: &mut RinchApp, full: bool) -> usize {
    app.refresh_editor_overlays();
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.scene_dirty = true;
    if full {
        app.has_previous_frame = false;
    }
    let (px, _, _) = app.build_pixels(1.0, VP, true);
    caret_px(px)
}

#[test]
fn a_hidden_caret_paints_nothing_on_a_full_or_an_incremental_frame() {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>hello world</p>"));
    let h = handle.clone();
    let ed = Rc::new(std::cell::Cell::new(0usize));
    let ed_in = ed.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 20px; font-size: 16px; line-height: 20px");
        let e = h.mount(scope);
        ed_in.set(e.node_id().0);
        root.append_child(&e);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.focus_target = FocusTarget::Editor(ed.get());
    handle.set_selection(Selection::cursor(Pos(3)));

    let shown = frame(&mut app, true);
    assert!(shown > 0, "positive control: a shown caret is painted");

    assert_eq!(handle.set_caret_blink(false), Some(true));
    assert_eq!(frame(&mut app, false), 0, "blink-off, next frame");
    assert_eq!(frame(&mut app, true), 0, "blink-off, full frame");

    assert_eq!(handle.set_caret_blink(true), Some(true));
    assert!(frame(&mut app, false) > 0, "blink-on, next frame");

    // Blur: the focus-aware overlay pass hides every overlay of an editor
    // that is not focused (`hide_overlays` -> `hide_caret`).
    app.focus_target = FocusTarget::None;
    assert_eq!(frame(&mut app, false), 0, "hidden on blur, next frame");
    assert_eq!(frame(&mut app, true), 0, "hidden on blur, full frame");
}
