//! What a copy holds after real gestures (a press and drag, a triple click,
//! Shift+click, Select All) around inline-blocks and floats, against
//! Chrome 156 driven by Playwright's mouse (review of #2249). Each end of a
//! selection says which side of the boxes at its offset it stands on: a
//! point past an in-flow box is after it, a float is on no line, a triple
//! click takes the in-flow boxes at its paragraph's edges and Select All
//! everything.

use super::text_selection_across_tests::{drag, mount, node, press, release};
use super::text_selection_chrome_tests::SEL;
use super::*;

const VP: (u32, u32) = (800, 600);

/// The point `fx` of the way across `#id`'s painted box, 10px down. For a
/// flowed inline span, the caret `fx` of the way through its first text (as
/// a point `fx` across its glyphs is in Chrome), a little inside the glyph
/// beside it. Chosen by display, not by an empty painted box: since #2257 a
/// span's painted box is its fragments' union, and its edge `fx = 1.0` on a
/// right-to-left line is the line's right end, which is outside the root
/// (601 in a 600px root while #1869's break slack shifts the line a pixel
/// right; at a box's right edge even without it).
fn pt(app: &RinchApp, id: &str, fx: f32) -> (f32, f32) {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    if d.tree.get(n).unwrap().display_mode != rinch_dom::DisplayMode::Inline {
        let (x, y, w, _) = super::hit_testing::painted_element_box(&d.tree, n);
        return (x + w * fx, y + 10.0);
    }
    let text = d.tree.get(n).unwrap().children[0];
    let mut id = text;
    let root = loop {
        let at = d.tree.get(id).unwrap();
        if let Some(b) = at.run_box {
            break b;
        }
        id = at.parent.unwrap();
        if d.tree.get(id).unwrap().text_layout.is_some() {
            break id;
        }
    };
    let l = d.tree.get(root).unwrap().text_layout.as_ref().unwrap();
    let r = l.text_ranges.iter().find(|r| r.node_id == text).unwrap();
    let chars: Vec<usize> = l.text_content[r.flat_start..r.flat_end]
        .char_indices()
        .map(|(i, _)| r.flat_start + i)
        .chain([r.flat_end])
        .collect();
    // A point `fx` across a right-to-left root is `1 - fx` into its text.
    let fx = if d.tree.get(root).unwrap().computed_style.direction_rtl {
        1.0 - fx
    } else {
        fx
    };
    let i = ((chars.len() - 1) as f32 * fx).round() as usize;
    let (cx, cy, ch) = d.text_caret_window_rect(root, chars[i]).unwrap();
    // A quarter of the way toward the next caret (the previous at the
    // end): inside the glyph beside it, nearest this caret.
    let j = if i + 1 < chars.len() {
        i + 1
    } else {
        i.saturating_sub(1)
    };
    let (nx, _, _) = d.text_caret_window_rect(root, chars[j]).unwrap();
    (cx + (nx - cx) * 0.25, cy + ch / 2.0)
}

fn clicks(app: &mut RinchApp, p: (f32, f32), times: usize) {
    for _ in 0..times {
        press(app, p);
        release(app, p);
    }
}

fn modifiers(app: &mut RinchApp, m: Modifiers) {
    app.handle_event(PlatformEvent::ModifiersChanged(m), VP, 1.0);
}

