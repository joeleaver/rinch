//! MCP `get_caret_position` / `get_glyph_bounds` answer where the caret and
//! the glyphs are **painted** (#1136), read off the pixels of a software
//! frame at scale 1 and at scale 2.
//!
//! The two tools used to keep their own copy of the text geometry, and it had
//! drifted from paint's: no single-line centring, the value measured where a
//! password field paints bullets, a textarea wrapped at its border box, and
//! IFC text placed at the border box rather than the content box (a padded
//! `<p>` was off by its padding). They now ask the app's painter-matched
//! helpers, so the oracle here is the frame itself, never those helpers.
//!
//! Every field is `border: 3px solid transparent` over a transparent page and
//! its text is pure blue, so any opaque pixel is the field's own ink. The
//! caret is found as the columns whose ink runs at least 13 logical px tall:
//! paint draws it `1.2 × font-size` (16.8px at 14px) tall, and no glyph of the
//! texts used here (no ascender letter taller than a capital) runs that far.
//! Text is set in the bundled Inter.

use super::*;
use std::cell::Cell;

const VP: (u32, u32) = (500, 400);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

fn page(
    tag: &'static str,
    ty: Option<&'static str>,
    style: impl Into<String>,
    value: &'static str,
) -> (RinchApp, u64) {
    let id: Rc<Cell<u64>> = Rc::default();
    let id_in = id.clone();
    let style: String = style.into();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 500px; height: 400px; margin: 0; \
             font-family: sans-serif; font-size: 14px; line-height: 20px",
        );
        let el = scope.create_element(tag);
        el.set_attribute("style", &style);
        if let Some(ty) = ty {
            el.set_attribute("type", ty);
        }
        if tag == "p" {
            el.append_child(&scope.create_text(value));
        } else {
            el.set_attribute("value", value);
        }
        root.append_child(&el);
        id_in.set(el.node_id().0);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, id.get())
}

fn query(app: &mut RinchApp, kind: DebugCommandKind, scale: f64) -> serde_json::Value {
    let mut actions = Vec::new();
    let size = ((VP.0 as f64 * scale) as u32, (VP.1 as f64 * scale) as u32);
    match app.execute_debug_command(kind, &mut actions, scale, size) {
        DebugResult::Json { data } => data,
        other => panic!("expected JSON, got {other:?}"),
    }
}

fn mcp_caret(app: &mut RinchApp, node_id: u64, byte_offset: usize, scale: f64) -> (f64, f64) {
    let v = query(
        app,
        DebugCommandKind::GetCaretPosition {
            node_id,
            byte_offset,
        },
        scale,
    );
    (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap())
}

fn mcp_glyph(app: &mut RinchApp, node_id: u64, byte_offset: usize, scale: f64) -> [f64; 4] {
    let v = query(
        app,
        DebugCommandKind::GetGlyphBounds {
            node_id,
            byte_offset,
        },
        scale,
    );
    ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap())
}

/// A full software frame at `scale`: `(pixels, width in device px)`.
fn frame(app: &mut RinchApp, scale: f64) -> (Vec<u8>, usize) {
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let (px, w, _) = app.build_pixels(scale, VP, true);
    (px.to_vec(), w as usize)
}

fn alpha(px: &[u8], w: usize, x: usize, y: usize) -> f64 {
    px[(y * w + x) * 4 + 3] as f64 / 255.0
}

