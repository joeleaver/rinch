//! The `Select` component's open list answers the keyboard, through the real
//! desktop key path (issue #434).
//!
//! `rinch-components/tests/select_keyboard_434.rs` pins the behaviour on the
//! mock by calling `dispatch_keyboard_event` directly. This file is the part
//! the mock cannot see: that a `PlatformEvent::KeyDown` reaches that function
//! at all while the trigger holds `FocusTarget::Node`, and that a consumed key
//! stops there — in particular that **Enter on the open list commits instead of
//! re-running the trigger's own Enter activation**, which would toggle the list
//! shut without picking and look, in a screenshot, exactly like a pick of the
//! option that was already selected. Every fixture starts from an option in
//! the middle of the list so that reading is impossible.

use super::*;
use std::cell::RefCell;

use rinch_components::select::{Select, SelectOption};
use rinch_core::{Component, InputCallback};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

struct Fixture {
    app: RinchApp,
    picks: Rc<RefCell<Vec<String>>>,
}

fn mount(value: &str) -> Fixture {
    let picks = Rc::new(RefCell::new(Vec::new()));
    let sink = picks.clone();
    let value = value.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 40px; width: 240px");
        let select = Select {
            value: value.clone(),
            data: vec![
                SelectOption::new("apple", "Apple"),
                SelectOption::new("banana", "Banana"),
                SelectOption::new("cherry", "Cherry"),
                SelectOption::new("date", "Date"),
            ],
            onchange: Some(InputCallback::new({
                let sink = sink.clone();
                move |v: String| sink.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&select);
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
    Fixture { app, picks }
}

fn find_all(app: &RinchApp, class: &str) -> Vec<usize> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let mut ids: Vec<usize> = d
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
    ids.sort_unstable();
    ids
}

fn attr(app: &RinchApp, node: usize, name: &str) -> Option<String> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .get(node)
        .and_then(|n| n.attributes.get(name).cloned())
}

impl Fixture {
    fn trigger(&self) -> usize {
        find_all(&self.app, "rinch-select__input")[0]
    }

    fn is_open(&self) -> bool {
        attr(&self.app, self.trigger(), "aria-expanded").as_deref() == Some("true")
    }

    fn highlighted(&self) -> Option<usize> {
        let options = find_all(&self.app, "rinch-select__option");
        let lit = find_all(&self.app, "rinch-select__option--highlighted");
        assert!(lit.len() <= 1, "more than one highlighted option: {lit:?}");
        lit.first()
            .map(|id| options.iter().position(|o| o == id).unwrap())
    }

    fn centre(&self, node: usize) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (x, y, w, h) = painted_element_box(&d.tree, node);
        assert!(w > 0.0 && h > 0.0, "node {node} has no box");
        (x + w / 2.0, y + h / 2.0)
    }

    fn tap(&mut self, node: usize) {
        let (x, y) = self.centre(node);
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
            self.app.handle_event(ev, (800, 600), 1.0);
        }
        self.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }

    fn key(&mut self, key: KeyCode, text: Option<&str>) {
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Fresh,
            },
            (800, 600),
            1.0,
        );
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers: Modifiers::default(),
            },
            (800, 600),
            1.0,
        );
        self.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }

    /// Open the list from the keyboard alone: Tab to the trigger, Enter.
    fn open_by_keyboard(&mut self) {
        self.key(KeyCode::Tab, None);
        assert_eq!(
            self.app.focus_target,
            FocusTarget::Node(self.trigger()),
            "precondition: Tab reaches the trigger"
        );
        self.key(KeyCode::Enter, None);
        assert!(self.is_open(), "precondition: Enter opened the list");
    }
}

#[test]
fn the_keyboard_alone_opens_moves_and_commits() {
    let mut f = mount("banana");
    f.open_by_keyboard();
    assert_eq!(
        f.highlighted(),
        Some(1),
        "the highlight starts at the selection"
    );

    f.key(KeyCode::ArrowDown, None);
    assert_eq!(f.highlighted(), Some(2));
    f.key(KeyCode::ArrowDown, None);
    f.key(KeyCode::ArrowUp, None);
    f.key(KeyCode::ArrowDown, None);
    assert_eq!(f.highlighted(), Some(3));

    f.key(KeyCode::Enter, None);
    assert!(!f.is_open(), "Enter closes the list");
    assert_eq!(
        *f.picks.borrow(),
        vec!["date".to_string()],
        "Enter commits the highlighted option — not the trigger's toggle"
    );
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(f.trigger()),
        "the trigger keeps the keyboard after a commit"
    );
}

#[test]
fn escape_closes_without_committing_and_keeps_the_trigger_focused() {
    let mut f = mount("banana");
    f.open_by_keyboard();
    f.key(KeyCode::ArrowDown, None);
    f.key(KeyCode::Escape, None);
    assert!(!f.is_open(), "Escape closes the list");
    assert!(f.picks.borrow().is_empty(), "and commits nothing");
    assert_eq!(f.app.focus_target, FocusTarget::Node(f.trigger()));

    // And the closed trigger's Enter opens it again: the keyboard is still
    // where it was, and nothing is left armed.
    f.key(KeyCode::Enter, None);
    assert!(f.is_open());
    assert_eq!(
        f.highlighted(),
        Some(1),
        "reopening starts at the selection"
    );
}

