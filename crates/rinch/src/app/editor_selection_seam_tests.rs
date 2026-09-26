//! A selection across a wrapped paragraph at a fractional line-height paints
//! one even wash, with no darker seam between two lines (#1024).
//!
//! The highlight is one translucent `rgba(26,115,232,0.3)` div per line.
//! Parley quantizes each line's block coords on its own, so two neighbouring
//! lines' boxes could overlap by a pixel — at 15px x 1.65 on Inter, `50..75`
//! and `74..99` — and the row both covered was painted twice: a darker line
//! across the selection. (At other line-heights the boxes left a one-row gap
//! instead, an unpainted line.) Chrome paints a contiguous highlight.
//!
//! This is a local pixel oracle: the text is `color: transparent`, so under
//! the highlight's leftmost column the only ink is the wash, and every row of
//! it must be the same colour. The face is the bundled Inter and the
//! `font-size` and `line-height` are declared, so which rows collide is not a
//! property of the host's fonts.

use super::hit_testing::painted_element_box;
use super::*;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                    lambda mu nu xi omicron pi rho sigma tau upsilon";

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

/// A painted box, `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

fn highlights(app: &RinchApp) -> Vec<Rect> {
    let doc = app.doc.as_ref().unwrap().borrow();
    doc.query_selector_all("[data-pm-selection]")
        .into_iter()
        .filter(|id| {
            doc.tree.nodes[id.0].computed_style.display
                != rinch_dom::computed_style::DisplayValue::None
        })
        .map(|id| painted_element_box(&doc.tree, id.0))
        .filter(|b| b.2 > 0.0 && b.3 > 0.0)
        .collect()
}

/// Select all of a wrapped paragraph set at `font` and return, top to bottom,
/// the colour of every row under the highlight's leftmost column, with the
/// highlight rects for the message.
fn wash_column(font: &str) -> (Vec<[u8; 3]>, Vec<Rect>) {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p>{TEXT}</p>")));
    let handle_in = handle.clone();
    let style = format!(
        "width: 160px; {font}; font-family: sans-serif; color: transparent; \
         background: white"
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; background: white");
        let editor = handle_in.mount(scope);
        editor.set_attribute("style", &style);
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    handle.focus();
    idle(&mut app);
    assert!(handle.command("selectAll"), "positive control");
    idle(&mut app);
    app.has_previous_frame = false;
    let rects = highlights(&app);
    assert!(rects.len() >= 4, "{font}: the paragraph wraps: {rects:?}");
    let col = rects.iter().map(|r| r.0).fold(f32::MAX, f32::min).round() as usize;
    let top = rects.iter().map(|r| r.1).fold(f32::MAX, f32::min).round() as usize;
    let bottom = rects
        .iter()
        .map(|r| r.1 + r.3)
        .fold(f32::MIN, f32::max)
        .round() as usize;
    let (px, w, _) = app.build_pixels(1.0, VP, false);
    let rows = (top..bottom)
        .map(|y| {
            let i = (y * w as usize + col) * 4;
            [px[i], px[i + 1], px[i + 2]]
        })
        .collect();
    (rows, rects)
}

#[track_caller]
fn assert_even_wash(font: &str) {
    let (rows, rects) = wash_column(font);
    let first = rows[0];
    assert_ne!(first, [255, 255, 255], "{font}: the wash is painted");
    for (y, c) in rows.iter().enumerate() {
        assert_eq!(
            *c, first,
            "{font}: row {y} (counted from the highlight's top) of the highlight is {c:?}, the rest {first:?} — a \
             seam where two lines' highlights overlap (darker) or leave a gap \
             (white). Rects: {rects:?}"
        );
    }
}

#[test]
fn no_seam_at_15px_x_1_65() {
    assert_even_wash("font-size: 15px; line-height: 1.65");
}

#[test]
fn no_seam_at_16px_x_1_65() {
    assert_even_wash("font-size: 16px; line-height: 1.65");
}

#[test]
fn no_seam_at_13px_x_1_37() {
    assert_even_wash("font-size: 13px; line-height: 1.37");
}

/// The line before a Shift+Enter break is highlighted to its text plus a
/// newline, not as a 1px sliver (review of #1108; Chrome 153 paints `abc` +
/// its newline across `0..32`).
#[test]
fn the_line_before_a_hard_break_is_highlighted() {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>abc<br>def ghi</p><p>second</p>"));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; background: white");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 160px; font-size: 16px; line-height: 40px; font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    handle.focus();
    idle(&mut app);
    assert!(handle.command("selectAll"), "positive control");
    idle(&mut app);
    let rects = highlights(&app);
    assert_eq!(rects.len(), 3, "three lines: {rects:?}");
    assert!(
        rects.iter().all(|r| r.2 > 25.0),
        "no line is a sliver: {rects:?}"
    );
}
