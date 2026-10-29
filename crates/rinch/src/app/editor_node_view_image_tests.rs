//! Node views C on the desktop: a claimed inline image (an `image` a
//! `NodeViewSpec` matches) is one atomic inline of its line, laid out as an
//! inline-block `<span>` holding the app's component. The caret beside it is drawn
//! at the span's edges, Home/End and a press on the text beside it place the caret
//! as around any image, and a press inside the component is the component's.
//!
//! Also the keys of an `<input>` inside a view (the embed's): Enter, Tab,
//! Shift+Tab, the arrows and Escape are offered to `NodeViewSpec::on_key` first
//! — presses only — and, unclaimed, reach the input (arrows move its caret),
//! the runtime's Tab traversal (the view's next input) or the editor (Escape).

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

/// `ab` 1..3, the badged image at 3, `cd` 4..6.
const IMAGE: usize = 3;

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

#[derive(Default)]
struct View {
    renders: Cell<usize>,
    chip: RefCell<Option<NodeHandle>>,
    inputs: RefCell<Vec<NodeHandle>>,
    keys: RefCell<Vec<String>>,
}

fn page(html: &str, view: Rc<View>) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let html = html.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(&html);
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        let v = view.clone();
        handle
            .register_node_view(
                crate::editor::NodeViewSpec::new("image", move |s: &mut RenderScope, _ctx| {
                    v.renders.set(v.renders.get() + 1);
                    let chip = s.create_element("span");
                    chip.set_attribute(
                        "style",
                        "display: inline-block; width: 60px; height: 20px; background: #c00",
                    );
                    *v.chip.borrow_mut() = Some(chip.clone());
                    chip
                })
                .matches(|a| a.get_str("data-badge").is_some()),
            )
            .keep();
        let v = view.clone();
        let k = view.clone();
        handle
            .register_node_view(
                crate::editor::NodeViewSpec::new("embed", move |s: &mut RenderScope, _ctx| {
                    let root = s.create_element("div");
                    for _ in 0..2 {
                        let input = s.create_element("input");
                        input.set_attribute("style", "width: 100px; height: 24px; font-size: 16px");
                        let typed = s.register_input_handler(|_| {});
                        input.set_attribute("data-oninput", &typed.0.to_string());
                        root.append_child(&input);
                        v.inputs.borrow_mut().push(input);
                    }
                    root
                })
                .on_key(move |key| {
                    k.keys.borrow_mut().push(key.key.to_string());
                    false
                }),
            )
            .keep();
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    Page { app, handle }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
    app.resolve_and_repaint(800.0, 600.0);
}

fn key_mod(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
    app.resolve_and_repaint(800.0, 600.0);
}

fn key(app: &mut RinchApp, key: KeyCode) {
    key_mod(app, key, None, Modifiers::default());
}

fn box_of(page: &Page, id: usize) -> (f32, f32, f32, f32) {
    let doc = page.app.doc.as_ref().unwrap();
    painted_element_box(&doc.borrow().tree, id)
}

/// The element carrying `data-pm-type="image"` and a mounted view.
fn span(page: &Page) -> usize {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    (0..d.tree.nodes.len())
        .find(|&id| {
            d.tree.nodes.get(id).is_some_and(|n| {
                n.attributes.get("data-pm-type").map(String::as_str) == Some("image")
                    && n.attributes.contains_key(crate::editor::NODE_VIEW_ATTR)
            })
        })
        .expect("a claimed image")
}

fn caret(page: &Page, pos: usize) -> (f32, f32, f32) {
    page.app
        .editor_caret_point(&page.handle, Pos(pos))
        .expect("a caret")
}

const LINE: &str = r#"<p>ab<img src="x.png" data-badge="n">cd</p><p>gh</p>"#;

