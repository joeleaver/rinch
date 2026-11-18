//! `TestApp` playing outputs (#2836), through the public harness only: a
//! window per played output, every output reader run again as the shell's
//! publication runs it, the paths a child takes when it is opened (placed by
//! the id of a played output, of one not played, by a name, with nothing
//! played), a content-sized window fitting and starting on its output, two
//! apps on one thread, and an output removed under a child.

#![cfg(all(feature = "testing", feature = "desktop"))]

use rinch::outputs::{
    Output, OutputId, OutputTracking, output, output_ids, output_tracking, outputs,
};
use rinch::prelude::*;
use rinch::testing::TestApp;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// An output as a compositor lists it: `id`, named `name`, 1920x1080 at
/// the origin, at `scale`.
fn out(id: u64, name: &str, scale: f64) -> Output {
    Output {
        id: OutputId::from_raw(id),
        name: Some(name.to_owned()),
        position: (0.0, 0.0),
        size: (1920.0, 1080.0),
        scale,
    }
}

/// A layer surface on the output `id`.
fn on(id: u64) -> Option<LayerSurface> {
    Some(LayerSurface {
        output_id: Some(OutputId::from_raw(id)),
        ..LayerSurface::default()
    })
}

/// A signal the component makes and the test sets, handed out of the mount.
type Out<T> = Rc<Cell<Option<Signal<T>>>>;

/// Two bars on two named outputs, as a panel opens one per output: each
/// bar is a window on its own output, showing its own output's name. None
/// opens before the outputs are played.
#[test]
fn two_bars_on_two_named_outputs_each_show_their_own_output() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                for id in output_ids() {
                    Window { key: id, title: "bar".to_owned(), wayland_layer: Some(LayerSurface {
                            output_id: Some(id),
                            ..LayerSurface::default()
                        }),
                        p { id: "name", {move || output(id).and_then(|o| o.name).unwrap_or_default()} }
                    }
                }
            }
        }
    });
    assert_eq!(app.windows(), [], "a bar before any output was played");
    app.set_outputs([out(1, "DP-1", 1.0), out(2, "HDMI-A-1", 1.0)]);
    let bars = app.windows();
    assert_eq!(bars.len(), 2, "one bar per played output");
    for (bar, (id, name)) in bars.iter().zip([(1, "DP-1"), (2, "HDMI-A-1")]) {
        let window = app.window(*bar);
        let layer = window
            .props()
            .wayland_layer
            .as_ref()
            .expect("a layer surface");
        assert_eq!(
            layer.output_id,
            Some(OutputId::from_raw(id)),
            "on its own output"
        );
        assert_eq!(window.text("#name"), name, "showing its own output's name");
    }
}

/// The readers [`readers_run_again_on_each_change`] counts, in its table's
/// order.
const READERS: [&str; 6] = [
    "outputs()",
    "output_ids()",
    "output(1), the output that stays",
    "output(2), the output that changes",
    "output_tracking()",
    "outputs() and output_tracking() in one reader",
];

/// Every reader of the outputs runs again on each kind of change as the
/// shell's publication runs it, once per change; an equal list runs none.
/// The counts are today's: `output(id)` runs on any output's change
/// (#2837), so `output(1)` runs when only output 2 changed.
#[test]
fn readers_run_again_on_each_change() {
    let runs: Rc<[Cell<u32>; 6]> = Rc::default();
    let r = runs.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let count = |r: &Rc<[Cell<u32>; 6]>, at: usize| {
            let r = r.clone();
            move || r[at].set(r[at].get() + 1)
        };
        let (a, b, c, d, e, f) = (
            count(&r, 0),
            count(&r, 1),
            count(&r, 2),
            count(&r, 3),
            count(&r, 4),
            count(&r, 5),
        );
        let (one, two) = (OutputId::from_raw(1), OutputId::from_raw(2));
        rinch::Effect::new(move || {
            let _ = outputs();
            a()
        });
        rinch::Effect::new(move || {
            let _ = output_ids();
            b()
        });
        rinch::Effect::new(move || {
            let _ = output(one);
            c()
        });
        rinch::Effect::new(move || {
            let _ = output(two);
            d()
        });
        rinch::Effect::new(move || {
            let _ = output_tracking();
            e()
        });
        rinch::Effect::new(move || {
            let _ = (outputs(), output_tracking());
            f()
        });
        rsx! { div {} }
    });
    let taken = || -> [u32; 6] { std::array::from_fn(|i| runs[i].replace(0)) };
    assert_eq!(taken(), [1; 6], "each reader's first run, at the mount");

    let dp1 = out(1, "DP-1", 1.0);
    let mut dp2 = out(2, "DP-2", 1.0);
    let mut steps: Vec<(&str, Vec<Output>, [u32; 6])> = vec![
        ("first list", vec![dp1.clone()], [1, 1, 1, 1, 1, 1]),
        (
            "output 2 added",
            vec![dp1.clone(), dp2.clone()],
            [1, 1, 1, 1, 0, 1],
        ),
    ];
    dp2.name = Some("HDMI-A-1".to_owned());
    steps.push((
        "output 2 renamed",
        vec![dp1.clone(), dp2.clone()],
        [1, 0, 1, 1, 0, 1],
    ));
    dp2.scale = 2.0;
    steps.push((
        "output 2 rescaled",
        vec![dp1.clone(), dp2.clone()],
        [1, 0, 1, 1, 0, 1],
    ));
    steps.push((
        "the same list again",
        vec![dp1.clone(), dp2.clone()],
        [0; 6],
    ));
    steps.push(("output 2 removed", vec![dp1.clone()], [1, 1, 1, 1, 0, 1]));
    for (step, list, expected) in steps {
        app.set_outputs(list);
        let got = taken();
        for (reader, (got, expected)) in READERS.iter().zip(got.iter().zip(expected)) {
            assert_eq!(*got, expected, "{reader} ran {got} times on \"{step}\"");
        }
    }
}

