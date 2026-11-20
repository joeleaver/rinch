//! A window's menu drawn in its document, with and without `native-menus`
//! (#2435): [`with_menu_bar_in_document`], which is the whole of the startup
//! path every Linux window takes, and every window of a build without the
//! native menus.
//!
//! These run in both builds and must answer the same, except where a test
//! says why not: with `native-menus` building the native bar registers the
//! chords, without them the wrapped component arms them as it mounts.

use super::with_menu_bar_in_document;
use crate::app::RinchApp;
use crate::menu::{Menu, MenuItem};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::WindowProps;
use rinch_platform::Modifiers;
use std::cell::Cell;
use std::rc::Rc;
use winit::keyboard::{Key, KeyCode};

const VP: (f32, f32) = (800.0, 600.0);

/// One counting item per chord.
struct Counter(Rc<Cell<u32>>);

impl Counter {
    fn new() -> Self {
        Self(Rc::new(Cell::new(0)))
    }

    fn item(&self, label: &str, chord: &str) -> MenuItem {
        let fired = self.0.clone();
        MenuItem::new(label)
            .shortcut(chord)
            .on_click(move || fired.set(fired.get() + 1))
    }

    fn get(&self) -> u32 {
        self.0.get()
    }
}

/// The app's own content: a `<main>` holding one line of text.
fn content(scope: &mut RenderScope) -> NodeHandle {
    let main = scope.create_element("main");
    main.append_child(&scope.create_text("the app"));
    main
}

/// Mount `component` the way the shell mounts a window's.
fn mount(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let mut app = RinchApp::new(component);
    app.mount_component(VP.0, VP.1);
    app.resolve_and_repaint(VP.0, VP.1);
    app
}

/// `menus` set on a window with `props`, mounted.
fn window_with(menus: Vec<(&str, Menu)>, props: &WindowProps) -> (RinchApp, bool) {
    let owned = menus
        .into_iter()
        .map(|(label, menu)| (label.to_string(), menu))
        .collect();
    let (wrapped, native_menu) = with_menu_bar_in_document(content, props, owned);
    (mount(wrapped), native_menu.is_some())
}

/// Ctrl + `letter` as the shell sends it to the chord registry first, on an
/// app whose focus is on nothing. Answers whether a chord ran.
fn ctrl(app: &RinchApp, key: KeyCode, letter: &str) -> bool {
    let mods = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    app.try_menu_shortcut(mods, key, &Key::Character(letter.into()), None)
}

/// The ids of the element nodes whose `class` holds `class`, in id order.
fn with_class(app: &RinchApp, class: &str) -> Vec<u64> {
    let d = app.doc.as_ref().expect("mounted").borrow();
    let mut found: Vec<u64> = d
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
    found.sort_unstable();
    found
}

/// The text of `node`'s text children, joined.
fn text_of(app: &RinchApp, node: u64) -> String {
    let d = app.doc.as_ref().expect("mounted").borrow();
    d.tree
        .get(node)
        .map(|n| {
            n.children
                .iter()
                .filter_map(|c| d.tree.get(*c).and_then(|c| c.text_content()))
                .collect()
        })
        .unwrap_or_default()
}

/// Two menus of two items each; the one each test presses is the second
/// item of the second menu.
fn two_menus(
    new: &Counter,
    open: &Counter,
    undo: &Counter,
    find: &Counter,
) -> Vec<(&'static str, Menu)> {
    vec![
        (
            "File",
            Menu::new()
                .item(new.item("New", "Ctrl+N"))
                .item(open.item("Open", "Ctrl+O")),
        ),
        (
            "Edit",
            Menu::new()
                .item(undo.item("Undo", "Ctrl+Z"))
                .separator()
                .item(find.item("Find", "Ctrl+F")),
        ),
    ]
}

/// A chord fires its own item's callback and no other, in either build.
/// Without `native-menus` nothing but the wrapped component arms it.
#[test]
fn a_menu_chord_fires_its_own_item() {
    let (new, open, undo, find) = (
        Counter::new(),
        Counter::new(),
        Counter::new(),
        Counter::new(),
    );
    let (app, has_native) = window_with(
        two_menus(&new, &open, &undo, &find),
        &WindowProps::default(),
    );
    assert_eq!(
        has_native,
        cfg!(feature = "native-menus"),
        "a native bar only with native-menus"
    );

    assert!(
        ctrl(&app, KeyCode::KeyF, "f"),
        "Ctrl+F ran no menu callback"
    );
    assert_eq!(
        [new.get(), open.get(), undo.get(), find.get()],
        [0, 0, 0, 1],
        "Ctrl+F reached an item other than Edit > Find, or none"
    );
    assert!(
        ctrl(&app, KeyCode::KeyO, "o"),
        "Ctrl+O ran no menu callback"
    );
    assert_eq!(
        [new.get(), open.get(), undo.get(), find.get()],
        [0, 1, 0, 1]
    );
    assert!(
        !ctrl(&app, KeyCode::KeyQ, "q"),
        "a chord no item declares ran something"
    );
}

