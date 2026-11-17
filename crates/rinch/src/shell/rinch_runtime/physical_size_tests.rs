//! `rinch::window::window_physical_size`: what a window's surface was
//! configured at, published from the shell's `SurfaceResized`, not the
//! logical size times the scale recomputed. Driven through the runtime
//! without an OS window, as the shell's event path handles the event.

use super::*;
use crate::window::{window_physical_size, window_size};
use winit::dpi::PhysicalSize;

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// A runtime with its main window mounted at scale `scale`.
fn runtime(scale: f64) -> RinchRuntime {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.set_device_pixel_ratio(scale);
    runtime.main.app.mount_component(800.0, 600.0);
    runtime
}

/// The shell's handling of a `SurfaceResized` to `physical` at `scale`: the
/// window's part, then the `Resized` it becomes, handed to the document.
fn surface_resized(runtime: &mut RinchRuntime, physical: (u32, u32), scale: f64) {
    let event = runtime
        .main
        .surface_resized(PhysicalSize::new(physical.0, physical.1), scale);
    let _ = runtime.main.app.handle_event(event, physical, scale);
}

/// At 1.25 a surface 1282 pixels wide is laid out 1026 logical pixels wide
/// (1282 / 1.25 = 1025.6), and 1026 times 1.25 is 1282.5, which rounds to
/// 1283: a recomputation from the logical size is a pixel wider than the
/// surface. The getter answers the surface's own 1282.
#[test]
fn at_a_fractional_scale_it_is_the_surface_and_not_a_recomputation() {
    let scale = 1.25;
    let mut runtime = runtime(scale);
    surface_resized(&mut runtime, (1282, 721), scale);

    assert_eq!(
        window_size(),
        (1026.0, 577.0),
        "laid out at the logical size"
    );
    let recomputed = (window_size().0 * scale).round() as u32;
    assert_eq!(recomputed, 1283, "the recomputation the getter must not be");
    assert_eq!(
        window_physical_size(),
        (1282, 721),
        "the surface's own size"
    );
}

/// A reader of the physical size runs again when the surface is resized,
/// once per resize, with the logical size it is laid out at in the same run;
/// an equal size wakes nothing.
#[test]
fn a_resize_reruns_a_reader() {
    let mut runtime = runtime(1.0);
    surface_resized(&mut runtime, (800, 600), 1.0);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _effect = {
        let seen = seen.clone();
        rinch_core::Effect::new(move || {
            seen.borrow_mut()
                .push((window_physical_size(), window_size()));
        })
    };
    surface_resized(&mut runtime, (1600, 900), 1.0);
    surface_resized(&mut runtime, (1600, 900), 1.0);
    runtime.main.app.set_device_pixel_ratio(2.0);
    surface_resized(&mut runtime, (3200, 1800), 2.0);
    assert_eq!(
        *seen.borrow(),
        [
            ((800, 600), (800.0, 600.0)),
            ((1600, 900), (1600.0, 900.0)),
            ((3200, 1800), (1600.0, 900.0)),
        ],
        "one run per change, none for an equal size; a new scale's surface \
         is followed though the logical size stays"
    );
}

/// The size is the mounted window's from its creation: what the shell set
/// before the mount is published with it.
#[test]
fn the_mount_publishes_the_surface_size_set_before_it() {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.set_device_pixel_ratio(1.5);
    runtime.main.app.set_surface_size((1201, 900));
    runtime.main.app.mount_component(801.0, 600.0);
    assert_eq!(window_physical_size(), (1201, 900));
}

// ── The software renderer's buffer is the surface (#2786) ────────────────

/// The root background, and a box at the top right whose colour a signal
/// sets: what reaches the right edge after a partial repaint.
const BG: [u8; 4] = [20, 40, 60, 255];
const SIDE_RED: [u8; 4] = [200, 0, 0, 255];
const SIDE_GREEN: [u8; 4] = [0, 160, 0, 255];
const EDGE_CSS: &str = "
    body { margin: 0; background: rgb(20, 40, 60); }
    .side { position: absolute; top: 0; right: 0; width: 50px; height: 100px; }
    .red { background: rgb(200, 0, 0); }
    .green { background: rgb(0, 160, 0); }
";

fn edge_component(green: rinch_core::Signal<bool>) -> impl FnOnce(&mut RenderScope) -> NodeHandle {
    move |__scope: &mut RenderScope| {
        rinch_macros::rsx! {
            div {
                style { {EDGE_CSS} }
                div { class: {move || if green.get() { "side green" } else { "side red" }} }
            }
        }
    }
}

/// A runtime whose main window's surface is `surface` at `scale`, laid out
/// at that surface's logical size and painted by the software renderer as
/// the shell paints a window: at the window's scale and logical size.
fn painted(scale: f64, surface: (u32, u32)) -> (RinchRuntime, rinch_core::Signal<bool>) {
    let green = rinch_core::Signal::new(false);
    let mut rt = RinchRuntime::new("t", 800, 600, edge_component(green), &[]);
    rt.main.app.set_device_pixel_ratio(scale);
    rt.main.app.mount_component(800.0, 600.0);
    surface_resized(&mut rt, surface, scale);
    let _ = paint(&mut rt, scale, surface);
    (rt, green)
}

