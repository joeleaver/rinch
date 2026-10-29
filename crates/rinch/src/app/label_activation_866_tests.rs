//! A press on a `<label>` activates its control (issue #866).
//!
//! HTML's label activation behaviour, as Chrome 156 runs it (measured with a
//! real mouse through CDP, `pointerdown … click@label | focus@control |
//! click@control`): after the label's own click, the label's **labeled
//! control** — the element its `for` names, else its first labelable
//! descendant — is focused and then clicked, unless the press landed on
//! interactive content inside the label (a `<button>`, a link, the control
//! itself) or the control is disabled (its own `disabled`, or a disabled
//! `<fieldset>` around it). A `for` that names nothing labelable labels
//! nothing: it does not fall back to a descendant.
//!
//! Desktop's `click` is its `data-rid` dispatch, so "click the control" is the
//! nearest live `data-rid` above the control — the walk a press on the control
//! itself takes. Desktop used to do none of it: a press on a `<label for>`
//! dispatched **0** times and focused nothing. rinch-web gets it from the
//! browser (its `#272` fixtures in `rinch-web/tests/keyboard_activation.rs`).
//!
//! A `data-rid` the press already dispatched is **not** dispatched again by the
//! forwarded click: `Checkbox`, `Radio` and `Switch` put their handler on the
//! `<label>` that wraps the `<input>`, so the control's nearest `data-rid` is
//! the label's own, and a browser's second click there is the one rinch-web
//! suppresses as the gesture's duplicate (#272). One press, one toggle.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use rinch_components::{Checkbox, Radio, Switch};
use rinch_core::{Callback, Component};
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 1200);

type Log = Rc<RefCell<Vec<&'static str>>>;