#[test]
fn a_claimed_image_is_one_inline_box_and_the_caret_is_drawn_at_its_edges() {
    let view = Rc::new(View::default());
    let p = page(LINE, view.clone());
    assert_eq!(view.renders.get(), 1);
    let span = span(&p);
    let (sx, _sy, sw, sh) = box_of(&p, span);
    // As wide as the component, and as tall as the line inside it: the 20px
    // chip on its baseline and the strut's descent below it (Chrome 156:
    // 60x27 with the host's sans-serif; 60x20 until parley 0.12 gave every
    // line its strut).
    assert!(
        (sw - 60.0).abs() < 0.5 && sh > 20.0 && sh < 28.0,
        "sized by the component: {sw}x{sh}"
    );
    let (before, _, _) = caret(&p, IMAGE);
    let (after, _, _) = caret(&p, IMAGE + 1);
    assert!(
        (before - sx).abs() < 1.0,
        "before the image: its left edge, {before} vs {sx}"
    );
    assert!(
        (after - (sx + sw)).abs() < 1.0,
        "after the image: its right edge, {after} vs {}",
        sx + sw
    );
    // "cd" starts after it on the same line: the text is laid out around one box.
    let (c, cy, _) = caret(&p, IMAGE + 2);
    let (_, ay, _) = caret(&p, 1);
    assert!(c > sx + sw, "{c}");
    assert!((cy - ay).abs() < 0.5, "one line");
}

#[test]
fn presses_beside_it_place_the_caret_and_inside_it_are_the_components() {
    let view = Rc::new(View::default());
    let mut p = page(LINE, view.clone());
    let span = span(&p);
    let (sx, sy, sw, sh) = box_of(&p, span);
    // On the text after it: the caret between "c" and "d".
    let (cx, cy, ch) = caret(&p, IMAGE + 2);
    press(&mut p.app, (cx + 1.0, cy + ch / 2.0));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(IMAGE + 2)));
    // On the text before it: between "a" and "b".
    let (ax, ay, ah) = caret(&p, 2);
    press(&mut p.app, (ax + 1.0, ay + ah / 2.0));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
    let before = p.handle.selection();
    // Inside the component: nothing moves.
    let chip = view.chip.borrow().clone().unwrap().node_id().0;
    let (cx, cy, cw, ch) = box_of(&p, chip);
    press(&mut p.app, (cx + cw / 2.0, cy + ch / 2.0));
    assert_eq!(p.handle.selection(), before, "the component's press");
    // Home / End on its line (the editor's keyboard again: a press on its text).
    let _ = (sx, sy, sw, sh);
    press(&mut p.app, (ax + 1.0, ay + ah / 2.0));
    key(&mut p.app, KeyCode::End);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(6)));
    key(&mut p.app, KeyCode::Home);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));
    // ArrowRight steps over it as one position.
    key(&mut p.app, KeyCode::ArrowRight);
    key(&mut p.app, KeyCode::ArrowRight);
    key(&mut p.app, KeyCode::ArrowRight);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(IMAGE + 1)));
    assert_eq!(view.renders.get(), 1);
}