/// With nothing played the harness is as it was: no outputs listed, and a
/// child placed by an output id or a name is opened, never refused.
#[test]
fn with_nothing_played_no_output_is_listed_and_no_child_is_refused() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        rsx! {
            div {
                Window { title: "by id".to_owned(), wayland_layer: on(7), p { "id" } }
                Window { title: "by name".to_owned(), wayland_layer: Some(LayerSurface {
                        output: Some("DP-9".to_owned()),
                        ..LayerSurface::default()
                    }),
                    p { "name" }
                }
            }
        }
    });
    assert_eq!(outputs(), []);
    assert_eq!(output_tracking(), OutputTracking::NotListed);
    let titles: Vec<String> = app
        .windows()
        .into_iter()
        .map(|w| app.window(w).props().title.clone())
        .collect();
    assert_eq!(titles, ["by id", "by name"], "both opened");
}

/// Played outputs, then three children opened: placed by the id of a
/// played output (opened), by the id of one not played (refused as the
/// shell refuses it: never opened, its `on_closed` called once), and by a
/// name no played output has (opened: the compositor chooses).
#[test]
fn a_child_placed_by_the_id_of_an_output_not_played_is_refused() {
    let closed = Rc::new(RefCell::new(Vec::new()));
    let open: Out<bool> = Rc::default();
    let (c, o) = (closed.clone(), open.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let shown = Signal::new(false);
        o.set(Some(shown));
        let close = |name: &'static str| {
            let c = c.clone();
            move || c.borrow_mut().push(name)
        };
        let (a, b, n) = (close("played"), close("not played"), close("by name"));
        rsx! {
            div {
                if shown.get() {
                    Window { title: "played".to_owned(), wayland_layer: on(1), on_closed: a.clone(), p { "1" } }
                    Window { title: "not played".to_owned(), wayland_layer: on(7), on_closed: b.clone(), p { "7" } }
                    Window { title: "by name".to_owned(), on_closed: n.clone(),
                        wayland_layer: Some(LayerSurface {
                            output: Some("DP-9".to_owned()),
                            ..LayerSurface::default()
                        }),
                        p { "name" }
                    }
                }
            }
        }
    });
    app.set_outputs([out(1, "DP-1", 1.0)]);
    open.get().expect("the mount made it").set(true);
    app.settle();
    let titles: Vec<String> = app
        .windows()
        .into_iter()
        .map(|w| app.window(w).props().title.clone())
        .collect();
    assert_eq!(
        titles,
        ["played", "by name"],
        "the child on output 7 was made"
    );
    assert_eq!(*closed.borrow(), ["not played"], "its on_closed, once");
}