/// The painted caret's top-left in logical px, with field `id` marked focused
/// and its caret at `byte` (the attributes the runtime's caret sync writes,
/// which paint reads), from a frame at `scale`.
fn painted_caret(app: &mut RinchApp, id: u64, byte: usize, scale: f64) -> (f64, f64) {
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let n = d.tree.nodes.get_mut(id).unwrap();
        n.attributes.insert("data-focused".into(), "true".into());
        n.attributes
            .insert("data-cursor-pos".into(), byte.to_string());
        n.attributes
            .insert("data-selection-start".into(), byte.to_string());
        d.tree.mark_paint_dirty(id);
    }
    let (px, w) = frame(app, scale);
    let h = px.len() / 4 / w;
    let tall = (13.0 * scale) as usize;
    // Columns holding a vertical run of ink (coverage > 1/2) at least `tall`.
    let mut cols: Vec<(usize, usize)> = Vec::new(); // (column, first row of the run)
    for x in 0..w {
        let (mut run, mut start) = (0, 0);
        for y in 0..h {
            if alpha(&px, w, x, y) > 0.5 {
                if run == 0 {
                    start = y;
                }
                run += 1;
                if run >= tall {
                    cols.push((x, start));
                    break;
                }
            } else {
                run = 0;
            }
        }
    }
    assert!(!cols.is_empty(), "positive control: a caret is painted");
    let (first, last) = (cols[0].0, cols[cols.len() - 1].0);
    assert!(
        last - first <= 4,
        "one caret, not several tall runs: {cols:?}"
    );
    // The caret's left edge: the coverage-weighted centre of its columns, at
    // a row inside it, less half its 1.5px width.
    // The caret's top edge, from its best-covered column: the first row of
    // its run, less the share of the row above it covers.
    let (x, start) = *cols
        .iter()
        .max_by(|a, b| {
            let (ra, rb) = (a.1 + tall / 2, b.1 + tall / 2);
            alpha(&px, w, a.0, ra).total_cmp(&alpha(&px, w, b.0, rb))
        })
        .unwrap();
    let top = start as f64
        - if start > 0 {
            alpha(&px, w, x, start - 1)
        } else {
            0.0
        };
    // Its left edge: the coverage-weighted centre of its columns, less half
    // its 1.5px width, on a row one logical px below its top — above every
    // glyph's ink (a capital starts about 3.4px below the line top in Inter
    // at 14px), so a glyph beside the caret adds nothing.
    let row = (top + scale).ceil() as usize;
    let (mut sum, mut moment) = (0.0, 0.0);
    for x in first.saturating_sub(1)..=(last + 1).min(w - 1) {
        let a = alpha(&px, w, x, row);
        sum += a;
        moment += a * (x as f64 + 0.5);
    }
    let left = moment / sum - 0.75 * scale;
    (left / scale, top / scale)
}

/// The bounding box of every inked pixel, in logical px `(x0, y0, x1, y1)`.
fn ink_box(app: &mut RinchApp, scale: f64) -> (f64, f64, f64, f64) {
    let (px, w) = frame(app, scale);
    let h = px.len() / 4 / w;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha(&px, w, x, y) > 0.25 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    assert!(x0 != usize::MAX, "positive control: the text is painted");
    let s = scale;
    (x0 as f64 / s, y0 as f64 / s, x1 as f64 / s, y1 as f64 / s)
}

/// MCP's caret at `byte` is the painted caret, at paint scales 1 and 2 (and
/// MCP asked at both).
fn assert_caret_is_painted(app: &mut RinchApp, id: u64, byte: usize, what: &str) {
    for scale in [1.0, 2.0] {
        let painted = painted_caret(app, id, byte, scale);
        let mcp = mcp_caret(app, id, byte, scale);
        assert!(
            (mcp.0 - painted.0).abs() <= 0.5 && (mcp.1 - painted.1).abs() <= 0.5,
            "{what}, byte {byte}, scale {scale}: MCP {mcp:?} vs painted {painted:?}"
        );
    }
}

/// A glyph box at `byte` runs from the caret before it to the caret after it,
/// on the caret's line.
fn assert_glyph_between_carets(app: &mut RinchApp, id: u64, byte: usize, next: usize, what: &str) {
    let g = mcp_glyph(app, id, byte, 1.0);
    let a = mcp_caret(app, id, byte, 1.0);
    let b = mcp_caret(app, id, next, 1.0);
    assert!(
        (g[0] - a.0).abs() <= 0.5 && (g[0] + g[2] - b.0).abs() <= 0.5 && (g[1] - a.1).abs() <= 0.5,
        "{what}: glyph {g:?} between carets {a:?} and {b:?}"
    );
    assert_eq!(
        g,
        mcp_glyph(app, id, byte, 2.0),
        "{what}: the same box at scale 2"
    );
}

const FIELD: &str = "border: 3px solid transparent; background: transparent; \
     color: rgb(0, 0, 255); font-family: sans-serif; font-size: 14px; line-height: 20px; margin: 0";