/// A borderless window draws its bar through `BorderlessWindow`, not around
/// the content, and its chords are armed all the same.
#[test]
fn a_borderless_windows_menu_chord_fires_its_own_item() {
    let (new, open, undo, find) = (
        Counter::new(),
        Counter::new(),
        Counter::new(),
        Counter::new(),
    );
    let props = WindowProps {
        borderless: true,
        ..WindowProps::default()
    };
    let (app, _) = window_with(two_menus(&new, &open, &undo, &find), &props);
    assert!(
        ctrl(&app, KeyCode::KeyF, "f"),
        "Ctrl+F ran no menu callback"
    );
    assert_eq!(
        [new.get(), open.get(), undo.get(), find.get()],
        [0, 0, 0, 1]
    );
}

/// The window's document holds the DOM bar, its menus' labels in order, and
/// the app's content after it, in either build.
#[test]
fn the_window_wears_the_document_menu_bar() {
    let (new, open, undo, find) = (
        Counter::new(),
        Counter::new(),
        Counter::new(),
        Counter::new(),
    );
    let (app, _) = window_with(
        two_menus(&new, &open, &undo, &find),
        &WindowProps::default(),
    );

    let wrappers = with_class(&app, "rinch-app-menu-bar-wrapper");
    assert_eq!(wrappers.len(), 1, "no DOM menu bar around the content");
    let labels: Vec<String> = with_class(&app, "rinch-app-menu-item__label")
        .into_iter()
        .map(|id| text_of(&app, id))
        .collect();
    assert_eq!(labels, ["File", "Edit"], "the bar's menus");

    let d = app.doc.as_ref().expect("mounted").borrow();
    let main = d
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.tag() == Some("main"))
        .map(|(id, _)| id)
        .expect("the app's content is mounted");
    let mut up = d.tree.get(main).and_then(|n| n.parent);
    while let Some(id) = up {
        if id == wrappers[0] {
            break;
        }
        up = d.tree.get(id).and_then(|n| n.parent);
    }
    assert_eq!(
        up,
        Some(wrappers[0]),
        "the content is not inside the bar's wrapper"
    );
}

/// Two windows' menus on one thread: the registry is one per thread, and the
/// bar armed last is the one a chord reaches. The first window's teardown does
/// not take the second one's chords with it.
#[test]
fn of_two_windows_on_one_thread_the_later_menu_receives_the_chord() {
    let first_new = Counter::new();
    let (first, _) = window_with(
        vec![("File", Menu::new().item(first_new.item("New", "Ctrl+N")))],
        &WindowProps::default(),
    );
    let (second_new, second_find) = (Counter::new(), Counter::new());
    let (second, _) = window_with(
        vec![
            ("File", Menu::new().item(second_new.item("New", "Ctrl+N"))),
            ("Edit", Menu::new().item(second_find.item("Find", "Ctrl+F"))),
        ],
        &WindowProps::default(),
    );
    let presser = mount(content);

    assert!(ctrl(&presser, KeyCode::KeyN, "n"));
    assert_eq!(
        (first_new.get(), second_new.get()),
        (0, 1),
        "Ctrl+N went to the first window's menu, or to both"
    );

    drop(first);
    assert!(
        ctrl(&presser, KeyCode::KeyN, "n"),
        "the first window's teardown released the second window's chords"
    );
    assert_eq!((first_new.get(), second_new.get()), (0, 2));
    assert!(ctrl(&presser, KeyCode::KeyF, "f"));
    assert_eq!(second_find.get(), 1);

    drop(second);
    // With `native-menus` the chords belong to the native bar, which has the
    // app's lifetime and is only ever replaced; without them they belong to
    // the window whose root scope armed them, and leave with it.
    assert_eq!(
        ctrl(&presser, KeyCode::KeyN, "n"),
        cfg!(feature = "native-menus"),
        "whether a chord outlives its window's document"
    );
}