/// A content-sized child starts at the scale of the output it is placed on,
/// as the shell measures it for the surface it will get (#2652); a child
/// that is not content-sized starts at the main window's.
#[test]
fn a_content_sized_child_starts_at_its_outputs_scale() {
    let open: Out<bool> = Rc::default();
    let o = open.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let shown = Signal::new(false);
        o.set(Some(shown));
        rsx! {
            div {
                if shown.get() {
                    Window { title: "content".to_owned(), wayland_layer: on(2),
                        size_to_content: Some(rinch::ContentSizing::new()),
                        div { style: "width: 100px; height: 20px" }
                    }
                    Window { title: "fixed".to_owned(), wayland_layer: on(2), p { "fixed" } }
                }
            }
        }
    });
    app.set_outputs([out(1, "DP-1", 1.0), out(2, "DP-2", 2.0)]);
    open.get().expect("the mount made it").set(true);
    app.settle();
    let scales: Vec<(String, f64)> = app
        .windows()
        .into_iter()
        .map(|w| {
            let window = app.window(w);
            (window.props().title.clone(), window.scale_factor())
        })
        .collect();
    assert_eq!(
        scales,
        [("content".to_owned(), 2.0), ("fixed".to_owned(), 1.0)],
        "the content-sized child at output 2's scale, the other at the main window's"
    );
}

/// A content-sized window is as wide as its content up to its output's
/// width, and follows the output when it is resized; with no output played
/// its props' width is the limit.
#[test]
fn a_content_sized_window_fits_the_output_it_is_on() {
    let mut app = TestApp::builder(|__scope: &mut RenderScope| {
        rsx! { div { style: "width: 3000px; height: 50px" } }
    })
    .window_props(WindowProps {
        size_to_content: Some(rinch::ContentSizing::new()),
        ..WindowProps::default()
    })
    .mount();
    assert_eq!(app.size(), (800, 50), "the props' width, with no output");
    app.set_outputs([out(1, "DP-1", 1.0)]);
    assert_eq!(app.size(), (1920, 50), "the output's width");
    let mut narrow = out(1, "DP-1", 1.0);
    narrow.size = (1280.0, 1024.0);
    app.set_outputs([narrow]);
    assert_eq!(app.size(), (1280, 50), "the output's new width");
}

/// The text a [`counting`] app shows: how many outputs it reads.
fn count_of(app: &TestApp) -> String {
    app.text("#n")
}

/// An app showing how many outputs it reads.
fn counting() -> TestApp {
    TestApp::mount(|__scope: &mut RenderScope| {
        rsx! { p { id: "n", {|| outputs().len().to_string()} } }
    })
}

/// Two apps on one thread read one list, the thread's, as the shell's one
/// store per thread: the last `set_outputs` wins for both. Dropping one
/// leaves the other's; dropping the last forgets them, so a new app on the
/// thread reads none and `NotListed`.
#[test]
fn two_apps_on_one_thread_share_the_list_and_the_last_to_go_forgets_it() {
    let mut first = counting();
    let mut second = counting();
    first.set_outputs([out(1, "DP-1", 1.0), out(2, "DP-2", 1.0)]);
    second.settle();
    assert_eq!(
        (count_of(&first), count_of(&second)),
        ("2".into(), "2".into())
    );
    second.set_outputs([out(3, "DP-3", 1.0)]);
    first.settle();
    assert_eq!(
        (count_of(&first), count_of(&second)),
        ("1".into(), "1".into())
    );
    drop(first);
    second.settle();
    assert_eq!(count_of(&second), "1", "kept for the app still live");
    assert_eq!(output_tracking(), OutputTracking::Live);
    drop(second);
    let third = counting();
    assert_eq!(count_of(&third), "0", "a new app after the last went");
    assert_eq!(output_tracking(), OutputTracking::NotListed);
}

/// An output removed under a child on it: the shell does nothing to the
/// child until the compositor closes its surface, which the test plays
/// with `close_by_system`; it then closes, with its `on_closed` once.
#[test]
fn a_child_stays_open_when_its_output_goes_until_the_compositor_closes_it() {
    let closed = Rc::new(Cell::new(0));
    let open: Out<bool> = Rc::default();
    let (c, o) = (closed.clone(), open.clone());
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let shown = Signal::new(false);
        o.set(Some(shown));
        let c = c.clone();
        rsx! {
            div {
                if shown.get() {
                    Window { title: "on 2".to_owned(), wayland_layer: on(2),
                        on_closed: { let c = c.clone(); move || c.set(c.get() + 1) },
                        p { "2" }
                    }
                }
            }
        }
    });
    app.set_outputs([out(1, "DP-1", 1.0), out(2, "DP-2", 1.0)]);
    open.get().expect("the mount made it").set(true);
    app.settle();
    let child = app.windows();
    assert_eq!(child.len(), 1);
    app.set_outputs([out(1, "DP-1", 1.0)]);
    assert_eq!(app.windows(), child, "still open with its output gone");
    assert_eq!(closed.get(), 0);
    app.window(child[0]).close_by_system();
    assert_eq!(app.windows(), []);
    assert!(!child[0].is_open());
    assert_eq!(closed.get(), 1, "its on_closed, once");
}