#[test]
fn home_end_and_type_ahead_reach_the_list() {
    let mut f = mount("banana");
    f.open_by_keyboard();
    f.key(KeyCode::End, None);
    assert_eq!(f.highlighted(), Some(3));
    f.key(KeyCode::Home, None);
    assert_eq!(f.highlighted(), Some(0));
    f.key(KeyCode::KeyC, Some("c"));
    assert_eq!(f.highlighted(), Some(2), "c → Cherry");
}

/// A pointer pick keeps the keyboard on the trigger (`data-nofocus` on the
/// list), so the next Enter reopens the list rather than going nowhere.
#[test]
fn a_tapped_option_leaves_the_trigger_holding_the_keyboard() {
    let mut f = mount("banana");
    let trigger = f.trigger();
    f.tap(trigger);
    assert!(f.is_open());
    assert_eq!(f.app.focus_target, FocusTarget::Node(trigger));

    let cherry = find_all(&f.app, "rinch-select__option")[2];
    f.tap(cherry);
    assert!(!f.is_open());
    assert_eq!(*f.picks.borrow(), vec!["cherry".to_string()]);
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Node(trigger),
        "a tap on an option must not take the keyboard off the trigger"
    );
    f.key(KeyCode::Enter, None);
    assert!(f.is_open(), "so Enter reopens it");
    assert_eq!(f.highlighted(), Some(2));
}

// ------------------------------------------------ review of PR #1165

/// "/" focuses a search field (an app interceptor) while the list is open.
/// The letters typed next are the field's; the list closes once the focus has
/// left it; Enter in the field commits nothing to the `Select`. At the PR's
/// first head the letters were eaten, the list stayed open, and Enter
/// committed `date`.
#[test]
fn a_field_focused_while_the_list_is_open_keeps_its_own_keys() {
    let picks = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = picks.clone();
    let typed = Rc::new(RefCell::new(Vec::<String>::new()));
    let input_id = rinch_core::register_input_handler(InputCallback::new({
        let t = typed.clone();
        move |v: String| t.borrow_mut().push(v)
    }));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 40px; width: 240px");
        let select = Select {
            value: "banana".into(),
            data: vec![
                SelectOption::new("apple", "Apple"),
                SelectOption::new("banana", "Banana"),
                SelectOption::new("cherry", "Cherry"),
                SelectOption::new("date", "Date"),
            ],
            onchange: Some(InputCallback::new({
                let s = sink.clone();
                move |v: String| s.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&select);
        let input = scope.create_element("input");
        input.set_attribute("class", "search");
        input.set_attribute("data-oninput", &input_id.0.to_string());
        root.append_child(&input);
        let inp = input.clone();
        rinch_core::set_keyboard_interceptor(move |k| {
            if k.is_down() && k.key == "/" {
                inp.focus();
                return true;
            }
            false
        });
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
    let mut f = Fixture {
        app,
        picks: picks.clone(),
    };
    let trigger = f.trigger();
    f.tap(trigger);
    assert!(f.is_open(), "precondition: open");

    f.key(KeyCode::Other, Some("/"));
    // The shell's wake after a handler: the focus request is applied then.
    f.app.handle_event(
        PlatformEvent::UserEvent(UserEvent::ReRender),
        (800, 600),
        1.0,
    );
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let input = find_all(&f.app, "search")[0];
    assert_eq!(
        f.app.focus_target,
        FocusTarget::Input(input),
        "precondition: the shortcut moved focus to the field"
    );
    assert!(!f.is_open(), "focus leaving the Select closes its list");

    f.key(KeyCode::KeyD, Some("d"));
    f.key(KeyCode::KeyA, Some("a"));
    assert_eq!(
        typed.borrow().last().map(String::as_str),
        Some("da"),
        "the letters reached the focused field"
    );
    f.key(KeyCode::Enter, None);
    assert!(
        picks.borrow().is_empty(),
        "Enter in the field commits nothing"
    );
}

/// The highlighted option is scrolled into view in a list taller than its
/// `max-height` (200px): End scrolls the list to its bottom.
#[test]
fn end_scrolls_the_highlighted_option_into_view() {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 40px; width: 240px");
        let select = Select {
            value: "o0".into(),
            data: (0..30)
                .map(|i| SelectOption::new(format!("o{i}"), format!("Option {i}")))
                .collect(),
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&select);
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
    let mut f = Fixture {
        app,
        picks: Rc::new(RefCell::new(Vec::new())),
    };
    f.open_by_keyboard();
    let list = find_all(&f.app, "rinch-select__dropdown")[0];
    let scroll = |f: &Fixture| {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree.get(list).unwrap().scroll_offset.1
    };
    assert_eq!(scroll(&f), 0.0, "precondition: the list opens at its top");
    f.key(KeyCode::End, None);
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(f.highlighted(), Some(29));
    assert!(
        scroll(&f) > 0.0,
        "End must scroll the list to show the last option"
    );
}
