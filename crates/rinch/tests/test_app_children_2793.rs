//! `TestApp` driving the windows a component opens (#2793), through the
//! public harness only: opening after the turn that built the element,
//! the first facts in the shell's order, a child's own requests, input and
//! reading in a child's document, the configures a compositor sends, and
//! both ways of closing.

#![cfg(all(feature = "testing", feature = "desktop"))]

use rinch::prelude::*;
use rinch::testing::{TestApp, WindowRequest};
use rinch::window::{scale_factor, window_physical_size, window_size};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Reads = Rc<RefCell<Vec<((f64, f64), (u32, u32), f64)>>>;

/// A child's content whose mount makes an effect logging its window's
/// facts into `reads`.
fn reading(__scope: &mut RenderScope, reads: Reads, id: &'static str) -> NodeHandle {
    rinch::Effect::new(move || {
        reads
            .borrow_mut()
            .push((window_size(), window_physical_size(), scale_factor()));
    });
    rsx! { p { id: id, "child" } }
}

/// (a) A root with two `Window {}` children: each one's opening props,
/// and a query and a click in each one's own document.
#[test]
fn two_children_each_have_their_props_and_their_own_document() {
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let c = clicks.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (a, b) = (c.clone(), c.clone());
        rsx! {
            div {
                Window { title: "Left".to_owned(), width: 300, height: 200,
                    button { id: "go", onclick: move || a.borrow_mut().push("left"), "Left" }
                }
                Window { title: "Right".to_owned(), width: 320, height: 240,
                    button { id: "go", onclick: move || b.borrow_mut().push("right"), "Right" }
                }
            }
        }
    });
    let windows = app.windows();
    assert_eq!(windows.len(), 2);
    let mut left = app.window(windows[0]);
    assert_eq!(
        (left.props().title.as_str(), left.props().width),
        ("Left", 300)
    );
    assert_eq!(left.text("#go"), "Left");
    left.click_on("#go");
    let mut right = app.window(windows[1]);
    assert_eq!(right.props().title, "Right");
    assert_eq!(right.size(), (320, 240));
    right.click_on("#go");
    assert_eq!(*clicks.borrow(), ["left", "right"]);
    assert!(
        app.query("#go").is_none(),
        "neither is in the main document"
    );
}

/// A child's content that counts its mounts.
fn counted(__scope: &mut RenderScope, mounted: Rc<Cell<u32>>) -> NodeHandle {
    mounted.set(mounted.get() + 1);
    rsx! { p { "inside" } }
}

/// A child opens after the turn that built its element, never inside it.
#[test]
fn a_child_opens_after_the_turn_that_built_it() {
    let show = Rc::new(Cell::new(None::<Signal<bool>>));
    let mounted = Rc::new(Cell::new(0));
    let (s, m) = (show.clone(), mounted.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let open = Signal::new(false);
        s.set(Some(open));
        let m = m.clone();
        rsx! {
            div {
                if open.get() {
                    Window { title: "Later".to_owned(),
                        {counted(__scope, m.clone())}
                    }
                }
            }
        }
    });
    let open = show.get().unwrap();
    let seen_in_turn = Rc::new(Cell::new(None));
    let seen = seen_in_turn.clone();
    let m = mounted.clone();
    // Inside the turn that builds the element: not mounted yet.
    rinch::Effect::new(move || {
        if open.get() {
            seen.set(Some(m.get()));
        }
    });
    open.set(true);
    assert_eq!(seen_in_turn.get(), Some(0), "not inside the building turn");
    assert!(
        app.windows().is_empty(),
        "not before the requests are served"
    );
    app.settle();
    assert_eq!(mounted.get(), 1);
    assert_eq!(app.windows().len(), 1);
}

/// A child's content whose mount asks its window for a size twice: through
/// its handle, then from its document, with the height `height` holds.
fn asking(__scope: &mut RenderScope, height: Signal<u32>) -> NodeHandle {
    let h = f64::from(rinch::untracked(|| height.get()));
    let me = WindowHandle::current().expect("the child's own handle");
    me.set_size(1280.0, h);
    rinch::windows::set_current_window_size(1280.0, h + 1.0);
    rsx! { p { "bar" } }
}