/// A 40px-tall `<input>`: paint centres its line vertically.
#[test]
fn an_input_caret_is_centred_as_painted() {
    let (mut app, id) = page(
        "input",
        None,
        format!(
            "position: absolute; left: 41px; top: 33px; width: 300px; height: 40px; \
                 padding: 5px 0 0 13px; {FIELD}"
        ),
        "a c e mo",
    );
    for byte in [0, 3, 8] {
        assert_caret_is_painted(&mut app, id, byte, "centred input");
    }
    assert_glyph_between_carets(&mut app, id, 2, 3, "centred input");
}

/// A password field paints bullets, not its value.
#[test]
fn a_password_caret_follows_the_bullets() {
    let (mut app, id) = page(
        "input",
        Some("password"),
        format!(
            "position: absolute; left: 41px; top: 33px; width: 300px; height: 30px; \
                 padding: 0 0 0 13px; {FIELD}"
        ),
        "WWWWWW",
    );
    for byte in [3, 6] {
        assert_caret_is_painted(&mut app, id, byte, "password");
    }
    assert_glyph_between_carets(&mut app, id, 2, 3, "password");
}

/// A textarea with a wide left padding wraps at its text column, so its
/// last caret is on the second line, where paint draws it.
#[test]
fn a_textarea_caret_wraps_at_the_painted_column() {
    let text = "wax was a vow, a raw saw: a war";
    let (mut app, id) = page(
        "textarea",
        None,
        format!(
            "position: absolute; left: 41px; top: 100px; width: 200px; height: 120px; \
                 padding: 7px 0 0 40px; {FIELD}"
        ),
        text,
    );
    // Positive control: the text fits the border box but not the column, so
    // paint draws its end on the second line.
    let (_, top) = painted_caret(&mut app, id, text.len(), 1.0);
    assert!(top > 100.0 + 7.0 + 10.0, "the end is on line 2, top {top}");
    for byte in [4, text.len()] {
        assert_caret_is_painted(&mut app, id, byte, "textarea");
    }
    assert_glyph_between_carets(&mut app, id, text.len() - 2, text.len() - 1, "textarea");
}

/// A padded `<p>`: its IFC text is painted at the content box, one padding
/// and border in, and so is MCP's answer (caret and glyph). One glyph, so the
/// ink box is that glyph's.
#[test]
fn a_padded_paragraph_answers_its_painted_glyph() {
    let (mut app, id) = page(
        "p",
        None,
        "position: absolute; left: 41px; top: 300px; width: 300px; margin: 0; \
         padding: 9px 0 0 17px; border-left: 4px solid transparent; \
         color: rgb(0, 0, 255); font-size: 14px; line-height: 20px",
        "H",
    );
    for scale in [1.0, 2.0] {
        let ink = ink_box(&mut app, scale);
        let g = mcp_glyph(&mut app, id, 0, scale);
        let c0 = mcp_caret(&mut app, id, 0, scale);
        let c1 = mcp_caret(&mut app, id, 1, scale);
        // The glyph's ink lies inside its box, and the box starts within a
        // side bearing of it.
        assert!(
            ink.0 >= g[0] - 0.5
                && ink.2 <= g[0] + g[2] + 0.5
                && ink.1 >= g[1] - 0.5
                && ink.3 <= g[1] + g[3] + 0.5
                && ink.0 - g[0] < 2.5,
            "scale {scale}: ink {ink:?} vs MCP glyph {g:?}"
        );
        assert!(
            (c0.0 - g[0]).abs() <= 0.5
                && (c0.1 - g[1]).abs() <= 0.5
                && (c1.0 - (g[0] + g[2])).abs() <= 0.5,
            "scale {scale}: carets {c0:?} {c1:?} vs glyph {g:?}"
        );
    }
}

/// Positive control for the paragraph oracle: with no padding the border
/// box and the content box coincide.
#[test]
fn an_unpadded_paragraph_answers_its_painted_glyph() {
    let (mut app, id) = page(
        "p",
        None,
        "position: absolute; left: 41px; top: 300px; width: 300px; margin: 0; \
         color: rgb(0, 0, 255); font-size: 14px; line-height: 20px",
        "H",
    );
    let ink = ink_box(&mut app, 1.0);
    let g = mcp_glyph(&mut app, id, 0, 1.0);
    assert!(
        ink.0 >= g[0] - 0.5 && ink.0 - g[0] < 2.5 && ink.1 >= g[1] - 0.5,
        "ink {ink:?} vs {g:?}"
    );
}

