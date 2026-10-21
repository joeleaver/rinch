//! A read-only selection highlight that stands out of its box is repainted
//! where it stands (review of #1706).
//!
//! Under a `line-height` smaller than the text, each line's highlight is its
//! content area (#1008), so the first and last lines' rects reach above and
//! below the IFC root's box — what Chrome 155 paints. The highlight is the
//! node's own paint, and its damage named only the box, so a partial frame
//! left rows of wash behind when the selection was cleared and missed rows of
//! it when the selection was made. Each case compares a partial frame with a
//! full repaint of the same state, pixel for pixel.
use super::*;

const SIZE: (u32, u32) = (600, 400);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                    lambda mu nu xi omicron pi rho sigma tau upsilon";

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn partial_frame(app: &mut RinchApp) -> Vec<u8> {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let s = app.end_perf_frame().expect("mounted");
    assert_eq!(
        s.get(rinch_dom::perf::Counter::RepaintFull),
        0,
        "a partial frame"
    );
    assert_eq!(
        s.get(rinch_dom::perf::Counter::RepaintPartial),
        1,
        "a partial frame"
    );
    px
}

fn differing_rows(a: &[u8], b: &[u8]) -> Vec<usize> {
    let w = SIZE.0 as usize * 4;
    (0..SIZE.1 as usize)
        .filter(|y| {
            a[y * w..(y + 1) * w]
                .iter()
                .zip(&b[y * w..(y + 1) * w])
                .any(|(p, q)| (*p as i32 - *q as i32).abs() > 2)
        })
        .collect()
}

#[track_caller]
fn assert_partial_frames_are_whole(font: &str) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let s2 = slot.clone();
    let style = format!(
        "position: absolute; left: 20px; top: 150px; width: 160px; \
         font-family: sans-serif; {font}; color: transparent"
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px; background: white");
        let d = scope.create_element("div");
        d.set_attribute("style", &style);
        let t = scope.create_text(TEXT);
        d.append_child(&t);
        outer.append_child(&d);
        *s2.borrow_mut() = Some(d);
        outer
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let d = slot.borrow().clone().unwrap();
    let before = full_frame(&mut app);

    d.set_attribute("data-text-sel", "true");
    d.set_attribute("data-text-sel-start", "0");
    d.set_attribute("data-text-sel-end", &TEXT.len().to_string());
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let partial = partial_frame(&mut app);
    let full = full_frame(&mut app);
    assert_ne!(
        full, before,
        "{font}: positive control: the highlight paints"
    );
    assert_eq!(
        differing_rows(&partial, &full),
        Vec::<usize>::new(),
        "{font}: rows of the new highlight a partial frame missed"
    );

    d.set_attribute("data-text-sel", "false");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let partial = partial_frame(&mut app);
    let full = full_frame(&mut app);
    assert_eq!(
        differing_rows(&partial, &full),
        Vec::<usize>::new(),
        "{font}: rows of the cleared highlight a partial frame left behind"
    );
}

#[test]
fn a_highlight_over_negative_leading_is_repainted_at_16px_over_10px() {
    assert_partial_frames_are_whole("font-size: 16px; line-height: 10px");
}

#[test]
fn a_highlight_over_negative_leading_is_repainted_at_32px_x_1() {
    assert_partial_frames_are_whole("font-size: 32px; line-height: 1");
}

#[test]
fn a_highlight_over_negative_leading_is_repainted_at_48px_x_1() {
    assert_partial_frames_are_whole("font-size: 48px; line-height: 1");
}
