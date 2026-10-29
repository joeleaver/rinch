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

fn node_by_id(app: &RinchApp, wanted: &str) -> u64 {
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

fn focused(app: &RinchApp) -> Option<u64> {
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

type Render = Box<dyn Fn(&mut RenderScope, Callback) -> NodeHandle>;

#[test]
fn checkbox_radio_and_switch_toggle_once_per_press_on_their_label() {
    let cases: Vec<(&str, &str, &str, Render)> = vec![
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

#[test]
fn the_control_is_resolved_after_the_labels_own_click() {
    // Chrome resolves `label.control` after the label's listeners: a label
    // whose own handler removes its control forwards to nothing.
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font-size: 16px; line-height: 20px");
        let label = scope.create_element("label");
        label.set_attribute("id", "l14");
        label.set_attribute("for", "c14");
        label.set_attribute("style", "display: inline-block; width: 120px; height: 24px");
        label.append_child(&scope.create_text("Fourteen"));
        let wrapper = scope.create_element("div");
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("id", "c14");
        wrapper.append_child(&input);
        let w = scope.register_handler(logger(&l, "w14"));
        wrapper.set_attribute("data-rid", &w.0.to_string());
        let a = scope.register_handler({
            let l = l.clone();
            let wrapper = wrapper.clone();
            move || {
                l.borrow_mut().push("a14");
                wrapper.remove();
            }
        });
        label.set_attribute("data-rid", &a.0.to_string());
        root.append_child(&label);
        root.append_child(&wrapper);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    click(&mut app, "l14");
    assert_eq!(
        *log.borrow(),
        vec!["a14"],
        "the removed control is not clicked"
    );
    assert_eq!(focused(&app), None, "nor focused");
}

// ── review-2036 probes ──────────────────────────────────────────────────────

fn down(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        VP,
        1.0,
    );
}
fn up(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        VP,
        1.0,
    );
}
fn mv(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
}

#[test]
fn review_probe_press_only() {
    let mut f = mount();
    let p = point_in(&f.app, "l1");
    down(&mut f.app, p);
    eprintln!(
        "PROBE press-only: log={:?} focus={:?}",
        f.take(),
        focused(&f.app)
    );
    let far = point_in(&f.app, "l12");
    mv(&mut f.app, (far.0 + 300.0, far.1 + 200.0));
    up(&mut f.app, (far.0 + 300.0, far.1 + 200.0));
    eprintln!("PROBE drag-off release: log={:?}", f.take());
}

#[test]
fn review_probe_drag_select_in_label() {
    let mut f = mount();
    let (x, y) = point_in(&f.app, "l1");
    down(&mut f.app, (x - 2.0, y));
    eprintln!(
        "PROBE after down: log={:?} focus={:?} sel={:?} selecting={}",
        f.take(),
        focused(&f.app),
        f.app
            .text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset)),
        f.app.text_selecting
    );
    for dx in [10.0, 20.0, 30.0] {
        mv(&mut f.app, (x + dx, y));
    }
    up(&mut f.app, (x + 30.0, y));
    eprintln!(
        "PROBE drag-select l1: log={:?} focus={:?} sel={:?}",
        f.take(),
        focused(&f.app),
        f.app
            .text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset))
    );

    // A wrapping label around a text field: drag across "Two".
    let (x, y) = point_in(&f.app, "l2");
    down(&mut f.app, (x - 2.0, y));
    eprintln!(
        "PROBE l2 after down: focus_target={:?} sel={:?} selecting={}",
        f.app.focus_target,
        f.app
            .text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset)),
        f.app.text_selecting
    );
    for dx in [10.0, 20.0, 28.0] {
        mv(&mut f.app, (x + dx, y));
    }
    up(&mut f.app, (x + 28.0, y));
    eprintln!(
        "PROBE drag-select l2: focus_target={:?} sel={:?} input_sel={:?}",
        f.app.focus_target,
        f.app
            .text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset)),
        f.app
            .focused_input_state
            .as_ref()
            .map(|s| s.selection.clone())
    );
}

