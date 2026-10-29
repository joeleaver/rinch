//! A press on a field component's label focuses its field, on desktop
//! (issue #2034).
//!
//! `rinch-components/tests/field_label_for_2034.rs` pins the markup — the
//! label's `for` is the field's id. Turning that `for` into focus is the
//! runtime's label activation (#866), so the five `for` cases here wait for
//! it. `Select` does not: its trigger is a `div`, which no `for` can reach on
//! either backend, so its label forwards the press through its own click
//! handler, and that half runs today.

use super::*;

use rinch_components::color_input::ColorInput;
use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::select::{Select, SelectOption};
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::Component;

const VIEWPORT: (f32, f32) = (800.0, 900.0);

fn mount() -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 20px; width: 300px");
        let parts = [
            TextInput {
                label: "Text".into(),
                ..Default::default()
            }
            .render(scope, &[]),
            Textarea {
                label: "Area".into(),
                ..Default::default()
            }
            .render(scope, &[]),
            NumberInput {
                label: "Number".into(),
                ..Default::default()
            }
            .render(scope, &[]),
            PasswordInput {
                label: "Password".into(),
                ..Default::default()
            }
            .render(scope, &[]),
            ColorInput {
                label: "Colour".into(),
                ..Default::default()
            }
            .render(scope, &[]),
            Select {
                label: "Fruit".into(),
                data: vec![SelectOption::new("apple", "Apple")],
                ..Default::default()
            }
            .render(scope, &[]),
        ];
        for part in &parts {
            root.append_child(part);
        }
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    app
}

fn by_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no .{class}"))
}

/// Press 4px inside `node`'s left edge — on the label's text.
fn press(app: &mut RinchApp, node: usize) {
    let (x, y) = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, node);
        assert!(w > 8.0 && h > 8.0, "node {node} has a box: {w}x{h}");
        (x + 4.0, y + h / 2.0)
    };
    for ev in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        app.handle_event(ev, (VIEWPORT.0 as u32, VIEWPORT.1 as u32), 1.0);
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn focused(app: &RinchApp) -> Option<usize> {
    match app.focus_target {
        FocusTarget::Input(id) | FocusTarget::Node(id) => Some(id),
        _ => None,
    }
}

#[test]
#[ignore = "needs #2036 (desktop label activation, #866)"]
fn a_press_on_each_field_label_focuses_its_field() {
    let mut app = mount();
    for (label, field) in [
        ("rinch-text-input__label", "rinch-text-input__input"),
        ("rinch-textarea__label", "rinch-textarea__input"),
        ("rinch-number-input__label", "rinch-number-input__input"),
        ("rinch-password-input__label", "rinch-password-input__input"),
        ("rinch-color-input__label", "rinch-color-input__input"),
    ] {
        app.set_focus_target(FocusTarget::None);
        let label = by_class(&app, label);
        press(&mut app, label);
        assert_eq!(focused(&app), Some(by_class(&app, field)), "{label}");
    }
}

#[test]
fn a_press_on_the_select_label_focuses_the_trigger() {
    let mut app = mount();
    let trigger = by_class(&app, "rinch-select__input");
    assert_ne!(focused(&app), Some(trigger));
    let label = by_class(&app, "rinch-select__label");
    press(&mut app, label);
    assert_eq!(focused(&app), Some(trigger));
    // Focus only: the list stays shut, as a combobox's label leaves it.
    let expanded = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .get(trigger)
            .and_then(|n| n.attributes.get("aria-expanded").cloned())
    };
    assert_eq!(expanded.as_deref(), Some("false"));
}