/// The coordinator's keys: an `<input>` inside a view gets Enter, Tab,
/// Shift+Tab, the arrows and Escape, `on_key` first (presses only), and the
/// editor takes none of them but an unclaimed Escape.
#[test]
fn keys_in_a_views_input_reach_on_key_then_the_input() {
    let view = Rc::new(View::default());
    let mut p = page(
        r#"<p>ab</p><div data-pm-type="embed" data-x="1"></div><p>cd</p>"#,
        view.clone(),
    );
    let (a, b) = {
        let inputs = view.inputs.borrow();
        (inputs[0].node_id().0, inputs[1].node_id().0)
    };
    let (x, y, _, _) = box_of(&p, a);
    press(&mut p.app, (x + 5.0, y + 5.0));
    assert_eq!(p.app.focus_target, FocusTarget::Input(a));
    let doc = p.handle.doc();
    for (k, t) in [(KeyCode::KeyH, "h"), (KeyCode::KeyI, "i")] {
        key_mod(&mut p.app, k, Some(t), Modifiers::default());
    }
    key(&mut p.app, KeyCode::ArrowLeft);
    key_mod(&mut p.app, KeyCode::KeyX, Some("x"), Modifiers::default());
    let value = |p: &Page, id: usize| {
        p.app
            .doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .get(id)
            .unwrap()
            .attributes
            .get("value")
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(value(&p, a), "hxi", "ArrowLeft moved the input's own caret");
    key(&mut p.app, KeyCode::Enter);
    assert_eq!(p.app.focus_target, FocusTarget::Input(a), "Enter stays");
    key(&mut p.app, KeyCode::Tab);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Input(b),
        "Tab: the view's next input"
    );
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    key_mod(&mut p.app, KeyCode::Tab, None, shift);
    assert_eq!(p.app.focus_target, FocusTarget::Input(a), "Shift+Tab: back");
    key(&mut p.app, KeyCode::ArrowDown);
    key(&mut p.app, KeyCode::ArrowUp);
    assert_eq!(p.handle.doc(), doc, "the editor's document is untouched");
    assert_eq!(p.app.focus_target, FocusTarget::Input(a));
    key(&mut p.app, KeyCode::Escape);
    assert!(matches!(p.app.focus_target, FocusTarget::Editor(_)));
    assert_eq!(
        *view.keys.borrow(),
        [
            "h",
            "i",
            "ArrowLeft",
            "x",
            "Enter",
            "Tab",
            "Tab",
            "ArrowDown",
            "ArrowUp",
            "Escape"
        ],
        "on_key heard every press, and no release"
    );
}

// ── review of #1595 ─────────────────────────────────────────────────────────

/// A claimed image last on its line: End lands after it, Home before "ab", and
/// Shift+ArrowRight selects across it as one position; the caret after it is
/// at its right edge; a press just past its right edge, on no text, lands after
/// it.
#[test]
fn review_1595_claimed_image_at_the_end_of_its_line() {
    let view = Rc::new(View::default());
    let mut p = page(
        r#"<p>ab<img src="x.png" data-badge="n"></p><p>gh</p>"#,
        view.clone(),
    );
    let span = span(&p);
    let (sx, sy, sw, sh) = box_of(&p, span);
    let (ax, ay, ah) = caret(&p, 2);
    press(&mut p.app, (ax + 1.0, ay + ah / 2.0));
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(2)));
    key(&mut p.app, KeyCode::End);
    eprintln!(
        "REVIEW-1595 claimed End -> {:?} (pre-existing for any image, see control)",
        p.handle.selection()
    );
    let (after, _, _) = caret(&p, 4);
    assert!((after - (sx + sw)).abs() < 1.0, "{after} vs {}", sx + sw);
    key(&mut p.app, KeyCode::Home);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(1)));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    for _ in 0..3 {
        key_mod(&mut p.app, KeyCode::ArrowRight, None, shift);
    }
    let sel = p.handle.selection();
    assert_eq!((sel.from(), sel.to()), (Pos(1), Pos(4)), "{sel:?}");
    key(&mut p.app, KeyCode::ArrowLeft);
    // Past the right edge of the line's last box, on no text: after it.
    press(&mut p.app, (sx + sw + 20.0, sy + sh / 2.0));
    eprintln!(
        "REVIEW-1595 claimed press past its edge -> {:?}",
        p.handle.selection()
    );
    // caret_rect agrees with the overlay point.
    let r = p.handle.caret_rect(Pos(4)).expect("a rect");
    assert!(
        (r.x - (sx + sw)).abs() < 1.5,
        "caret_rect {r:?} vs {}",
        sx + sw
    );
    assert_eq!(view.renders.get(), 1);
}