#[test]
fn review_probe_double_click() {
    let mut f = mount();
    let p = point_in(&f.app, "l1");
    press(&mut f.app, p, MouseButton::Left);
    press(&mut f.app, p, MouseButton::Left);
    eprintln!("PROBE dblclick l1: log={:?}", f.take());
    let (mut app, count) = mount_component(|s, cb| {
        Checkbox {
            label: "Accept".into(),
            onchange: Some(cb),
            ..Default::default()
        }
        .render(s, &[])
    });
    let [on_text, _] = component_points(&app, "rinch-checkbox__label", "rinch-checkbox__box");
    press(&mut app, on_text, MouseButton::Left);
    press(&mut app, on_text, MouseButton::Left);
    eprintln!(
        "PROBE dblclick Checkbox: count={} focus={:?}",
        count.get(),
        app.focus_target
    );
}

#[test]
fn review_probe_keyboard_on_focused_label() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "font-size: 16px; line-height: 20px",
                label { id: "kl", tabindex: "0", "for": "kb", style: "display: inline-block; width: 120px; height: 24px", "K" }
                button { id: "kb", onclick: logger(&l, "kb"), "B" }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let kl = node_by_id(&app, "kl");
    app.try_focus_input(kl);
    eprintln!("PROBE focus kl: {:?} (kl={kl})", app.focus_target);
    for k in [KeyCode::Enter, KeyCode::Space] {
        app.handle_event(
            PlatformEvent::KeyDown {
                key: k,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        );
        app.handle_event(
            PlatformEvent::KeyUp {
                key: k,
                logical_key: None,
                modifiers: Modifiers::default(),
            },
            VP,
            1.0,
        );
    }
    eprintln!("PROBE keyboard on focused label: log={:?}", log.borrow());
}

