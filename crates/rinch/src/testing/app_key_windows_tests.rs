//! `on_app_key` across windows (#3062 round 3, #3082): a release reaches
//! exactly the listeners that heard its press, whichever window it arrives
//! in — a listener registered from `main`, and the window the key went down
//! in — and no listener of a window that never saw the key go down.
//!
//! Each window registers a listener while its document renders, so it hears
//! that document's presses; the test body registers one outside any document,
//! as `main` does, which hears every window's.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{LayerSurface, Placement, WindowProps};
use rinch_core::{AppKeyListener, on_app_key};

use crate::WindowHandle;
use crate::popup::{PopupAnchor, PopupProps, open_popup};
use crate::testing::TestApp;

type Log = Rc<RefCell<Vec<String>>>;
type Slot = Rc<RefCell<Option<NodeHandle>>>;

/// A listener logging `name:down:key` / `name:up:key`, consuming nothing.
fn listener(log: &Log, name: &'static str) -> AppKeyListener {
    let l = log.clone();
    on_app_key(move |k| {
        let phase = if k.is_up() { "up" } else { "down" };
        l.borrow_mut().push(format!("{name}:{phase}:{}", k.key));
        false
    })
}

/// A window's content: a 20 px row (kept in `row`, to anchor a popup to)
/// in a box, and a listener named `name` registered as the document renders.
fn content(
    log: &Log,
    name: &'static str,
    row: Slot,
) -> impl FnOnce(&mut RenderScope) -> NodeHandle + 'static {
    let log = log.clone();
    move |s: &mut RenderScope| {
        listener(&log, name);
        let root = s.create_element("div");
        root.set_attribute("style", "width: 200px; height: 100px; margin: 0");
        let r = s.create_element("div");
        r.set_attribute("style", "height: 20px");
        root.append_child(&r);
        *row.borrow_mut() = Some(r);
        root
    }
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// The main window (listener `a`), a second window (listener `b`), and a
/// listener registered from `main` (`app`).
struct Two {
    app: TestApp,
    b: WindowHandle,
    log: Log,
    _main: AppKeyListener,
}

fn two_windows() -> Two {
    rinch_core::clear_keyboard_interceptor();
    let log: Log = Rc::default();
    let mut app = TestApp::mount(content(&log, "a", Rc::default()));
    let b = crate::open_window(WindowProps::default(), content(&log, "b", Rc::default()));
    app.settle();
    app.window_focus(true);
    let main = listener(&log, "app");
    take(&log);
    Two {
        app,
        b,
        log,
        _main: main,
    }
}

/// W held in the main window, which then loses the keyboard to the second
/// window, where W is let go: the listener registered from `main` and the
/// main window's own both hear the release. Before, the main window's blur
/// forgot the press and the release reached neither, so W stayed down for
/// good for both (macOS sends no focus-in press to recover with).
#[test]
fn a_key_held_into_another_window_releases_to_the_listeners_that_heard_it() {
    let Two {
        mut app, b, log, ..
    } = two_windows();
    app.key_down("w");
    assert_eq!(take(&log), ["a:down:w", "app:down:w"]);
    app.window_focus(false);
    app.window(b).key_up("w");
    assert_eq!(take(&log), ["a:up:w", "app:up:w"]);
}

/// The race: the second window's focus-in and a press there arrive before
/// the main window's blur. Nothing a blur does undoes that press, so its
/// release is heard. Before, the blur erased it.
#[test]
fn a_press_in_the_new_window_before_the_old_ones_blur_keeps_its_release() {
    let Two {
        mut app, b, log, ..
    } = two_windows();
    app.window(b).key_down("q");
    assert_eq!(take(&log), ["b:down:q", "app:down:q"]);
    app.window_focus(false);
    app.window(b).key_up("q");
    assert_eq!(take(&log), ["b:up:q", "app:up:q"]);
}

/// #3082: a key pressed in the main window and released in the second, with
/// no blur between (focus moved before the blur came, or a window that takes
/// the keyboard without one): the second window's listener never heard the
/// press and does not hear the release; the main window's does. Before, the
/// thread-wide record handed the release to the second window's listener
/// and not to the main window's.
#[test]
fn a_release_never_reaches_a_listener_that_did_not_hear_its_press() {
    let Two {
        mut app, b, log, ..
    } = two_windows();
    app.key_down("s");
    assert_eq!(take(&log), ["a:down:s", "app:down:s"]);
    app.window(b).key_up("s");
    assert_eq!(take(&log), ["a:up:s", "app:up:s"]);
}

/// The menu window variant (#2552): a key held in a bar when one of its menus
/// opens is released while the menu holds the keyboard, so the release is
/// dispatched in the menu's document. The bar's listener and `main`'s hear
/// it; the menu's, which never saw the key go down, does not. Before, the
/// menu's listener heard it and the bar's never did.
#[test]
fn a_key_held_when_a_menu_opens_releases_to_the_bars_listeners_not_the_menus() {
    rinch_core::clear_keyboard_interceptor();
    let log: Log = Rc::default();
    let mut app = TestApp::mount(|s: &mut RenderScope| s.create_element("div"));
    let button: Slot = Rc::default();
    let bar = crate::open_window(
        WindowProps {
            width: 800,
            height: 40,
            wayland_layer: Some(LayerSurface::default()),
            ..WindowProps::default()
        },
        content(&log, "bar", button.clone()),
    );
    app.settle();
    let _main = listener(&log, "app");
    take(&log);

    app.window(bar).key_down("w");
    assert_eq!(take(&log), ["bar:down:w", "app:down:w"]);
    // The press a menu's grab names.
    app.window(bar).click(5.0, 5.0);
    let menu = open_popup(
        PopupProps {
            anchor: PopupAnchor::Node(button.borrow().clone().expect("the bar is mounted")),
            placement: Placement::TOP_START,
            ..PopupProps::default()
        },
        content(&log, "menu", Rc::default()),
    )
    .expect("a menu after a press in its bar");
    app.settle();
    assert_eq!(app.windows(), vec![bar, menu]);
    take(&log);

    // Typed where the keyboard is (the bar): the topmost menu takes it.
    app.window(bar).key_up("w");
    assert_eq!(take(&log), ["bar:up:w", "app:up:w"]);
    // A key pressed now is the menu's, and so is its release (`app` was
    // registered before the menu's listener, so it hears first).
    app.window(bar).key_down("x");
    app.window(bar).key_up("x");
    assert_eq!(
        take(&log),
        ["app:down:x", "menu:down:x", "app:up:x", "menu:up:x"]
    );
}