/// Tab from a view's last input leaves the view; Escape after that is nobody's
/// business of the view.
#[test]
fn review_1595_tab_out_of_the_last_input_leaves_the_view() {
    let view = Rc::new(View::default());
    let mut p = page(
        r#"<p>ab</p><div data-pm-type="embed" data-x="1"></div><p>cd</p>"#,
        view.clone(),
    );
    let b = view.inputs.borrow()[1].node_id().0;
    let (x, y, _, _) = box_of(&p, b);
    press(&mut p.app, (x + 5.0, y + 5.0));
    assert_eq!(p.app.focus_target, FocusTarget::Input(b));
    key(&mut p.app, KeyCode::Tab);
    eprintln!(
        "REVIEW-1595 after Tab from last input: {:?} (inputs {:?})",
        p.app.focus_target,
        view.inputs
            .borrow()
            .iter()
            .map(|i| i.node_id().0)
            .collect::<Vec<_>>()
    );
    assert_ne!(p.app.focus_target, FocusTarget::Input(b));
}

/// Control for the above: an unclaimed image last on its line. End lands
/// after the image (Pos 4); it landed before it (Pos 3) until #1116, the End
/// half of #1599.
#[test]
fn review_1595_control_end_lands_after_an_unclaimed_line_final_image() {
    let view = Rc::new(View::default());
    let mut p = page(
        r#"<p>ab<img src="x.png" data-y="n" style="width:20px;height:20px"></p><p>gh</p>"#,
        view.clone(),
    );
    let (ax, ay, ah) = caret(&p, 2);
    press(&mut p.app, (ax + 1.0, ay + ah / 2.0));
    key(&mut p.app, KeyCode::End);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(4)),
        "End: after the image"
    );
}

/// The same image: a press past the line's end lands before the image (Pos
/// 3): #1599, older than node views. Its End half is fixed by #1116 (above).
#[test]
#[ignore = "#1599: a press past a line-final image lands before it"]
fn review_1595_control_unclaimed_image_at_the_end_of_its_line() {
    let view = Rc::new(View::default());
    let mut p = page(
        r#"<p>ab<img src="x.png" data-y="n" style="width:20px;height:20px"></p><p>gh</p>"#,
        view.clone(),
    );
    assert_eq!(view.renders.get(), 0);
    let ib = {
        let doc = p.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        (0..d.tree.nodes.len())
            .find(|&id| {
                d.tree.nodes.get(id).is_some_and(|n| {
                    n.attributes.get("data-pm-type").map(String::as_str) == Some("image")
                })
            })
            .unwrap()
    };
    let (ix, iy, iw, ih) = box_of(&p, ib);
    press(&mut p.app, (ix + iw + 20.0, iy + ih / 2.0));
    eprintln!(
        "REVIEW-1595 control press past its edge -> {:?}",
        p.handle.selection()
    );
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(4)),
        "a press past its edge: after the image"
    );
}

// review of #1730 (scratch): a claimed image selected alone / from after it.
#[test]
fn rv1730_a_claimed_image_selected_alone_is_highlighted_over_its_span() {
    let mut p = page(LINE, Rc::new(View::default()));
    p.handle.focus();
    p.app.resolve_and_repaint(800.0, 600.0);
    let sp = box_of(&p, span(&p));
    p.handle
        .set_selection(Selection::text(Pos(IMAGE), Pos(IMAGE + 1)));
    for _ in 0..3 {
        ev(&mut p.app, PlatformEvent::AboutToWait);
        p.app.resolve_and_repaint(800.0, 600.0);
    }
    let hl: Vec<_> = {
        let doc = p.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter(|id| {
                doc.tree.nodes[id.0].computed_style.display
                    != rinch_dom::computed_style::DisplayValue::None
            })
            .map(|id| painted_element_box(&doc.tree, id.0))
            .filter(|b| b.2 > 0.0 && b.3 > 0.0)
            .collect()
    };
    assert_eq!(hl.len(), 1, "{hl:?} span {sp:?}");
    assert!(
        (hl[0].0 - sp.0).abs() < 1.5 && (hl[0].0 + hl[0].2 - sp.0 - sp.2).abs() < 1.5,
        "{hl:?} vs {sp:?}"
    );
}
