//! #2749: a font registered after the first paint changes text already on
//! screen, and the desktop frames must repaint it. A registration changes no
//! computed style and pushes no node: `note_fonts_registered` drops every
//! shaped layout and measure for the *next* layout, but the desktop shell
//! runs a frame only when a node is dirty (`App::has_dirty_nodes`, and the
//! short-circuit at the top of `resolve_and_repaint`), so that layout never
//! ran: the software frame (`build_pixels`) kept its pixels and the GPU frame
//! (`build_gpu_frame`) came back `Unchanged`, the text in the fallback face.
//! The page is sized well inside the window so the frames are partial: each
//! text the layout re-shapes has to name its own damage.

use super::*;

const SIZE: (u32, u32) = (900, 700);

/// A line set in `Inter` (not installed on the host, so a fallback face draws
/// it until Inter is registered) in a box whose size no face changes, and a
/// second one overflowing its 60px box (Inter Bold's `W` is far wider than
/// the fallback's), with its text node.
fn app() -> (RinchApp, Rc<RefCell<Option<NodeHandle>>>) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "width: 320px; height: 220px; background: #fff; \
             font: 32px/40px Inter, monospace; color: #000",
        );
        let line = scope.create_element("div");
        line.set_attribute("style", "width: 300px; height: 40px; margin: 10px");
        line.append_child(&scope.create_text("Lap 3/7 Wqg"));
        root.append_child(&line);
        let wide = scope.create_element("div");
        wide.set_attribute(
            "style",
            "width: 60px; height: 40px; margin: 10px; white-space: nowrap",
        );
        let text = scope.create_text("WWWWWWW");
        wide.append_child(&text);
        root.append_child(&wide);
        // A flex item's text, measured as a Taffy leaf of its own (no IFC
        // root), in a row whose size no face changes.
        let row = scope.create_element("div");
        row.set_attribute(
            "style",
            "display: flex; width: 300px; height: 40px; margin: 10px",
        );
        row.append_child(&scope.create_text("Wqg 3/7"));
        root.append_child(&row);
        // A fixed-size inline-block's text, and a list marker.
        let list = scope.create_element("ol");
        list.set_attribute("style", "margin: 0 0 0 60px; padding: 0");
        let item = scope.create_element("li");
        let chip = scope.create_element("span");
        chip.set_attribute(
            "style",
            "display: inline-block; width: 120px; height: 40px; overflow: hidden",
        );
        chip.append_child(&scope.create_text("Wq 1/2"));
        item.append_child(&chip);
        list.append_child(&item);
        root.append_child(&list);
        *slot_in.borrow_mut() = Some(text);
        root
    });
    (app, slot)
}

fn inter_bold() -> &'static [u8] {
    include_bytes!("../../assets/fonts/Inter-Bold.ttf").as_slice()
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 1))
        .count()
}

/// What a fresh app with Inter registered before its first paint draws, with
/// the wide line's text set to `wide`.
#[cfg(software_shell)]
fn fresh_pixels(wide: &str) -> Vec<u8> {
    let (mut app, text) = app();
    app.register_font_data(inter_bold());
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    text.borrow().as_ref().unwrap().set_text(wide);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    app.build_pixels(1.0, SIZE, false).0.to_vec()
}

/// The desktop software frame: after a late registration the next frame
/// draws what a fresh app with the font draws, and so does a later partial
/// frame: the damage of the overflowing line, shortened, covers the ink the
/// new face drew (recorded by the late font's frame), not the old face's.
#[cfg(software_shell)]
#[test]
fn a_late_font_repaints_text_on_the_software_frame() {
    let (mut app, text) = app();
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let before = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.build_pixels(1.0, SIZE, false);
    let want = fresh_pixels("WWWWWWW");
    assert!(
        differing(&before, &want) > 500,
        "positive control: the fallback face draws other pixels than Inter"
    );

    app.register_font_data(inter_bold());
    // The shell's loop: a frame runs when a node is dirty.
    assert!(
        app.has_dirty_nodes(),
        "a late font leaves nothing for the loop to wake for"
    );
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let got = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let bad = differing(&got, &want);
    assert_eq!(bad, 0, "{bad} stale pixels after a late font");

    text.borrow().as_ref().unwrap().set_text("i");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let got = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let bad = differing(&got, &fresh_pixels("i"));
    assert_eq!(
        bad, 0,
        "{bad} stale pixels after the overflowing line changed"
    );
}

/// The desktop GPU frame decides its damage the same way (`frame_damage`):
/// a late registration must not leave it `Unchanged` or a partial frame
/// that misses the text (every line's box, and the overflowing line's ink).
/// Needs no adapter: this is what the frame hands the renderer.
#[cfg(all(software_shell, feature = "gpu-renderer"))]
#[test]
fn a_late_font_repaints_text_on_the_gpu_frame() {
    use rinch_dom::perf::FullRepaintReason;
    let (mut app, _) = app();
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    assert!(matches!(
        app.build_gpu_frame(1.0, SIZE, Some(FullRepaintReason::FirstFrame)),
        GpuFrame::Full(_)
    ));
    assert!(matches!(
        app.build_gpu_frame(1.0, SIZE, None),
        GpuFrame::Unchanged
    ));

    app.register_font_data(inter_bold());
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    match app.build_gpu_frame(1.0, SIZE, None) {
        GpuFrame::Full(_) => {}
        GpuFrame::Partial { rects, .. } => {
            // The text: x 10..310 (the wide line's ink reaches past its 60px
            // box), y 10..200, every 5th device pixel.
            let covered = |x: u32, y: u32| {
                rects
                    .iter()
                    .any(|&(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
            };
            let missed: Vec<(u32, u32)> = (10..310)
                .step_by(5)
                .flat_map(|x| (10..200).step_by(5).map(move |y| (x, y)))
                .filter(|&(x, y)| !covered(x, y))
                .collect();
            assert!(missed.is_empty(), "{rects:?} misses {missed:?}");
        }
        GpuFrame::Unchanged => panic!("an unchanged frame after a late font"),
    }
}