/// (b) A signal written in the turn that builds a `Window {}` element is
/// what the child's mount sees on the next pump, and the requests its mount
/// makes, through its handle and from its document, are its own.
#[test]
fn a_childs_mount_sees_the_newest_value_and_its_requests_are_its_own() {
    let state = Rc::new(Cell::new(None::<(Signal<bool>, Signal<u32>)>));
    let s = state.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let open = Signal::new(false);
        let height = Signal::new(30_u32);
        s.set(Some((open, height)));
        rsx! {
            div {
                if open.get() {
                    Window { title: "Bar".to_owned(), width: 1280, height: 30,
                        wayland_layer: Some(LayerSurface::default()),
                        {asking(__scope, height)}
                    }
                    Window { title: "Other".to_owned(), p { "other" } }
                }
            }
        }
    });
    let (open, height) = state.get().unwrap();
    // One turn: the element is built and the value changes in it.
    rinch::batch(|| {
        open.set(true);
        height.set(40);
    });
    app.settle();
    let windows = app.windows();
    assert_eq!(windows.len(), 2);
    let bar = app.window(windows[0]).take_requests();
    assert_eq!(
        bar,
        [
            WindowRequest::SetSize(1280.0, 40.0),
            WindowRequest::SetSize(1280.0, 41.0)
        ],
        "both, in the order made, in one take"
    );
    assert!(app.window(windows[1]).take_requests().is_empty());
    assert!(
        !app.take_window_requests().iter().any(|r| matches!(
            r,
            WindowRequest::OtherWindow { .. } | WindowRequest::SetSize(..)
        )),
        "nothing of the child's in the app's list"
    );
}

/// A child's content whose mount makes an effect that notes the current
/// window each time `tick` changes.
fn noting_current(
    __scope: &mut RenderScope,
    seen: Rc<RefCell<Vec<Option<WindowHandle>>>>,
    tick: Signal<u32>,
) -> NodeHandle {
    rinch::Effect::new(move || {
        let _ = tick.get();
        seen.borrow_mut().push(WindowHandle::current());
    });
    rsx! { p { "child" } }
}

/// `WindowHandle::current()` answers the child inside its mount and inside
/// a later run of an effect made there.
#[test]
fn the_current_window_is_the_child_in_its_mount_and_its_effects() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let tick = Rc::new(Cell::new(None::<Signal<u32>>));
    let (s, t) = (seen.clone(), tick.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let tick = Signal::new(0_u32);
        t.set(Some(tick));
        let s = s.clone();
        rsx! {
            div {
                Window { title: "Child".to_owned(),
                    {noting_current(__scope, s.clone(), tick)}
                }
            }
        }
    });
    let child = app.windows()[0];
    tick.get().unwrap().set(1);
    app.settle();
    assert_eq!(*seen.borrow(), [Some(child), Some(child)]);
}

/// (c) A child's first facts in the shell's order: no surface at its mount,
/// then the configure `create_window` gives it; then a new scale and a
/// resize, which a reader made in its mount follows, with its own values,
/// not the main window's.
#[test]
fn a_childs_readers_follow_its_first_facts_then_its_configures() {
    let reads: Reads = Rc::new(RefCell::new(Vec::new()));
    let r = reads.clone();
    let mut app = TestApp::builder(move |__scope: &mut RenderScope| {
        let r = r.clone();
        rsx! { div { Window { title: "Child".to_owned(), width: 301, height: 201,
            {reading(__scope, r.clone(), "c")}
        } } }
    })
    .size(1000, 700)
    .mount();
    assert_eq!(
        *reads.borrow(),
        [
            ((301.0, 201.0), (0, 0), 1.0),
            ((301.0, 201.0), (301, 201), 1.0)
        ],
        "(0, 0) at the mount, then the first configure"
    );
    let child = app.windows()[0];
    let mut window = app.window(child);
    window.set_scale(2.0);
    assert_eq!(
        reads.borrow().last(),
        Some(&((301.0, 201.0), (602, 402), 2.0))
    );
    window.resize(400, 250);
    assert_eq!(
        reads.borrow().last(),
        Some(&((400.0, 250.0), (800, 500), 2.0))
    );
    assert_eq!(
        window_size(),
        (1000.0, 700.0),
        "the main window's are its own"
    );
}

/// `set_surface_size` is the surface of the next configure only, published
/// with it in one batch; the configure after that has the stand-in again.
#[test]
fn a_surface_set_by_the_test_is_the_next_configures_only() {
    let reads: Reads = Rc::new(RefCell::new(Vec::new()));
    let r = reads.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let r = r.clone();
        rsx! { div { Window { title: "Child".to_owned(), width: 300, height: 200,
            {reading(__scope, r.clone(), "c")}
        } } }
    });
    let child = app.windows()[0];
    let mut window = app.window(child);
    window.set_scale(1.25);
    let before = reads.borrow().len();
    window.set_surface_size(1001, 751);
    assert_eq!(
        reads.borrow().len(),
        before,
        "it publishes nothing by itself"
    );
    window.resize(801, 601);
    assert_eq!(reads.borrow().len(), before + 1, "one run for the pair");
    assert_eq!(
        reads.borrow().last(),
        Some(&((801.0, 601.0), (1001, 751), 1.25))
    );
    window.resize(400, 300);
    assert_eq!(
        reads.borrow().last(),
        Some(&((400.0, 300.0), (500, 375), 1.25)),
        "the stand-in again"
    );
}