// ---- From the review of PR #1745 ----

fn page_html(
    build: impl Fn(&mut RenderScope) -> (NodeHandle, NodeHandle) + 'static,
) -> (RinchApp, u64) {
    let id: Rc<Cell<u64>> = Rc::default();
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (root, target) = build(scope);
        id_in.set(target.node_id().0);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, id.get())
}

fn glyph_result(app: &mut RinchApp, node_id: u64, byte_offset: usize) -> DebugResult {
    let mut actions = Vec::new();
    app.execute_debug_command(
        DebugCommandKind::GetGlyphBounds {
            node_id,
            byte_offset,
        },
        &mut actions,
        1.0,
        VP,
    )
}

fn root_div(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute(
        "style",
        "position: relative; width: 500px; height: 400px; margin: 0; \
         font-family: sans-serif; font-size: 14px; line-height: 20px",
    );
    root
}

/// A padded `<p>` inside a scroller scrolled down by 37px.
#[test]
fn r1745_scrolled_scroller() {
    let (mut app, id) = page_html(|scope| {
        let root = root_div(scope);
        let sc = scope.create_element("div");
        sc.set_attribute(
            "style",
            "position: absolute; left: 30px; top: 50px; width: 300px; height: 200px; overflow: auto; --rinch-scrollbar-width: none",
        );
        let spacer = scope.create_element("div");
        spacer.set_attribute("style", "height: 100px");
        let p = scope.create_element("p");
        p.set_attribute(
            "style",
            "margin: 0; padding: 9px 0 0 17px; color: rgb(0,0,255); font-size: 14px; line-height: 20px",
        );
        p.append_child(&scope.create_text("H"));
        let tail = scope.create_element("div");
        tail.set_attribute("style", "height: 600px");
        sc.append_child(&spacer);
        sc.append_child(&p);
        sc.append_child(&tail);
        root.append_child(&sc);
        (root, p)
    });
    {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let sc = d.tree.nodes[id].parent.unwrap();
        d.tree.nodes[sc].scroll_offset.1 = 37.0;
        d.tree.mark_scrolled(sc);
    }
    for scale in [1.0, 1.5] {
        let ink = ink_box(&mut app, scale);
        let g = mcp_glyph(&mut app, id, 0, scale);
        eprintln!("scroll scale {scale}: ink {ink:?} glyph {g:?}");
        assert!(
            ink.0 >= g[0] - 0.7
                && ink.0 - g[0] < 2.5
                && ink.1 >= g[1] - 0.7
                && ink.3 <= g[1] + g[3] + 0.7,
            "scrolled, scale {scale}: ink {ink:?} vs glyph {g:?}"
        );
    }
}

/// A rotated ancestor: the reported box should contain the ink.
#[test]
fn r1745_rotated_ancestor() {
    let (mut app, id) = page_html(|scope| {
        let root = root_div(scope);
        let w = scope.create_element("div");
        w.set_attribute(
            "style",
            "position: absolute; left: 150px; top: 150px; width: 100px; height: 40px; transform: rotate(45deg)",
        );
        let p = scope.create_element("p");
        p.set_attribute(
            "style",
            "margin: 0; color: rgb(0,0,255); font-size: 28px; line-height: 40px",
        );
        p.append_child(&scope.create_text("H"));
        w.append_child(&p);
        root.append_child(&w);
        (root, p)
    });
    let ink = ink_box(&mut app, 1.0);
    let g = mcp_glyph(&mut app, id, 0, 1.0);
    eprintln!("rotate: ink {ink:?} glyph {g:?}");
    assert!(
        g[2] > 5.0 && g[3] > 5.0,
        "rotated glyph box degenerate: {g:?} ink {ink:?}"
    );
    assert!(
        ink.0 >= g[0] - 1.0
            && ink.2 <= g[0] + g[2] + 1.0
            && ink.1 >= g[1] - 1.0
            && ink.3 <= g[1] + g[3] + 1.0,
        "rotated: ink {ink:?} outside glyph AABB {g:?}"
    );
}

