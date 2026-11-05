//! A `Tree` with `keyboard`, driven through the real key path: a click
//! gives a row the keyboard, the arrows and type-ahead move the cursor (and
//! the selection, and tell the app once it rests), the app sees every key
//! first and turns one into a structural move, and keys typed while focus
//! is elsewhere are not the tree's.

use super::*;
use std::cell::RefCell;

use rinch_components::{
    Tree, TreeDrop, TreeKey, TreeMove, TreeNodeData, TreeSlot, UseTreeOptions, UseTreeReturn,
};
use rinch_core::Component;
use rinch_core::element::ValueCallback;
use rinch_platform::{KeyCode, KeyRepeat, Modifiers};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

struct Fixture {
    app: RinchApp,
    state: UseTreeReturn,
    rested: Rc<RefCell<Vec<String>>>,
    moves: Rc<RefCell<Vec<TreeDrop>>>,
    seen: Rc<RefCell<Vec<String>>>,
}

/// a (open: a1, a2), b, c; and a box outside the tree to click away to.
/// The app takes F2 and turns Alt+Shift+ArrowUp into a move up.
fn fixture() -> Fixture {
    let rested: Rc<RefCell<Vec<String>>> = Rc::default();
    let moves: Rc<RefCell<Vec<TreeDrop>>> = Rc::default();
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: ["a".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let (r, m, s) = (rested.clone(), moves.clone(), seen.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (r, m, s) = (r.clone(), m.clone(), s.clone());
        let tree = Tree {
            data: vec![
                TreeNodeData::new("a", "Apples").with_children(vec![
                    TreeNodeData::new("a1", "Avocado"),
                    TreeNodeData::new("a2", "Banana"),
                ]),
                TreeNodeData::new("b", "Beets"),
                TreeNodeData::new("c", "Carrots"),
            ],
            tree: Some(state),
            select_on_click: true,
            keyboard: true,
            cursor_delay_ms: 0,
            oncursor: Some(ValueCallback::new(move |v| r.borrow_mut().push(v))),
            handle_key: Some(Rc::new(move |key: &TreeKey| {
                s.borrow_mut().push(key.key.clone());
                if key.key == "F2" {
                    return true;
                }
                if key.alt && key.shift && key.key == "ArrowUp" {
                    if let Some(drop) = key.drop_for(TreeMove::Up) {
                        m.borrow_mut().push(drop);
                    }
                    return true;
                }
                false
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
        let away = scope.create_element("div");
        away.set_attribute("tabindex", "0");
        away.set_attribute("class", "away");
        away.set_attribute("style", "height: 40px; margin-top: 20px");
        root.append_child(&away);
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
    Fixture {
        app,
        state,
        rested,
        moves,
        seen,
    }
}

fn has_class(n: &rinch_dom::Node, class: &str) -> bool {
    n.attributes
        .get("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn find(app: &RinchApp, pred: impl Fn(&rinch_dom::Node) -> bool) -> u64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| pred(n))
        .map(|(id, _)| id)
        .expect("the node")
}

/// The content box (the row) of `value`.
fn row(app: &RinchApp, value: &str) -> u64 {
    let li = find(app, |n| {
        n.attributes.get("data-value").map(String::as_str) == Some(value)
    });
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[li]
        .children
        .iter()
        .copied()
        .find(|&c| has_class(&d.tree.nodes[c], "rinch-tree__node-content"))
        .unwrap()
}

fn centre(app: &RinchApp, id: u64) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (x, y, w, h) = hit_testing::painted_element_box(&d.tree, id);
    (x + w / 2.0, y + h / 2.0)
}

fn click(f: &mut Fixture, (x, y): (f32, f32)) {
    for event in [
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
        f.app.handle_event(event, (800, 600), 1.0);
    }
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn press(f: &mut Fixture, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    f.app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        (800, 600),
        1.0,
    );
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

fn cursor_row_classes(f: &Fixture, value: &str) -> String {
    let id = row(&f.app, value);
    let doc = f.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[id]
        .attributes
        .get("class")
        .cloned()
        .unwrap_or_default()
}

#[test]
fn a_clicked_row_takes_the_keyboard_and_the_arrows_move_the_cursor() {
    let mut f = fixture();
    let a1 = centre(&f.app, row(&f.app, "a1"));
    click(&mut f, a1);
    assert_eq!(f.state.cursor.get().as_deref(), Some("a1"));

    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("a2"));
    assert!(
        f.state.selected.get().contains("a2"),
        "the cursor row is selected"
    );
    assert_eq!(
        f.rested.borrow().last().map(String::as_str),
        Some("a2"),
        "the app is told where it rests"
    );
    assert!(cursor_row_classes(&f, "a2").contains("rinch-tree__node-content--cursor"));
    assert!(!cursor_row_classes(&f, "a1").contains("rinch-tree__node-content--cursor"));

    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("b"));
    press(&mut f, KeyCode::ArrowLeft, None, Modifiers::default());
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("b"),
        "a top-level leaf has no parent to go to"
    );
    press(&mut f, KeyCode::Home, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("a"));
    press(&mut f, KeyCode::ArrowLeft, None, Modifiers::default());
    assert!(
        !f.state.expanded.get().contains("a"),
        "Left on an open row closes it"
    );
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("b"),
        "a closed row's children are skipped"
    );

    // Type-ahead: "c" finds Carrots.
    press(&mut f, KeyCode::KeyC, Some("c"), Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("c"));
}

#[test]
fn the_app_sees_keys_first_and_moves_rows_with_them() {
    let mut f = fixture();
    let b = centre(&f.app, row(&f.app, "b"));
    click(&mut f, b);
    press(&mut f, KeyCode::F2, None, Modifiers::default());
    assert_eq!(f.seen.borrow().last().map(String::as_str), Some("F2"));
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("b"),
        "a consumed key does nothing else"
    );

    let alt_shift = Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    press(&mut f, KeyCode::ArrowUp, None, alt_shift);
    let moves = f.moves.borrow();
    assert_eq!(moves.len(), 1, "Alt+Shift+Up became a move");
    assert_eq!(
        (
            moves[0].dragged.clone(),
            moves[0].parent.clone(),
            moves[0].slot.clone()
        ),
        (vec!["b".to_string()], None, TreeSlot::First)
    );
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("b"),
        "and not a cursor move"
    );
}

type Log<T> = Rc<RefCell<Vec<T>>>;

/// A tree with `multiple` selection: a, b, c, d at the top level.
fn multi_fixture() -> (Fixture, Log<Vec<String>>, Log<String>) {
    let changes: Log<Vec<String>> = Rc::default();
    let opened: Log<String> = Rc::default();
    let state = UseTreeReturn::new(UseTreeOptions {
        multiple: true,
        ..Default::default()
    });
    let (c, o) = (changes.clone(), opened.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (c, o) = (c.clone(), o.clone());
        let tree = Tree {
            data: ["a", "b", "c", "d"]
                .iter()
                .map(|v| TreeNodeData::new(*v, v.to_uppercase()))
                .collect(),
            tree: Some(state),
            select_on_click: true,
            keyboard: true,
            draggable: true,
            cursor_delay_ms: 0,
            onselect: Some(ValueCallback::new(move |v| o.borrow_mut().push(v))),
            onselectionchange: Some(ValueCallback::new(move |values| {
                c.borrow_mut().push(values)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 300px");
        root.append_child(&tree);
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
    (
        Fixture {
            app,
            state,
            rested: Rc::default(),
            moves: Rc::default(),
            seen: Rc::default(),
        },
        changes,
        opened,
    )
}

fn click_with(f: &mut Fixture, at: (f32, f32), modifiers: Modifiers) {
    f.app
        .handle_event(PlatformEvent::ModifiersChanged(modifiers), (800, 600), 1.0);
    click(f, at);
    f.app.handle_event(
        PlatformEvent::ModifiersChanged(Modifiers::default()),
        (800, 600),
        1.0,
    );
}

fn selection(f: &Fixture) -> Vec<String> {
    let mut s: Vec<String> = f.state.selected.get().into_iter().collect();
    s.sort();
    s
}

#[test]
fn ctrl_and_shift_clicks_select_several_rows_and_open_nothing() {
    let (mut f, changes, opened) = multi_fixture();
    let a = centre(&f.app, row(&f.app, "a"));
    let c = centre(&f.app, row(&f.app, "c"));
    let d = centre(&f.app, row(&f.app, "d"));
    click(&mut f, a);
    assert_eq!(
        *opened.borrow(),
        vec!["a".to_string()],
        "a plain click selects and opens"
    );

    click_with(
        &mut f,
        c,
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["a", "c"]);
    assert_eq!(opened.borrow().len(), 1, "a Ctrl-click opens nothing");
    assert_eq!(
        changes.borrow().last().cloned(),
        Some(vec!["a".to_string(), "c".to_string()]),
        "top to bottom"
    );

    click_with(
        &mut f,
        a,
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["c"], "Ctrl-click again takes a row out");

    // The anchor is a now (the last Ctrl-click); Shift-click d runs a..d.
    click_with(
        &mut f,
        d,
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["a", "b", "c", "d"]);
    assert_eq!(opened.borrow().len(), 1, "a Shift-click opens nothing");

    // Shift+Up from the cursor (d) shortens the run from the anchor.
    press(
        &mut f,
        KeyCode::ArrowUp,
        None,
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["a", "b", "c"]);
    // A plain arrow moves the cursor and selects nothing.
    press(&mut f, KeyCode::ArrowUp, None, Modifiers::default());
    assert_eq!(f.state.cursor.get().as_deref(), Some("b"));
    assert_eq!(selection(&f), ["a", "b", "c"]);
    // Space toggles the cursor row and anchors the next run there.
    press(&mut f, KeyCode::Space, Some(" "), Modifiers::default());
    assert_eq!(selection(&f), ["a", "c"]);
    assert_eq!(
        changes.borrow().last().cloned(),
        Some(vec!["a".to_string(), "c".to_string()])
    );
    press(
        &mut f,
        KeyCode::ArrowDown,
        None,
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["b", "c"], "a run from b, where Space was");
    // Ctrl+A: the cursor row's siblings.
    press(
        &mut f,
        KeyCode::KeyA,
        Some("a"),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(selection(&f), ["a", "b", "c", "d"]);
}

#[test]
fn keys_typed_elsewhere_are_not_the_trees() {
    let mut f = fixture();
    let a1 = centre(&f.app, row(&f.app, "a1"));
    click(&mut f, a1);
    let away = find(&f.app, |n| has_class(n, "away"));
    let away = centre(&f.app, away);
    click(&mut f, away);
    let seen_before = f.seen.borrow().len();
    press(&mut f, KeyCode::ArrowDown, None, Modifiers::default());
    assert_eq!(
        f.state.cursor.get().as_deref(),
        Some("a1"),
        "the cursor stays"
    );
    assert_eq!(
        f.seen.borrow().len(),
        seen_before,
        "the app's handle_key was not offered it either"
    );
}