/// A child's content whose mount asks its window for `width` × `height`.
fn sized(__scope: &mut RenderScope, width: f64, height: f64) -> NodeHandle {
    WindowHandle::current().unwrap().set_size(width, height);
    rsx! { p { "sized" } }
}

/// A toplevel's size request is applied as the shell applies the size winit
/// grants; a layer surface's is recorded only. Each handle says its kind
/// and its size.
#[test]
fn a_toplevels_size_request_is_applied_and_a_layer_surfaces_recorded() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                Window { title: "Top".to_owned(), width: 300, height: 200,
                    {sized(__scope, 350.0, 220.0)}
                }
                Window { title: "Layer".to_owned(), width: 1280, height: 30,
                    wayland_layer: Some(LayerSurface::default()),
                    {sized(__scope, 1280.0, 40.0)}
                }
            }
        }
    });
    let windows = app.windows();
    let (top, layer) = (windows[0], windows[1]);
    assert_eq!(top.surface_kind(), Some(WindowSurfaceKind::Toplevel));
    assert_eq!(layer.surface_kind(), Some(WindowSurfaceKind::LayerShell));
    assert_eq!(app.window(top).size(), (350, 220), "applied");
    assert_eq!(top.size(), Some((350.0, 220.0)));
    assert_eq!(app.window(layer).size(), (1280, 30), "recorded only");
    assert_eq!(
        app.window(layer).take_requests(),
        [WindowRequest::SetSize(1280.0, 40.0)]
    );
    app.window(layer).resize(1280, 40);
    assert_eq!(layer.size(), Some((1280.0, 40.0)));
    assert_eq!(layer.position(), None);
}

/// (d) Closing: by its handle (its `on_closed` runs), and by its owner's
/// scope going (it does not). Either way it is no longer listed.
#[test]
fn a_child_closes_by_its_handle_and_with_its_owner() {
    let closed = Rc::new(RefCell::new(Vec::new()));
    let state = Rc::new(Cell::new(None::<Signal<bool>>));
    let (c, s) = (closed.clone(), state.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let shown = Signal::new(true);
        s.set(Some(shown));
        let (a, b) = (c.clone(), c.clone());
        rsx! {
            div {
                Window { title: "One".to_owned(), on_closed: move || a.borrow_mut().push("one"),
                    p { "one" }
                }
                if shown.get() {
                    Window { title: "Two".to_owned(), on_closed: { let b = b.clone(); move || b.borrow_mut().push("two") },
                        p { "two" }
                    }
                }
            }
        }
    });
    let windows = app.windows();
    assert_eq!(windows.len(), 2);
    windows[0].close();
    app.settle();
    assert_eq!(app.windows(), [windows[1]]);
    assert!(!windows[0].is_open());
    assert_eq!(*closed.borrow(), ["one"], "its on_closed ran");
    state.get().unwrap().set(false);
    app.settle();
    assert!(app.windows().is_empty());
    assert!(!windows[1].is_open());
    assert_eq!(*closed.borrow(), ["one"], "not for its owner's going");
}

/// The system's close: a layer surface's `closed` closes it with no veto;
/// a toplevel's title-bar close asks its app, which may refuse.
#[test]
fn the_systems_close_follows_the_surface_kind() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                Window { title: "Top".to_owned(),
                    on_close_requested: || false,
                    p { "top" }
                }
                Window { title: "Layer".to_owned(),
                    wayland_layer: Some(LayerSurface::default()),
                    on_close_requested: || false,
                    p { "layer" }
                }
            }
        }
    });
    let windows = app.windows();
    app.window(windows[0]).close_by_system();
    assert_eq!(app.windows().len(), 2, "the toplevel refused");
    app.window(windows[1]).close_by_system();
    assert_eq!(app.windows(), [windows[0]], "the layer surface had no say");
}

/// A child's content with a row whose `bounds_signal` reader widens it
/// once it has been measured narrow, and shows the width it measured.
fn measuring(__scope: &mut RenderScope) -> NodeHandle {
    let wide = Signal::new(false);
    let measured = Signal::new(0.0_f32);
    let row = rsx! {
        div { id: "row", style: {move || if wide.get() { "width: 200px; height: 10px" } else { "width: 100px; height: 10px" }} }
    };
    let bounds = row.bounds_signal();
    rinch::Effect::new(move || {
        let b = bounds.get();
        measured.set(b.width);
        if b.width > 0.0 && b.width < 150.0 {
            wide.set(true);
        }
    });
    rsx! { div { {row} p { id: "out", {move || measured.get().to_string()} } } }
}