fn logger(log: &Log, what: &'static str) -> impl Fn() + 'static {
    let log = log.clone();
    move || log.borrow_mut().push(what)
}

struct Fixture {
    app: RinchApp,
    log: Log,
}

/// One row per case. Every label is a fixed `inline-block` box with its text
/// first, so a press 4px inside its left edge lands on the label's own text.
fn mount() -> Fixture {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = rsx! {
            div {
                style: "font-size: 16px; line-height: 20px; font-family: sans-serif",
                // 1. `for` a checkbox inside a clickable wrapper.
                div {
                    style: "height: 30px",
                    label { id: "l1", "for": "c1", style: "display: inline-block; width: 120px; height: 24px", "One" }
                    div {
                        id: "w1",
                        style: "display: inline-block",
                        onclick: logger(&l, "w1"),
                        input { r#type: "checkbox", id: "c1", style: "width: 13px; height: 13px" }
                    }
                }
                // 2. A label wrapping a text field.
                div {
                    style: "height: 30px",
                    label {
                        id: "l2",
                        style: "display: inline-block; width: 400px; height: 24px",
                        "Two "
                        input { id: "t2", value: "hello", oninput: |_v: String| {} }
                    }
                }
                // 3. `for` a button.
                div {
                    style: "height: 30px",
                    label { id: "l3", "for": "b3", style: "display: inline-block; width: 120px; height: 24px", "Three" }
                    button { id: "b3", onclick: logger(&l, "b3"), "B3" }
                }
                // 4. `for` a disabled checkbox.
                div {
                    style: "height: 30px",
                    label { id: "l4", "for": "c4", style: "display: inline-block; width: 120px; height: 24px", "Four" }
                    div {
                        style: "display: inline-block",
                        onclick: logger(&l, "w4"),
                        input { r#type: "checkbox", id: "c4", disabled: true }
                    }
                }
                // 5. A button inside a label whose `for` names a checkbox.
                div {
                    style: "height: 30px",
                    label {
                        id: "l5",
                        "for": "c5",
                        style: "display: inline-block; width: 300px; height: 24px",
                        button { id: "b5", onclick: logger(&l, "b5"), "inner" }
                        " Five"
                    }
                    div {
                        style: "display: inline-block",
                        onclick: logger(&l, "w5"),
                        input { r#type: "checkbox", id: "c5" }
                    }
                }
                // 6. `for` a `<select>`.
                div {
                    style: "height: 30px",
                    label { id: "l6", "for": "s6", style: "display: inline-block; width: 120px; height: 24px", "Six" }
                    select { id: "s6", option { "a" } option { "b" } }
                }
                // 7. `for` a `<textarea>`.
                div {
                    style: "height: 60px",
                    label { id: "l7", "for": "ta7", style: "display: inline-block; width: 120px; height: 24px", "Seven" }
                    textarea { id: "ta7", style: "height: 40px", oninput: |_v: String| {} }
                }
                // 8. A clickable label (A) `for` a control in another clickable (B).
                div {
                    style: "height: 30px",
                    label {
                        id: "l8",
                        "for": "c8",
                        style: "display: inline-block; width: 120px; height: 24px",
                        onclick: logger(&l, "a8"),
                        "Eight"
                    }
                    div {
                        style: "display: inline-block",
                        onclick: logger(&l, "b8"),
                        input { r#type: "checkbox", id: "c8" }
                    }
                }
                // 9. A card whose handler is the nearest one above both the
                //    label and the control the label wraps.
                div {
                    style: "height: 30px",
                    onclick: logger(&l, "card9"),
                    label {
                        id: "l9",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Nine "
                        input { r#type: "checkbox", id: "c9" }
                    }
                }
                // 10. A `for` naming no element labels nothing, even with a
                //     labelable descendant.
                div {
                    style: "height: 30px",
                    label {
                        id: "l10",
                        "for": "nope",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Ten "
                        span { onclick: logger(&l, "w10"), input { r#type: "checkbox", id: "c10" } }
                    }
                }
                // 11. `for` wins over the first labelable descendant.
                div {
                    style: "height: 30px",
                    label {
                        id: "l11",
                        "for": "c11b",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Eleven "
                        span { onclick: logger(&l, "a11"), input { r#type: "checkbox", id: "c11a" } }
                    }
                    span { onclick: logger(&l, "b11"), input { r#type: "checkbox", id: "c11b" } }
                }
                // 12. A disabled fieldset around the label and its control.
                fieldset {
                    disabled: true,
                    style: "height: 30px; margin: 0; padding: 0; border: 0",
                    label { id: "l12", "for": "c12", style: "display: inline-block; width: 120px; height: 24px", "Twelve" }
                    div {
                        style: "display: inline-block",
                        onclick: logger(&l, "w12"),
                        input { r#type: "checkbox", id: "c12" }
                    }
                }
                // 13. A wrapping label whose first labelable descendant is a
                //     button (Chrome forwards to it, not to the checkbox after it).
                div {
                    style: "height: 30px",
                    label {
                        id: "l13",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Thirteen "
                        button { id: "b13", onclick: logger(&l, "b13"), "x" }
                        span { onclick: logger(&l, "w13"), input { r#type: "checkbox", id: "c13" } }
                    }
                }
            }
        };
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    Fixture { app, log }
}

fn node_by_id(app: &RinchApp, wanted: &str) -> usize {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(wanted))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no #{wanted}"))
}

/// 4px inside the node's left edge, at its vertical centre.
fn point_in(app: &RinchApp, id: &str) -> (f32, f32) {
    let nid = node_by_id(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, nid);
    assert!(w > 8.0 && h > 8.0, "#{id} has a box: {w}x{h}");
    (x + 4.0, y + h / 2.0)
}

fn press(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

fn click(app: &mut RinchApp, id: &str) {
    let p = point_in(app, id);
    press(app, p, MouseButton::Left);
}

fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

impl Fixture {
    fn take(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

#[test]
fn a_label_for_a_checkbox_focuses_it_and_clicks_its_wrapper_once() {
    let mut f = mount();
    // Positive control: a press on the control itself reaches the wrapper.
    click(&mut f.app, "c1");
    assert_eq!(f.take(), vec!["w1"], "positive control");
    f.app.set_focus_target(FocusTarget::None);

    click(&mut f.app, "l1");
    assert_eq!(f.take(), vec!["w1"], "the label clicks its control, once");
    assert_eq!(focused(&f.app), Some(node_by_id(&f.app, "c1")));
}

#[test]
fn a_label_wrapping_a_text_field_focuses_the_field() {
    let mut f = mount();
    click(&mut f.app, "l2");
    let t2 = node_by_id(&f.app, "t2");
    assert_eq!(f.app.focus_target, FocusTarget::Input(t2));
    assert_eq!(f.app.focused_input_node_id, Some(t2));
    // Typing lands in it.
    f.app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyA,
            logical_key: None,
            text: Some("!".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    let value = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, t2)
            .map(|v| v.into_owned())
            .unwrap_or_default()
    };
    assert!(value.contains('!'), "the key reached the field: {value:?}");
}

#[test]
fn a_label_for_a_button_focuses_and_clicks_it() {
    let mut f = mount();
    click(&mut f.app, "l3");
    assert_eq!(f.take(), vec!["b3"]);
    assert_eq!(focused(&f.app), Some(node_by_id(&f.app, "b3")));
}

#[test]
fn a_label_for_a_textarea_or_select_focuses_it() {
    let mut f = mount();
    click(&mut f.app, "l7");
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Input(node_by_id(&f.app, "ta7"))
    );
    click(&mut f.app, "l6");
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(node_by_id(&f.app, "s6")),
        "a select is focused closed, as in Chrome"
    );
    assert!(!f.app.is_select_open(), "and its popup does not open");
}

#[test]
fn a_disabled_control_is_neither_focused_nor_clicked() {
    let mut f = mount();
    click(&mut f.app, "l4");
    assert_eq!(f.take(), Vec::<&str>::new(), "its own `disabled`");
    assert_eq!(focused(&f.app), None);
    click(&mut f.app, "l12");
    assert_eq!(f.take(), Vec::<&str>::new(), "a disabled fieldset");
    assert_eq!(focused(&f.app), None);
}

#[test]
fn a_press_on_interactive_content_inside_a_label_is_that_elements_alone() {
    let mut f = mount();
    click(&mut f.app, "b5");
    assert_eq!(f.take(), vec!["b5"], "the inner button only, no forward");
    // Positive control: a press on the same label beside the button forwards.
    let (x, y) = point_in(&f.app, "l5");
    let w = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node_by_id(&f.app, "l5")).2
    };
    press(&mut f.app, (x + w - 12.0, y), MouseButton::Left);
    assert_eq!(
        f.take(),
        vec!["w5"],
        "positive control: the label itself forwards"
    );
}

#[test]
fn a_wrapping_label_activates_its_first_labelable_descendant() {
    let mut f = mount();
    click(&mut f.app, "l13");
    assert_eq!(
        f.take(),
        vec!["b13"],
        "the button comes first in tree order"
    );
}

#[test]
fn a_clickable_label_and_its_controls_clickable_both_run_once() {
    let mut f = mount();
    click(&mut f.app, "l8");
    assert_eq!(
        f.take(),
        vec!["a8", "b8"],
        "the label's click, then the control's"
    );
}

#[test]
fn a_handler_above_both_label_and_control_runs_once() {
    let mut f = mount();
    click(&mut f.app, "l9");
    assert_eq!(f.take(), vec!["card9"]);
    assert_eq!(focused(&f.app), Some(node_by_id(&f.app, "c9")));
}

#[test]
fn for_resolution_follows_the_html_rules() {
    let mut f = mount();
    click(&mut f.app, "l10");
    assert_eq!(
        f.take(),
        Vec::<&str>::new(),
        "a dangling `for` labels nothing"
    );
    assert_eq!(focused(&f.app), None);
    click(&mut f.app, "l11");
    assert_eq!(f.take(), vec!["b11"], "`for` wins over the descendant");
}

#[test]
fn a_right_press_on_a_label_activates_nothing() {
    let mut f = mount();
    let p = point_in(&f.app, "l1");
    press(&mut f.app, p, MouseButton::Right);
    assert_eq!(f.take(), Vec::<&str>::new());
    assert_eq!(focused(&f.app), None);
}

// ── The components that wrap their `<input>` in a clickable `<label>` ──

fn mount_component(
    render: impl Fn(&mut RenderScope, Callback) -> NodeHandle + 'static,
) -> (RinchApp, Rc<Cell<u32>>) {
    let count = Rc::new(Cell::new(0u32));
    let c = count.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let c = c.clone();
        let cb = Callback::new(move || c.set(c.get() + 1));
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 400px; font-size: 16px; line-height: 20px");
        let node = render(scope, cb);
        node.set_attribute("id", "comp");
        root.append_child(&node);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, count)
}

/// The component's label text (the `span` inside its body) and its box.
fn component_points(app: &RinchApp, text_class: &str, box_class: &str) -> [(f32, f32); 2] {
    let d = app.doc.as_ref().unwrap().borrow();
    let find = |class: &str| {
        let (nid, _) = d
            .tree
            .nodes
            .iter()
            .find(|(_, n)| {
                n.attributes
                    .get("class")
                    .is_some_and(|c| c.split_whitespace().any(|k| k == class))
            })
            .unwrap_or_else(|| panic!("no .{class}"));
        let (x, y, w, h) = painted_element_box(&d.tree, nid);
        assert!(w > 0.0 && h > 0.0, ".{class} has a box");
        (x + w / 2.0, y + h / 2.0)
    };
    [find(text_class), find(box_class)]
}

#[test]
fn checkbox_radio_and_switch_toggle_once_per_press_on_their_label() {
    let cases: Vec<(
        &str,
        &str,
        &str,
        Box<dyn Fn(&mut RenderScope, Callback) -> NodeHandle>,
    )> = vec![
        (
            "Checkbox",
            "rinch-checkbox__label",
            "rinch-checkbox__box",
            Box::new(|s, cb| {
                Checkbox {
                    label: "Accept".into(),
                    onchange: Some(cb),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
        (
            "Radio",
            "rinch-radio__label",
            "rinch-radio__indicator",
            Box::new(|s, cb| {
                Radio {
                    label: "Pick".into(),
                    onchange: Some(cb),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
        (
            "Switch",
            "rinch-switch__label",
            "rinch-switch__track",
            Box::new(|s, cb| {
                Switch {
                    label: "Enable".into(),
                    onchange: Some(cb),
                    ..Default::default()
                }
                .render(s, &[])
            }),
        ),
    ];
    for (name, text, boxed, render) in cases {
        let render = Rc::new(render);
        let (mut app, count) = mount_component(move |s, cb| render(s, cb));
        let [on_text, on_box] = component_points(&app, text, boxed);
        press(&mut app, on_text, MouseButton::Left);
        assert_eq!(
            count.get(),
            1,
            "{name}: a press on its label text toggles once"
        );
        press(&mut app, on_box, MouseButton::Left);
        assert_eq!(count.get(), 2, "{name}: a press on its box toggles once");
    }
}
