//! Review of #1687 (#1684): probes.
use super::*;
use std::cell::Cell;

const FAM: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

fn page(tag: &'static str, value: String, width: u32) -> (RinchApp, u64) {
    let id = Rc::new(Cell::new(0u64));
    let id_in = id.clone();
    let style = format!(
        "display: block; width: {width}px; font: 32px/40px sans-serif; padding: 0; border: 0"
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", &style);
        field.set_attribute("value", &value);
        root.append_child(&field);
        id_in.set(field.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, id.get())
}

fn sweep_line(app: &mut RinchApp, field: u64, y_off: f32) -> Vec<(f32, usize)> {
    let (bx, by, bw, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, field)
    };
    let doc = app.doc.clone().unwrap();
    let d = doc.borrow();
    let mut out = Vec::new();
    let mut x = bx;
    while x < bx + bw {
        out.push((
            x - bx,
            RinchApp::compute_input_cursor_from_click(
                &d.tree,
                &mut app.hit_test_font_cx,
                &mut app.hit_test_layout_cx,
                field,
                x,
                by + y_off,
            ),
        ));
        x += 0.5;
    }
    out
}

/// A run of emoji with no spaces wraps between clusters. A press on the right
/// half of the LAST emoji of the first line must answer that cluster's END
/// (the wrap point), as a press on the right half of any other emoji does.
#[test]
fn review_1687_wrap_right_half_of_last_emoji() {
    let value: String = std::iter::repeat_n(FAM, 8).collect();
    // Find the emoji's width with a wide field first.
    let (mut wide, wf) = page("textarea", value.clone(), 2000);
    let s = sweep_line(&mut wide, wf, 20.0);
    let ends: Vec<f32> = s
        .iter()
        .filter(|(_, o)| *o == FAM.len())
        .map(|(x, _)| *x)
        .collect();
    let starts_of_second: Vec<f32> = s.iter().filter(|(_, o)| *o == 0).map(|(x, _)| *x).collect();
    eprintln!(
        "first emoji: start answered up to x={:?}, end from x={:?}",
        starts_of_second.last(),
        ends.first()
    );
    // An emoji is about 2 * (the x where its end starts being answered).
    let half = *ends.first().expect("end answered");
    let w = (half * 2.0).round() as u32;
    // A field three emoji and a bit wide: three per line.
    let (mut app, f) = page("textarea", value.clone(), 3 * w + w / 3);
    let line1 = sweep_line(&mut app, f, 20.0);
    let s3 = 2 * FAM.len();
    let e3 = 3 * FAM.len();
    let starts = line1.iter().filter(|(_, o)| *o == s3).count();
    let ends = line1.iter().filter(|(_, o)| *o == e3).count();
    let mut runs: Vec<(usize, f32, f32)> = Vec::new();
    for &(x, o) in &line1 {
        match runs.last_mut() {
            Some((lo, _, hi)) if *lo == o => *hi = x,
            _ => runs.push((o, x, x)),
        }
    }
    eprintln!("emoji width ~{w}; line 1 runs (offset, x0, x1): {runs:?}");
    assert!(
        ends * 3 >= starts,
        "the third (last) emoji of line 1: start answered {starts} times, end {ends}"
    );
}

const VALUE: &str = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b\u{2764}\u{FE0F}c\u{1F1EB}\u{1F1F7}d1\u{FE0F}\u{20E3}e\u{301}f";