/// Each turn resolves an open child's layout and bounds: a write made by a
/// `bounds_signal` reader settles within one `settle`.
#[test]
fn a_childs_bounds_readers_settle() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                Window { title: "Child".to_owned(), width: 300, height: 200,
                    {measuring(__scope)}
                }
            }
        }
    });
    app.settle();
    let child = app.windows()[0];
    let window = app.window(child);
    assert_eq!(window.bounds("#row").width, 200.0);
    assert_eq!(window.text("#out"), "200");
}

/// A windowless-style root with two children, each with a reader made in
/// its mount: each follows its own window's configures, neither the
/// other's nor the main window's. (Moved here from the runtime's
/// `open_windows_tests`, #2789's pin: the public harness shows all it
/// pinned.)
#[test]
fn two_childrens_readers_each_follow_their_own_window() {
    let (left, right): (Reads, Reads) = Default::default();
    let (l, r) = (left.clone(), right.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (l, r) = (l.clone(), r.clone());
        rsx! {
            div {
                Window { title: "Left".to_owned(), width: 640, height: 480,
                    {reading(__scope, l.clone(), "l")}
                }
                Window { title: "Right".to_owned(), width: 300, height: 200,
                    {reading(__scope, r.clone(), "r")}
                }
            }
        }
    });
    let windows = app.windows();
    app.window(windows[1]).set_scale(1.5);
    assert_eq!(
        left.borrow().last(),
        Some(&((640.0, 480.0), (640, 480), 1.0))
    );
    assert_eq!(
        right.borrow().last(),
        Some(&((300.0, 200.0), (450, 300), 1.5))
    );
    assert_eq!(window_size(), (800.0, 600.0), "the main window's");
}

/// A child whose button closes it on the press is clicked while a second
/// child is open: the release has nowhere to go, and the second child gets
/// nothing.
#[test]
fn a_click_that_closes_its_window_sends_the_release_nowhere() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let (a, b) = (l.clone(), l.clone());
        rsx! {
            div {
                Window { title: "Closing".to_owned(), width: 200, height: 100,
                    button { id: "close", style: "width: 200px; height: 100px",
                        onmousedown: move || {
                            a.borrow_mut().push("closing down");
                            WindowHandle::current().unwrap().close();
                        },
                        "Close"
                    }
                }
                Window { title: "Other".to_owned(), width: 200, height: 100,
                    div { style: "width: 200px; height: 100px",
                        onmouseup: move || b.borrow_mut().push("other up"),
                    }
                }
            }
        }
    });
    let windows = app.windows();
    let mut closing = app.window(windows[0]);
    closing.click_on("#close");
    assert!(!closing.is_open());
    assert_eq!(app.windows(), [windows[1]]);
    assert_eq!(*log.borrow(), ["closing down"], "nothing reached the other");
}

/// A `TestWindow` used after its window closed panics, naming the window,
/// rather than driving whichever window took its place.
#[test]
#[should_panic(expected = "is no longer open")]
fn a_test_window_used_after_its_window_closed_panics_naming_it() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                Window { title: "Layer".to_owned(),
                    wayland_layer: Some(LayerSurface::default()),
                    p { id: "a", "a" }
                }
                Window { title: "Other".to_owned(), p { id: "a", "b" } }
            }
        }
    });
    let first = app.windows()[0];
    let mut window = app.window(first);
    window.close_by_system();
    let _ = window.text("#a");
}

/// A chord in a child: its held modifiers reach its keys, as the shell
/// reports them to the window with the keyboard, and a menu chord runs.
#[test]
fn a_chord_in_a_child_runs() {
    use rinch::menu::{Menu, MenuItem, register_menu_shortcuts};
    let fired = Rc::new(Cell::new(0u32));
    let f = fired.clone();
    let menu = Menu::new().item(
        MenuItem::new("New")
            .shortcut("Ctrl+N")
            .on_click(move || f.set(f.get() + 1)),
    );
    let _chords = register_menu_shortcuts(&[("File", &menu)]);
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! { div { Window { title: "Child".to_owned(), p { "child" } } } }
    });
    let child = app.windows()[0];
    let mut window = app.window(child);
    window.press_key("n");
    assert_eq!(fired.get(), 0, "no modifier held: no chord");
    window.set_modifiers(ModifierState {
        ctrl: true,
        ..ModifierState::default()
    });
    window.press_key("n");
    assert_eq!(fired.get(), 1, "Ctrl held in the child: the chord runs");
}

/// A handle from the main window's document used on a child panics rather
/// than reading another node with the same id there.
#[test]
#[should_panic(expected = "used on the window of document")]
fn a_handle_of_another_document_is_refused() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! { div { p { id: "main", "main" } Window { title: "Child".to_owned(), p { "child" } } } }
    });
    let main = app.handle("#main");
    let child = app.windows()[0];
    let _ = app.window(child).text(main);
}
