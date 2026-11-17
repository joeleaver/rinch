//! #2633, from the review of #2646: a window prepared and then changed
//! (content, size, scale; prepared again; shown and hidden first) shows
//! exactly the pixels an unprepared twin shows. The preparation saves work;
//! it never changes what the show paints.
use super::*;
use crate as rinch;
use crate::shell::renderer::Renderer;
use rinch_core::Signal;
use rinch_core::element::WindowProps;
use rinch_macros::rsx;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 14px; line-height: 20px; }
    .row { height: 32px; display: flex; align-items: center; }
    .row.on { background: rgb(40, 80, 200); color: white; }
    .wide { width: 100%; height: 10px; background: rgb(200,0,0); }
";

fn app_fn(h: Signal<usize>) -> impl FnOnce(&mut RenderScope) -> NodeHandle {
    move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style { {CSS} }
                div { class: "wide" }
                div { class: {move || if h.get() == 0 { "row on" } else { "row" }}, "Files" }
                div { class: {move || if h.get() == 1 { "row on" } else { "row" }}, {|| format!("Terminal {}", h.get())} }
            }
        }
    }
}

fn hidden(shown: (u32, u32)) -> (RinchRuntime, Signal<usize>) {
    let h = Signal::new(0);
    let props = WindowProps {
        width: 800,
        height: 600,
        visible: false,
        ..Default::default()
    };
    let mut rt = build_runtime(
        app_fn(h),
        &props,
        None,
        &[crate::font::AppFont::sans_serif(INTER)],
    );
    rt.main.app.set_device_pixel_ratio(1.0);
    rt.main.app.mount_component(800.0, 600.0);
    rt.main.hide_window(None);
    rt.main.saved_geometry.size = Some(Size::Logical(LogicalSize::new(
        shown.0 as f64,
        shown.1 as f64,
    )));
    (rt, h)
}

fn show(rt: &mut RinchRuntime, size: (u32, u32), scale: f64) -> Vec<u8> {
    if (rt.main.app.device_pixel_ratio - scale).abs() > 1e-9 {
        rt.main.app.set_device_pixel_ratio(scale);
    }
    let phys = (
        (size.0 as f64 * scale) as u32,
        (size.1 as f64 * scale) as u32,
    );
    let _ = rt.main.app.handle_event(
        PlatformEvent::Resized {
            width: size.0,
            height: size.1,
        },
        phys,
        scale,
    );
    rt.main.repaint_whole_after_show();
    rt.main.paint_software_at(scale, size).unwrap();
    let _ = rt.main.app.end_perf_frame();
    rt.main.app.skia_painter.as_ref().unwrap().pixels().to_vec()
}

fn same(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: pixmap sizes differ");
    let diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
    assert_eq!(
        diff, 0,
        "{what}: {diff} bytes differ from the unprepared show"
    );
}

#[test]
fn signal_changed_after_prepare() {
    let (mut a, ha) = hidden((640, 420));
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    ha.set(1);
    let pa = show(&mut a, (640, 420), 1.0);
    let (mut b, hb) = hidden((640, 420));
    hb.set(1);
    let pb = show(&mut b, (640, 420), 1.0);
    same(&pa, &pb, "signal after prepare");
}

#[test]
fn resized_after_prepare() {
    let (mut a, _) = hidden((640, 420));
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    a.main.saved_geometry.size = Some(Size::Logical(LogicalSize::new(500.0, 300.0)));
    let pa = show(&mut a, (500, 300), 1.0);
    let (mut b, _) = hidden((500, 300));
    let pb = show(&mut b, (500, 300), 1.0);
    same(&pa, &pb, "resize after prepare");
}

#[test]
fn scale_changed_after_prepare() {
    let (mut a, _) = hidden((640, 420));
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let pa = show(&mut a, (640, 420), 2.0);
    let (mut b, _) = hidden((640, 420));
    let pb = show(&mut b, (640, 420), 2.0);
    same(&pa, &pb, "scale after prepare");
}

#[test]
fn prepare_mutate_prepare_show() {
    let (mut a, ha) = hidden((640, 420));
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    ha.set(1);
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    a.main.saved_geometry.size = Some(Size::Logical(LogicalSize::new(700.0, 400.0)));
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let pa = show(&mut a, (700, 400), 1.0);
    let (mut b, hb) = hidden((700, 400));
    hb.set(1);
    let pb = show(&mut b, (700, 400), 1.0);
    same(&pa, &pb, "prepare twice with changes");
}

#[test]
fn shown_hidden_prepared_shown_again() {
    let (mut a, ha) = hidden((640, 420));
    let _ = show(&mut a, (640, 420), 1.0);
    ha.set(1);
    a.main.prepare_hidden_as(Renderer::Software, Some(1.0));
    let pa = show(&mut a, (640, 420), 1.0);
    let (mut b, hb) = hidden((640, 420));
    let _ = show(&mut b, (640, 420), 1.0);
    hb.set(1);
    let pb = show(&mut b, (640, 420), 1.0);
    same(&pa, &pb, "show, mutate, prepare, show");
}