fn drag_from_to(app: &mut RinchApp, a: (f32, f32), b: (f32, f32)) {
    press(app, a);
    drag(app, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
    drag(app, b);
    release(app, b);
}

/// Run `ops` (the Chrome script's: `click:id:fx:n`, `clickpx:id:px:n`,
/// `shift:id:fx`, `drag:id:fx:id:fx`, `dragpx:id:px:id:fx`, `all`) on
/// `html` and copy.
pub(super) fn copy_after(html: &'static str, ops: &str) -> String {
    app_after(html, ops)
        .text_selection_text()
        .unwrap_or_default()
}

/// The app holding `html` after `ops` ([`copy_after`]).
pub(super) fn app_after(html: &'static str, ops: &str) -> RinchApp {
    let mut app = mount(html, SEL);
    for op in ops.split(';') {
        let p: Vec<&str> = op.split(':').collect();
        let f = |i: usize| p[i].parse::<f32>().unwrap();
        match p[0] {
            "click" => {
                let at = pt(&app, p[1], f(2));
                clicks(&mut app, at, p[3].parse().unwrap());
            }
            "clickpx" => {
                let (x, y) = pt(&app, p[1], 0.0);
                clicks(&mut app, (x + f(2), y), p[3].parse().unwrap());
            }
            "shift" => {
                let at = pt(&app, p[1], f(2));
                let shift = Modifiers {
                    shift: true,
                    ..Modifiers::default()
                };
                modifiers(&mut app, shift);
                clicks(&mut app, at, 1);
                modifiers(&mut app, Modifiers::default());
            }
            "drag" => {
                let (a, b) = (pt(&app, p[1], f(2)), pt(&app, p[3], f(4)));
                drag_from_to(&mut app, a, b);
            }
            "dragpx" => {
                let (x, y) = pt(&app, p[1], 0.0);
                let b = pt(&app, p[3], f(4));
                drag_from_to(&mut app, (x + f(2), y), b);
            }
            "all" => {
                let ctrl = Modifiers {
                    ctrl: true,
                    ..Modifiers::default()
                };
                modifiers(&mut app, ctrl);
                app.handle_event(
                    PlatformEvent::KeyDown {
                        key: KeyCode::KeyA,
                        logical_key: None,
                        text: None,
                        modifiers: ctrl,
                        repeat: KeyRepeat::Unknown,
                    },
                    VP,
                    1.0,
                );
                modifiers(&mut app, Modifiers::default());
            }
            "key" => {
                // `key:Control+Shift+ArrowRight:n`, Playwright's spelling.
                let mut m = Modifiers::default();
                let mut code = None;
                for part in p[1].split('+') {
                    match part {
                        "Shift" => m.shift = true,
                        "Control" => m.ctrl = true,
                        "Alt" => m.alt = true,
                        "Meta" => m.meta = true,
                        "ArrowLeft" => code = Some(KeyCode::ArrowLeft),
                        "ArrowRight" => code = Some(KeyCode::ArrowRight),
                        "ArrowUp" => code = Some(KeyCode::ArrowUp),
                        "ArrowDown" => code = Some(KeyCode::ArrowDown),
                        "Home" => code = Some(KeyCode::Home),
                        "End" => code = Some(KeyCode::End),
                        other => panic!("no key {other}"),
                    }
                }
                let times = p.get(2).map_or(1, |n| n.parse().unwrap());
                modifiers(&mut app, m);
                for _ in 0..times {
                    app.handle_event(
                        PlatformEvent::KeyDown {
                            key: code.expect("a key"),
                            logical_key: None,
                            text: None,
                            modifiers: m,
                            repeat: KeyRepeat::Unknown,
                        },
                        VP,
                        1.0,
                    );
                }
                modifiers(&mut app, Modifiers::default());
            }
            other => panic!("no op {other}"),
        }
    }
    app
}

const GESTURES: &[(&str, &str, &str, &str)] = &[
    (
        "triple trailing float",
        "<p id=\"p\" style=\"margin:0\"><span id=\"t\">text</span> <span style=\"float:right\">fr</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "text",
    ),
    (
        "triple leading float",
        "<p style=\"margin:0\"><span style=\"float:left\">fl</span><span id=\"t\">text</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "text\n\n",
    ),
    (
        "triple trailing ib",
        "<p style=\"margin:0\"><span id=\"t\">text</span> <span style=\"display:inline-block\">mid</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "text mid\n\n",
    ),
    (
        "triple leading ib",
        "<p style=\"margin:0\"><span style=\"display:inline-block\">mid</span> <span id=\"t\">text</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "mid text\n\n",
    ),
    (
        "triple inside ib",
        "<p style=\"margin:0\">lead <span id=\"t\" style=\"display:inline-block\">mid</span> tail</p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "lead mid tail\n\n",
    ),
    (
        "triple leading and trailing ib",
        "<p style=\"margin:0\"><span style=\"display:inline-block\">m1</span> <span id=\"t\">text</span> <span style=\"display:inline-block\">m2</span></p><p style=\"margin:0\">next</p>",
        "click:t:0.5:3",
        "m1 text m2\n\n",
    ),
    (
        "selectall last para ib",
        "<p id=\"t\" style=\"margin:0\">first</p><p style=\"margin:0\">text <span style=\"display:inline-block\">mid</span> tail</p>",
        "click:t:0.5:1;all",
        "first\n\ntext mid tail",
    ),
    (
        "selectall last para ends with ib",
        "<p id=\"t\" style=\"margin:0\">first</p><p style=\"margin:0\">text <span style=\"display:inline-block\">mid</span></p>",
        "click:t:0.5:1;all",
        "first\n\ntext mid",
    ),
    (
        "selectall floats",
        "<p style=\"margin:0\"><span style=\"float:left\">fl</span><span id=\"t\">first</span></p><p style=\"margin:0\">text<span style=\"float:right\">fr</span></p>",
        "click:t:0.5:1;all",
        "flfirst\n\ntextfr",
    ),
    (
        "selectall first para starts with ib",
        "<p style=\"margin:0\"><span style=\"display:inline-block\">mid</span> <span id=\"t\">first</span></p><p style=\"margin:0\">last</p>",
        "click:t:0.5:1;all",
        "mid first\n\nlast",
    ),
    (
        "drag from padding before leading ib",
        "<div id=\"d\" style=\"padding-left:40px\"><span style=\"display:inline-block\">mid</span><span>after</span></div><div id=\"e\">second</div>",
        "drag:d:0.02:e:0.9",
        "midafter\nsecond",
    ),
    (
        "drag from padding before leading float",
        "<div id=\"d\" style=\"padding-left:40px\"><span style=\"float:left\">fl</span><span>after</span></div><div id=\"e\">second</div>",
        "drag:d:0.02:e:0.9",
        "after\nsecond",
    ),
    (
        "drag past trailing ib",
        "<div id=\"a\">first</div><div id=\"d\"><span>before</span> <span style=\"display:inline-block\">mid</span></div>",
        "drag:a:0.0:d:0.9",
        "first\nbefore mid",
    ),
    (
        "drag past trailing float",
        "<div id=\"a\">first</div><div id=\"d\"><span>before</span><span style=\"float:right\">fr</span></div>",
        "drag:a:0.0:d:0.5",
        "first\nbefore",
    ),
    (
        "drag from left of after-float text",
        "<div id=\"d\"><span style=\"float:left\">fl</span><span id=\"t\">after</span></div><div id=\"e\">second</div>",
        "drag:t:0.0:e:0.9",
        "after\nsecond",
    ),
    (
        "triple then shift-click leading float",
        "<p style=\"margin:0\"><span style=\"float:left\">fl</span><span id=\"t\">text</span></p><p id=\"e\" style=\"margin:0\">next</p>",
        "click:t:0.5:3;shift:e:0.9",
        "text\n\nnext",
    ),
    (
        "triple then shift-click trailing ib",
        "<p id=\"e0\" style=\"margin:0\"><span id=\"s\">zero</span></p><p style=\"margin:0\"><span id=\"t\">text</span> <span style=\"display:inline-block\">mid</span></p>",
        "click:t:0.5:3;shift:s:0.0",
        "zero\n\ntext mid",
    ),
    (
        "drag a in ib to text after rtl",
        "<div dir=\"rtl\">שלום <span id=\"t\" style=\"display:inline-block\">mid</span> <span id=\"u\">עולם</span> סוף</div>",
        "drag:t:0.5:u:0.5",
        "id עו",
    ),
    (
        "px triple leading float",
        "<p id=\"p\" style=\"margin:0\"><span style=\"float:left\">fl</span><span>text</span></p><p style=\"margin:0\">next</p>",
        "clickpx:p:25:3",
        "text\n\n",
    ),
    (
        "px triple leading ib",
        "<p id=\"p\" style=\"margin:0\"><span style=\"display:inline-block\">mid</span> <span>text</span></p><p style=\"margin:0\">next</p>",
        "clickpx:p:48:3",
        "mid text\n\n",
    ),
    (
        "px triple leading and trailing ib",
        "<p id=\"p\" style=\"margin:0\"><span style=\"display:inline-block\">m1</span> <span>text</span> <span style=\"display:inline-block\">m2</span></p><p style=\"margin:0\">next</p>",
        "clickpx:p:40:3",
        "m1 text m2\n\n",
    ),
    (
        "px drag from start of after-float text",
        "<div id=\"p\"><span style=\"float:left\">fl</span><span>after</span></div><div id=\"e\">second</div>",
        "dragpx:p:13:e:0.9",
        "fter\nsecond",
    ),
    (
        "px drag from start of after-ib text",
        "<div id=\"p\"><span style=\"display:inline-block\">mid</span><span>after</span></div><div id=\"e\">second</div>",
        "dragpx:p:30:e:0.9",
        "after\nsecond",
    ),
    (
        "px triple then shift-click leading float",
        "<p id=\"p\" style=\"margin:0\"><span style=\"float:left\">fl</span><span>text</span></p><p id=\"e\" style=\"margin:0\">next</p>",
        "clickpx:p:25:3;shift:e:0.9",
        "text\n\nnext",
    ),
    (
        "px triple then shift-click leading ib",
        "<p id=\"p\" style=\"margin:0\"><span style=\"display:inline-block\">mid</span> <span>text</span></p><p id=\"e\" style=\"margin:0\">next</p>",
        "clickpx:p:48:3;shift:e:0.9",
        "mid text\n\nnext",
    ),
];

/// The cases of [`GESTURES`] named `names`, checked together. A triple
/// click's copy ends with the paragraph's line breaks, as Chrome's does
/// (#2256).
fn gestures_as_chrome(names: &[&str]) {
    let off: Vec<String> = names
        .iter()
        .map(|&n| {
            *GESTURES
                .iter()
                .find(|c| c.0 == n)
                .unwrap_or_else(|| panic!("no case {n}"))
        })
        .filter_map(|(name, html, ops, chrome)| {
            let t = copy_after(html, ops);
            (t != chrome).then(|| format!("{name}: {t:?}, Chrome {chrome:?}"))
        })
        .collect();
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn each_end_takes_the_boxes_on_its_side() {
    // Review of #2249, round 2: a point past an inline-block hit-tests to
    // after it, so a drag past a line's last inline-block takes it and one
    // from the padding before a leading inline-block takes that; a float
    // is on no line, so a press, drag or triple click never takes one at
    // its end; a triple click takes its paragraph's edge inline-blocks; and
    // an anchor keeps its side when Shift+click moves the focus. Points are
    // fixed offsets into block boxes: an inline span's painted box is empty
    // (#2257).
    gestures_as_chrome(&[
        "triple trailing float",
        "triple trailing ib",
        "drag from padding before leading ib",
        "drag from padding before leading float",
        "drag past trailing ib",
        "drag past trailing float",
        "px triple leading float",
        "px triple leading ib",
        "px triple leading and trailing ib",
        "px drag from start of after-ib text",
        "px triple then shift-click leading float",
        "px triple then shift-click leading ib",
        "drag from left of after-float text",
        "px drag from start of after-float text",
        "drag a in ib to text after rtl",
    ]);
}

#[test]
fn select_all_ends_where_its_last_paragraph_ends() {
    // Review of #2249, round 2: Select All ended at the last selectable
    // root, an inline-block inside the last paragraph, so the paragraph's
    // text after it was lost; and its ends take every box, floats too.
    gestures_as_chrome(&[
        "selectall last para ib",
        "selectall last para ends with ib",
        "selectall floats",
        "selectall first para starts with ib",
    ]);
}

#[test]
fn a_triple_click_and_a_shift_click_after_it_copy_as_chrome() {
    // #2256: these were left out above while a triple click stopped at its
    // paragraph's end and a Shift+click after one extended by characters.
    gestures_as_chrome(&[
        "triple leading float",
        "triple leading ib",
        "triple inside ib",
        "triple leading and trailing ib",
        "triple then shift-click leading float",
        "triple then shift-click trailing ib",
    ]);
}
