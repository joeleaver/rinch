//! A `NumberInput` stepper click must not steal focus from the field (#1324,
//! found while reviewing #1323).
//!
//! The up/down stepper buttons are real `<button>`s with `tabindex="-1"` —
//! which only removes them from the Tab order, per CLAUDE.md's "Keyboard
//! Focus" section: "a mouse press claims the nearest focusable ancestor of
//! the hit node", tabindex or no. Before this fix, clicking a stepper while
//! the field held focus with an uncommitted keystroke blurred the field,
//! firing its `data-onchange` commit (issue #226) with the stale typed value
//! a beat before `apply_step` (#1323, for #512) fired a second `onchange`
//! with the freshly-stepped one — one click, two `onchange` calls. The fix
//! is `data-nofocus` (#312) on the controls container, the same mechanism an
//! editor toolbar and `Select`'s dropdown already use.

use super::*;
use rinch_components::NumberInput;
use rinch_core::Component;
use rinch_core::events::InputCallback;

/// Find the one node whose `class` attribute contains `class` exactly.
fn find_by_class(app: &RinchApp, class: &str) -> usize {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let matches: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == class))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one node carrying `{class}`, found {}",
        matches.len()
    );
    matches[0]
}

/// Mount a bare uncontrolled `NumberInput` (default_value 5, step 1, no
/// min/max) recording every `oninput`/`onchange` call. Returns the app, the
/// `<input>` node id, the up-stepper button id, and the log.
fn mount_fixture() -> (RinchApp, usize, usize, Rc<RefCell<Vec<String>>>) {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let record = |tag: &'static str, log: &Rc<RefCell<Vec<String>>>| {
        let log = log.clone();
        InputCallback::new(move |v: String| {
            log.borrow_mut().push(format!("{tag}:{v}"));
        })
    };
    let oninput = record("input", &log);
    let onchange = record("change", &log);

    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px; height: 200px");
        let number_input = NumberInput {
            default_value: Some(5.0),
            oninput: Some(oninput.clone()),
            onchange: Some(onchange.clone()),
            ..Default::default()
        };
        let rendered = number_input.render(scope, &[]);
        root.append_child(&rendered);
        root
    });
    app.mount_component(800.0, 600.0);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(800.0, 600.0);

    let input_id = find_by_class(&app, "rinch-number-input__input");
    let up_id = find_by_class(&app, "rinch-number-input__control--up");
    (app, input_id, up_id, log)
}

fn key(app: &mut RinchApp, text: &str) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyA,
            logical_key: None,
            text: Some(text.to_string()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
    app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        (800, 600),
        1.0,
    );
}

fn click_center(app: &mut RinchApp, id: usize) {
    let (cx, cy) = {
        let doc = app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (ax, ay, aw, ah) = painted_element_box(&d.tree, id);
        (ax + aw / 2.0, ay + ah / 2.0)
    };
    click(app, cx, cy);
}

/// Reproduces #1324 at HEAD: focus the field, type an uncommitted keystroke,
/// click the up stepper. Exactly one `onchange`, carrying the stepped value —
/// and focus never leaves the field.
#[test]
fn a_stepper_click_does_not_steal_focus_or_double_fire_onchange() {
    let (mut app, input_id, up_id, log) = mount_fixture();

    // Focus the field and leave an uncommitted keystroke live: "5" -> "51".
    click_center(&mut app, input_id);
    key(&mut app, "1");
    assert_eq!(
        *log.borrow(),
        vec!["input:51".to_string()],
        "typing is per-keystroke oninput only; no change while the gesture is live"
    );
    assert_eq!(
        app.focus_target,
        FocusTarget::Input(input_id),
        "the field holds focus after typing"
    );

    log.borrow_mut().clear();

    // Click the up stepper. A browser's native spinner never blurs the
    // field, so this must not either.
    click_center(&mut app, up_id);

    assert_eq!(
        app.focus_target,
        FocusTarget::Input(input_id),
        "a stepper click must not steal focus from the field (data-nofocus)"
    );
    assert_eq!(
        *log.borrow(),
        vec!["input:52".to_string(), "change:52".to_string()],
        "exactly one onchange, from apply_step alone — not a blur-commit of \
         the stale typed value (\"51\") followed by apply_step's own \
         onchange (\"52\")"
    );
}