/// Read-only selection (`user-select: text`): every offset a press resolves to
/// is a cluster edge (`compute_ifc_offset_from_click`, text_selection.rs).
#[test]
fn review_1687_read_only_selection_lands_on_a_cluster_edge() {
    let id = Rc::new(Cell::new(0u64));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let p = scope.create_element("div");
        p.set_attribute(
            "style",
            "user-select: text; width: 700px; font: 32px/40px sans-serif",
        );
        let t = scope.create_text(VALUE);
        p.append_child(&t);
        root.append_child(&p);
        id_in.set(p.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let d = app.doc.as_ref().unwrap().borrow();
    let (bx, by, bw, _) = painted_element_box(&d.tree, id.get());
    let mut got = Vec::new();
    let mut x = bx;
    while x < bx + bw {
        got.push(RinchApp::compute_ifc_offset_from_click(
            &d.tree,
            id.get(),
            x,
            by + 20.0,
        ));
        x += 0.5;
    }
    let inside: Vec<usize> = got
        .iter()
        .copied()
        .filter(|&o| !rinch_words::is_grapheme_boundary(VALUE, o))
        .collect();
    assert!(inside.is_empty(), "inside a cluster at {inside:?}");
    for b in (0..=VALUE.len()).filter(|&b| rinch_words::is_grapheme_boundary(VALUE, b)) {
        assert!(got.contains(&b), "never reached {b}");
    }
}

/// ArrowDown in a textarea from every caret of a line of narrow letters onto a
/// line of emoji: the caret lands on a cluster edge (vertical_move_target).
#[test]
fn review_1687_textarea_arrow_down_lands_on_a_cluster_edge() {
    let line1: String = std::iter::repeat_n('i', 40).collect();
    let fams: String = std::iter::repeat_n(FAM, 3).collect();
    let value = format!("{line1}\n{fams}");
    let input_id = rinch_core::events::register_input_handler(
        rinch_core::events::InputCallback::new(|_v: String| {}),
    );
    let id = Rc::new(Cell::new(0u64));
    let id_in = id.clone();
    let v2 = value.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let f = scope.create_element("textarea");
        f.set_attribute("style", "display: block; width: 700px; height: 200px; font: 32px/40px sans-serif; padding: 0; border: 0");
        f.set_attribute("value", &v2);
        f.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&f);
        id_in.set(f.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (bx, by) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (x, y, _, _) = painted_element_box(&d.tree, id.get());
        (x, y)
    };
    let mut landed = Vec::new();
    for col in 0..=40usize {
        // click to put the caret at column `col` of line 1
        let x = {
            let doc = app.doc.clone().unwrap();
            let d = doc.borrow();
            let (cx, _, _, _) = RinchApp::input_caret_rect_for_offset(
                &d.tree,
                &mut app.hit_test_font_cx,
                &mut app.hit_test_layout_cx,
                id.get(),
                col,
            )
            .unwrap();
            cx
        };
        for ev in [
            PlatformEvent::MouseDown {
                x,
                y: by + 20.0,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x,
                y: by + 20.0,
                button: MouseButton::Left,
            },
        ] {
            app.handle_event(ev, (800, 600), 1.0);
        }
        app.resolve_and_repaint(800.0, 600.0);
        app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::ArrowDown,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            (800, 600),
            1.0,
        );
        app.resolve_and_repaint(800.0, 600.0);
        let d = app.doc.as_ref().unwrap().borrow();
        let c: usize = d
            .tree
            .get(id.get())
            .unwrap()
            .attributes
            .get("data-cursor-pos")
            .unwrap()
            .parse()
            .unwrap();
        landed.push(c);
    }
    let _ = bx;
    let inside: Vec<usize> = landed
        .iter()
        .copied()
        .filter(|&o| !rinch_words::is_grapheme_boundary(&value, o))
        .collect();
    assert!(
        inside.is_empty(),
        "ArrowDown landed inside a cluster at {inside:?} (all {landed:?})"
    );
    // positive control: landed somewhere on line 2 other than its start
    assert!(landed.iter().any(|&o| o > line1.len() + 1), "{landed:?}");
}

/// The editor press at a soft wrap: same as the textarea case.
#[cfg(feature = "desktop")]
#[test]
fn review_1687_editor_wrap_right_half_of_last_emoji() {
    let fams: String = std::iter::repeat_n(FAM, 8).collect();
    let slot: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    let slot_in = slot.clone();
    let f2 = fams.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(&format!("<p>{f2}</p>"));
        container.set_attribute(
            "style",
            "display: block; width: 133px; font: 32px/40px sans-serif; padding: 0; border: 0",
        );
        root.append_child(&container);
        slot_in.set(container.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (bx, by, bw, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, slot.get())
    };
    let mut runs: Vec<(usize, f32, f32)> = Vec::new();
    let mut x = bx + 1.0;
    while x < bx + bw {
        if let Some(p) = app.editor_point(x, by + 20.0) {
            match runs.last_mut() {
                Some((o, _, hi)) if *o == p.byte => *hi = x - bx,
                _ => runs.push((p.byte, x - bx, x - bx)),
            }
        }
        x += 0.5;
    }
    eprintln!("editor line 1 runs: {runs:?}");
    let s3 = 2 * FAM.len();
    let e3 = 3 * FAM.len();
    let w = |o: usize| {
        runs.iter()
            .filter(|r| r.0 == o)
            .map(|r| r.2 - r.1)
            .sum::<f32>()
    };
    assert!(3.0 * w(e3) >= w(s3), "start {} px, end {} px", w(s3), w(e3));
}

/// Read-only selection in a flex container's own text (a text leaf, no IFC):
/// does a drag select anything? Control: the same in a block.
#[test]
fn review_1687_flex_text_leaf_read_only_selection() {
    for display in ["block", "flex"] {
        let id = Rc::new(Cell::new(0u64));
        let id_in = id.clone();
        let style = format!(
            "display: {display}; user-select: text; width: 400px; font: 20px/24px sans-serif"
        );
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let p = scope.create_element("div");
            p.set_attribute("style", &style);
            p.append_child(&scope.create_text("hello world hello world"));
            root.append_child(&p);
            id_in.set(p.node_id().0);
            root
        });
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let (bx, by, _, bh) = {
            let d = app.doc.as_ref().unwrap().borrow();
            painted_element_box(&d.tree, id.get())
        };
        let y = by + bh / 2.0;
        app.handle_event(
            PlatformEvent::MouseDown {
                x: bx + 2.0,
                y,
                button: MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseMove { x: bx + 120.0, y },
            (800, 600),
            1.0,
        );
        app.handle_event(
            PlatformEvent::MouseUp {
                x: bx + 120.0,
                y,
                button: MouseButton::Left,
            },
            (800, 600),
            1.0,
        );
        app.resolve_and_repaint(800.0, 600.0);
        let d = app.doc.as_ref().unwrap().borrow();
        let any = d
            .tree
            .nodes
            .iter()
            .any(|(_, n)| n.attributes.contains_key("data-text-sel-start"));
        eprintln!("display {display}: selection attribute written: {any}");
    }
}
