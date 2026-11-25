//! The rect of the caret paint draws: affinity, hidden fields, the painted
//! caret and password bullets.

use super::*;

/// The caret rect at the head follows the caret's affinity: End on a soft-
/// wrapped first line puts the caret at that line's end (upstream), which
/// the downstream rect at the same offset draws at the next line's start.
#[test]
fn the_head_caret_rect_follows_its_affinity() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda");
    // To the first line, then its end.
    for key in [
        KeyCode::Home,
        KeyCode::ArrowUp,
        KeyCode::ArrowUp,
        KeyCode::End,
    ] {
        f.press(key, None, false);
    }
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (_, head, _) = ta.selection_range().unwrap();
    let up = ta.caret_rect().unwrap();
    let down = ta.caret_rect_at(head).unwrap();
    let c0 = ta.caret_rect_at(0).unwrap();
    assert!((up.y - c0.y).abs() < 0.5, "upstream: line 1 {up:?} {c0:?}");
    assert!(down.y > up.y + 10.0, "downstream: line 2 {down:?} {up:?}");
}

/// No box, no caret: a `display: none` field answers `None`.
#[test]
fn a_hidden_field_has_no_caret_rect() {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        let el = __scope.create_element("input");
        el.set_attribute("value", "abc");
        el.set_attribute("style", "display: none");
        root.append_child(&el);
        *o.borrow_mut() = Some(el);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let el = out.borrow().clone().unwrap();
    assert_eq!(el.caret_rect(), None);
    el.set_attribute("style", "display: block; width: 100px; height: 20px");
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(
        el.caret_rect().is_some(),
        "positive control: shown, it has one"
    );
}

/// The caret rect is where paint draws the caret — the same answer the
/// app's own painter-matched caret geometry gives — and moves with the
/// offset, the line and the placeholder.
#[test]
fn caret_rect_is_the_painted_caret() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("ab\ncd");
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);

    let id = ta.node_id().0;
    let painted = |f: &mut Fixture, byte: usize| {
        let app = &mut f.app;
        let d = app.doc.as_ref().unwrap().borrow();
        RinchApp::input_caret_rect_for_offset(
            &d.tree,
            &mut app.hit_test_font_cx,
            &mut app.hit_test_layout_cx,
            id,
            byte,
        )
        .expect("laid out")
    };

    let head = ta.caret_rect().expect("a caret");
    let at_end = painted(&mut f, 5);
    assert!((head.x - at_end.0).abs() < 0.5, "{head:?} vs {at_end:?}");
    assert!((head.y - at_end.1).abs() < 0.5, "{head:?} vs {at_end:?}");
    assert_eq!(head.width, 0.0);
    assert!(
        (head.height - 16.0 * 1.2).abs() < 0.01,
        "paint's caret height"
    );

    // Off the line start: offset 1 is right of offset 0, on line 1; offset 3
    // starts line 2, a line lower (at the field's `line-height`, which paint
    // and the caret both lay the text out at since #320).
    let c0 = ta.caret_rect_at(0).unwrap();
    let c1 = ta.caret_rect_at(1).unwrap();
    let c3 = ta.caret_rect_at(3).unwrap();
    let p3 = painted(&mut f, 3);
    let p1 = painted(&mut f, 1);
    assert!(
        (c1.x - p1.0).abs() < 0.5 && (c1.y - p1.1).abs() < 0.5,
        "{c1:?} vs {p1:?}"
    );
    assert!(c1.x > c0.x + 3.0, "{c0:?} {c1:?}");
    assert!(
        (c3.x - p3.0).abs() < 0.5 && (c3.y - p3.1).abs() < 0.5,
        "{c3:?} vs {p3:?}"
    );
    assert!(
        (c3.x - c0.x).abs() < 0.5 && c3.y > c0.y + 15.0,
        "{c0:?} {c3:?}"
    );
    // The textarea's padding is in it: 10px page padding + 6px.
    let (bx, by) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        let (x, y, _, _) = painted_element_box(&d.tree, id);
        (x, y)
    };
    assert!((c0.x - (bx + 6.0)).abs() < 0.5, "{c0:?} box {bx}");
    // The top padding too (the caret's top is its line's, which Parley puts
    // a little above the content box for this face).
    assert!((c0.y - (by + 4.0)).abs() < 2.0, "{c0:?} box {by}");
    let p0 = painted(&mut f, 0);
    assert!((c0.y - p0.1).abs() < 0.5, "{c0:?} vs {p0:?}");

    // An empty single-line field with a placeholder: the caret at its start,
    // centred as paint centres the placeholder's line.
    let input = f.input.clone();
    let r = input.caret_rect().expect("an empty field has a caret");
    let (ix, iy, _, ih) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, input.node_id().0)
    };
    assert!((r.x - (ix + 8.0)).abs() < 0.5, "{r:?}");
    assert!(
        (r.y + r.height / 2.0 - (iy + ih / 2.0)).abs() < 2.0,
        "centred: {r:?} in {iy}..{}",
        iy + ih
    );
    assert_eq!(f.tile.caret_rect(), None, "not a text control");
}

/// A password field's caret is measured over the bullets paint draws, not
/// the value.
#[test]
fn a_password_fields_caret_is_measured_over_its_bullets() {
    let ids: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        for ty in ["password", "text"] {
            let field = __scope.create_element("input");
            field.set_attribute("type", ty);
            field.set_attribute("value", "WWWWWW");
            field.set_attribute(
                "style",
                "width: 300px; height: 30px; font-size: 16px; padding: 0; border: 0",
            );
            root.append_child(&field);
        }
        let k = root.children();
        *ids_in.borrow_mut() = Some((k[0].clone(), k[1].clone()));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (pw, plain) = ids.borrow_mut().take().unwrap();
    let pw6 = pw.caret_rect_at(6).unwrap().x - pw.caret_rect_at(0).unwrap().x;
    let plain6 = plain.caret_rect_at(6).unwrap().x - plain.caret_rect_at(0).unwrap().x;
    assert!(
        pw6 > 1.0 && pw6 < plain6 * 0.8,
        "bullets are narrower than W: {pw6} vs {plain6}"
    );
}