#[test]
fn review_probe_drag_select_user_select_text() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "font-size: 16px; line-height: 20px; font-family: sans-serif; user-select: text",
                div { style: "height: 30px",
                    label { id: "sa", "for": "sc", style: "display: inline-block; width: 300px; height: 24px; user-select: text", "Selectable label text here" }
                    div { style: "display: inline-block", onclick: logger(&l, "sw"), input { r#type: "checkbox", id: "sc" } }
                }
                div { style: "height: 30px",
                    label { id: "sb", style: "display: inline-block; width: 300px; height: 24px; user-select: text", "Plain label text here" }
                }
                div { style: "height: 30px",
                    label { id: "st", style: "display: inline-block; width: 500px; height: 24px; user-select: text", "Wrapping label text ", input { id: "sti", value: "hello", oninput: |_v: String| {} } }
                }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let sel = |app: &RinchApp| {
        app.text_selection
            .as_ref()
            .map(|s| (s.ifc_node_id, s.anchor_offset, s.focus_offset))
    };
    for id in ["sb", "sa", "st"] {
        let (x, y) = point_in(&app, id);
        down(&mut app, (x - 2.0, y));
        let after_down = (sel(&app), app.text_selecting, app.focus_target);
        for dx in [20.0, 40.0, 70.0] {
            mv(&mut app, (x + dx, y));
        }
        up(&mut app, (x + 70.0, y));
        eprintln!(
            "PROBE sel-drag #{id}: after_down={after_down:?} final sel={:?} focus={:?} log={:?}",
            sel(&app),
            app.focus_target,
            std::mem::take(&mut *log.borrow_mut())
        );
    }
}

#[test]
fn review_probe_refocus_keeps_selection() {
    let mut f = mount();
    let t2 = node_by_id(&f.app, "t2");
    f.app.try_focus_input(t2);
    // select "ell" by keys: Home, Right, Shift+Right x3
    let key = |app: &mut RinchApp, k: KeyCode, shift: bool| {
        let modifiers = Modifiers {
            shift,
            ..Modifiers::default()
        };
        app.handle_event(
            PlatformEvent::KeyDown {
                key: k,
                logical_key: None,
                text: None,
                modifiers,
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        );
    };
    key(&mut f.app, KeyCode::Home, false);
    key(&mut f.app, KeyCode::ArrowRight, false);
    for _ in 0..3 {
        key(&mut f.app, KeyCode::ArrowRight, true);
    }
    eprintln!(
        "PROBE before label press: {:?} sel={:?}",
        f.app.focus_target,
        f.app
            .focused_input_state
            .as_ref()
            .map(|s| s.selection.clone())
    );
    click(&mut f.app, "l2");
    eprintln!(
        "PROBE after label press: {:?} sel={:?}",
        f.app.focus_target,
        f.app
            .focused_input_state
            .as_ref()
            .map(|s| s.selection.clone())
    );
}

// ── review-2036: fixtures for the HTML rules no test above pins ─────────────

fn mount_rules() -> Fixture {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "font-size: 16px; line-height: 20px; font-family: sans-serif",
                // R1. `for` across branches (a form laid out in two columns):
                //     the label and its control share no parent.
                div {
                    style: "height: 30px; display: flex",
                    div { label { id: "r1", "for": "rc1", style: "display: inline-block; width: 120px; height: 24px", "Col label" } }
                    div { span { onclick: logger(&l, "rw1"), input { r#type: "checkbox", id: "rc1" } } }
                }
                // R2. A link inside a label is interactive content.
                div {
                    style: "height: 30px",
                    label {
                        id: "r2",
                        "for": "rc2",
                        style: "display: inline-block; width: 300px; height: 24px",
                        a { id: "ra2", href: "#", style: "display: inline-block; width: 60px; height: 20px", onclick: logger(&l, "link2"), "terms" }
                        " Agree"
                    }
                    span { onclick: logger(&l, "rw2"), input { r#type: "checkbox", id: "rc2" } }
                }
                // R3. A hidden input is not labelable: the checkbox after it is.
                div {
                    style: "height: 30px",
                    label {
                        id: "r3",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Three "
                        input { r#type: "hidden", id: "rh3" }
                        span { onclick: logger(&l, "rw3"), input { r#type: "checkbox", id: "rc3" } }
                    }
                }
                // R4. Tree order is depth-first: the checkbox nested in the
                //     first child comes before the button after it.
                div {
                    style: "height: 30px",
                    label {
                        id: "r4",
                        style: "display: inline-block; width: 300px; height: 24px",
                        "Four "
                        span {
                            span { onclick: logger(&l, "rw4"), input { r#type: "checkbox", id: "rc4" } }
                            button { id: "rb4", onclick: logger(&l, "rb4"), "b" }
                        }
                    }
                }
                // R5. A label whose text sits in an anonymous block box (#566).
                div {
                    style: "height: 60px",
                    label {
                        id: "r5",
                        "for": "rc5",
                        style: "display: block; width: 300px",
                        "Five text"
                        div { style: "height: 20px", "block" }
                    }
                    span { onclick: logger(&l, "rw5"), input { r#type: "checkbox", id: "rc5" } }
                }
            }
        }
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    Fixture { app, log }
}

#[test]
fn review_for_reaches_a_control_in_another_branch() {
    let mut f = mount_rules();
    click(&mut f.app, "r1");
    assert_eq!(f.take(), vec!["rw1"]);
    assert_eq!(focused(&f.app), Some(node_by_id(&f.app, "rc1")));
}

#[test]
fn review_a_link_inside_a_label_is_its_own() {
    let mut f = mount_rules();
    click(&mut f.app, "ra2");
    assert_eq!(f.take(), vec!["link2"]);
    // Positive control: the label's own text forwards.
    let (x, y) = point_in(&f.app, "r2");
    let w = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node_by_id(&f.app, "r2")).2
    };
    press(&mut f.app, (x + w - 12.0, y), MouseButton::Left);
    assert_eq!(f.take(), vec!["rw2"]);
}

#[test]
fn review_a_hidden_input_is_not_labelable() {
    let mut f = mount_rules();
    click(&mut f.app, "r3");
    assert_eq!(f.take(), vec!["rw3"]);
    assert_eq!(focused(&f.app), Some(node_by_id(&f.app, "rc3")));
}

#[test]
fn review_the_first_labelable_is_depth_first() {
    let mut f = mount_rules();
    click(&mut f.app, "r4");
    assert_eq!(f.take(), vec!["rw4"]);
}

#[test]
fn review_a_press_on_an_anonymous_box_reaches_its_label() {
    let mut f = mount_rules();
    let hit = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        let (x, y, _, _) = painted_element_box(&d.tree, node_by_id(&f.app, "r5"));
        let p = (x + 4.0, y + 8.0);
        let h = f.app.shared_hit(&d, p.0, p.1);
        h.map(|h| {
            (
                h,
                d.tree.get(h).and_then(|n| n.tag().map(str::to_owned)),
                d.tree.get(h).and_then(|n| n.parent),
            )
        })
    };
    eprintln!("PROBE r5 hit = {hit:?}");
    let (x, y, _, _) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node_by_id(&f.app, "r5"))
    };
    press(&mut f.app, (x + 4.0, y + 8.0), MouseButton::Left);
    assert_eq!(f.take(), vec!["rw5"]);
}