/// A scaled ancestor: the box is the painted size.
#[test]
fn r1745_scaled_ancestor() {
    let (mut app, id) = page_html(|scope| {
        let root = root_div(scope);
        let w = scope.create_element("div");
        w.set_attribute(
            "style",
            "position: absolute; left: 60px; top: 70px; width: 100px; height: 40px; transform: scale(2); transform-origin: 0 0",
        );
        let p = scope.create_element("p");
        p.set_attribute(
            "style",
            "margin: 0; padding-left: 5px; color: rgb(0,0,255); font-size: 14px; line-height: 20px",
        );
        p.append_child(&scope.create_text("H"));
        w.append_child(&p);
        root.append_child(&w);
        (root, p)
    });
    let ink = ink_box(&mut app, 1.0);
    let g = mcp_glyph(&mut app, id, 0, 1.0);
    eprintln!("scale: ink {ink:?} glyph {g:?}");
    assert!(
        ink.0 >= g[0] - 0.7
            && ink.2 <= g[0] + g[2] + 0.7
            && ink.0 - g[0] < 5.0
            && ink.1 >= g[1] - 0.7
            && ink.3 <= g[1] + g[3] + 0.7,
        "scaled: ink {ink:?} vs glyph {g:?}"
    );
}

/// Text inside an inline-block with padding, inside a padded p.
#[test]
fn r1745_inline_block() {
    let (mut app, id) = page_html(|scope| {
        let root = root_div(scope);
        let p = scope.create_element("p");
        p.set_attribute("style", "position: absolute; left: 30px; top: 40px; margin: 0; padding: 11px 0 0 23px; color: transparent; font-size: 14px; line-height: 20px");
        p.append_child(&scope.create_text("xx "));
        let ib = scope.create_element("span");
        ib.set_attribute(
            "style",
            "display: inline-block; padding: 6px 0 0 9px; color: rgb(0,0,255)",
        );
        ib.append_child(&scope.create_text("H"));
        p.append_child(&ib);
        root.append_child(&p);
        (root, ib)
    });
    let ink = ink_box(&mut app, 1.0);
    let g = mcp_glyph(&mut app, id, 0, 1.0);
    eprintln!("inline-block: ink {ink:?} glyph {g:?}");
    assert!(
        ink.0 >= g[0] - 0.7
            && ink.0 - g[0] < 2.5
            && ink.1 >= g[1] - 0.7
            && ink.3 <= g[1] + g[3] + 0.7,
        "inline-block: ink {ink:?} vs glyph {g:?}"
    );
}

/// A password field with a multi-byte value, offset inside a char.
#[test]
fn r1745_password_mid_char() {
    let (mut app, id) = page(
        "input",
        Some("password"),
        format!("position: absolute; left: 41px; top: 33px; width: 300px; height: 30px; {FIELD}"),
        "\u{e9}\u{e9}",
    );
    // Byte 1 is inside the first `é`: answered at the char before it, as
    // `painted_caret_rect` snaps it, never a panic on the app's thread.
    assert!(
        matches!(glyph_result(&mut app, id, 1), DebugResult::Json { .. }),
        "an offset inside a char answers"
    );
    assert_eq!(
        mcp_glyph(&mut app, id, 1, 1.0),
        mcp_glyph(&mut app, id, 0, 1.0)
    );
}

/// A focused single-line input scrolled horizontally (#1723): the glyph box
/// at the last character sits between the painted carets.
#[test]
fn r1745_scrolled_input_glyph() {
    let v = "wax was a vow a raw saw a war wax";
    let (mut app, id) = page(
        "input",
        None,
        format!(
            "position: absolute; left: 41px; top: 33px; width: 90px; height: 30px; padding: 0 0 0 13px; {FIELD}"
        ),
        v,
    );
    let n = v.len();
    let painted_end = painted_caret(&mut app, id, n, 1.0);
    let mcp_end = mcp_caret(&mut app, id, n, 1.0);
    let g = mcp_glyph(&mut app, id, n - 1, 1.0);
    eprintln!("scrolled input: painted end {painted_end:?} mcp end {mcp_end:?} glyph {g:?}");
    assert!(
        (mcp_end.0 - painted_end.0).abs() <= 0.5,
        "caret {mcp_end:?} vs painted {painted_end:?}"
    );
    assert!(
        painted_end.0 < 41.0 + 90.0,
        "positive control: the field is scrolled, end caret in view"
    );
    assert!(
        (g[0] + g[2] - painted_end.0).abs() <= 0.7,
        "glyph {g:?} ends at painted caret {painted_end:?}"
    );
}