fn paint(
    rt: &mut RinchRuntime,
    scale: f64,
    surface: (u32, u32),
) -> Option<rinch_dom::perf::FrameStats> {
    let logical = rinch_platform::to_logical(surface, scale);
    rt.main.paint_software_at(scale, logical).unwrap();
    rt.main.app.end_perf_frame()
}

type Buffer = (Vec<u8>, u32, u32);

fn buffer(rt: &RinchRuntime) -> Buffer {
    let p = rt.main.app.skia_painter.as_ref().expect("painted");
    (p.pixels().to_vec(), p.width(), p.height())
}

fn px(buf: &Buffer, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * buf.1 + x) * 4) as usize;
    buf.0[i..i + 4].try_into().unwrap()
}

/// The first few pixels of `at` that are not `want`, as (x, y, got).
fn off(
    buf: &Buffer,
    at: impl Iterator<Item = (u32, u32)>,
    want: [u8; 4],
) -> Vec<(u32, u32, [u8; 4])> {
    at.map(|(x, y)| (x, y, px(buf, x, y)))
        .filter(|p| p.2 != want)
        .take(4)
        .collect()
}

/// [`off`] over column `x`, `rows`.
fn off_column(
    buf: &Buffer,
    x: u32,
    rows: std::ops::Range<u32>,
    want: [u8; 4],
) -> Vec<(u32, u32, [u8; 4])> {
    off(buf, rows.map(|y| (x, y)), want)
}

/// [`off`] over row `y`, `cols`.
fn off_row(
    buf: &Buffer,
    y: u32,
    cols: std::ops::Range<u32>,
    want: [u8; 4],
) -> Vec<(u32, u32, [u8; 4])> {
    off(buf, cols.map(|x| (x, y)), want)
}

/// At 1.75 a surface of 1000 × 601 is laid out at 571 × 343 (1000 / 1.75 =
/// 571.4, 601 / 1.75 = 343.4), which is 999.25 × 600.25 device pixels. The
/// buffer used to be that, rounded (999 × 600): a column and a row short of
/// the surface. It is the surface, and its last column and row show the
/// root background, as a browser's 571.4 × 343.4 viewport paints them.
#[test]
fn at_1_75_the_buffer_is_the_surface_and_its_edges_are_painted() {
    let scale = 1.75;
    let (rt, _) = painted(scale, (1000, 601));
    let buf = buffer(&rt);
    assert_eq!(
        (buf.1, buf.2),
        (1000, 601),
        "the buffer is the surface's size"
    );
    // The controls: the document paints the root background and the box.
    assert_eq!(px(&buf, 0, 300), BG, "the root background");
    assert_eq!(px(&buf, 960, 50), SIDE_RED, "the box");
    assert_eq!(off_column(&buf, 999, 200..601, BG), [], "the last column");
    assert_eq!(off_row(&buf, 600, 0..1000, BG), [], "the last row");
    assert_eq!(
        off_column(&buf, 999, 0..170, SIDE_RED),
        [],
        "the box at the edge reaches the surface's"
    );
}

/// Below a scale of 1 the rounded product is the surface already (1000 at
/// 0.75 is laid out 1333 wide, 999.75 device pixels, which rounds to 1000),
/// but the document covers only three quarters of the last column, which
/// showed the white under it blended in. It shows the document's edge.
#[test]
fn below_1_the_last_column_and_row_are_painted_whole() {
    let scale = 0.75;
    let (rt, _) = painted(scale, (1000, 751));
    let buf = buffer(&rt);
    assert_eq!(
        (buf.1, buf.2),
        (1000, 751),
        "the buffer is the surface's size"
    );
    assert_eq!(px(&buf, 0, 300), BG, "the root background");
    // 751 / 0.75 = 1001.3, laid out 1001 rows high: 750.75 device rows.
    assert_eq!(off_column(&buf, 999, 100..751, BG), [], "the last column");
    assert_eq!(off_row(&buf, 750, 0..900, BG), [], "the last row");
}

/// A frame that repaints only the box at the edge leaves the surface's last
/// column the box's new colour where the box is, and the background below.
#[test]
fn a_partial_repaint_at_the_edge_paints_the_last_column_too() {
    let scale = 1.75;
    let (mut rt, green) = painted(scale, (1000, 601));
    green.set(true);
    let frame = paint(&mut rt, scale, (1000, 601)).expect("a frame");
    assert_eq!(
        frame.get(rinch_dom::perf::Counter::RepaintPartial),
        1,
        "a partial repaint: {frame:?}"
    );
    let buf = buffer(&rt);
    assert_eq!(
        px(&buf, 960, 50),
        SIDE_GREEN,
        "the control: the box repainted"
    );
    assert_eq!(
        off_column(&buf, 999, 0..170, SIDE_GREEN),
        [],
        "the box's new colour"
    );
    assert_eq!(
        off_column(&buf, 999, 200..601, BG),
        [],
        "the background below it"
    );
}
