//! `TestApp` playing outputs (#2836), through the public harness only: a
//! window per played output, every output reader run again as the shell's
//! publication runs it, the paths a child takes when it is opened (placed by
//! the id of a played output, of one not played, by a name, with nothing
//! played), a content-sized window fitting and starting on its output, two
//! apps on one thread, and an output removed under a child.

#![cfg(all(feature = "testing", feature = "windowing"))]

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

/// [`out`] at `width` x `height`, scale 1.
fn sized(id: u64, name: &str, width: f64, height: f64) -> Output {
    Output {
        size: (width, height),
        ..out(id, name, 1.0)
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
/// A reader of `output(id)` runs only when that output's own record changes,
/// appears or goes (#2837): `output(1)` does not run when only output 2
/// changed, and neither runs when a third is added.
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

    let mut dp1 = out(1, "DP-1", 1.0);
    let mut dp2 = out(2, "DP-2", 1.0);
    let dp3 = out(3, "DP-3", 1.0);
    let mut steps: Vec<(&str, Vec<Output>, [u32; 6])> = vec![
        // Output 2 is still None: its reader does not run.
        ("first list", vec![dp1.clone()], [1, 1, 1, 0, 1, 1]),
        (
            "output 2 added",
            vec![dp1.clone(), dp2.clone()],
            [1, 1, 0, 1, 0, 1],
        ),
    ];
    dp2.name = Some("HDMI-A-1".to_owned());
    steps.push((
        "output 2 renamed",
        vec![dp1.clone(), dp2.clone()],
        [1, 0, 0, 1, 0, 1],
    ));
    dp2.scale = 2.0;
    steps.push((
        "output 2 rescaled",
        vec![dp1.clone(), dp2.clone()],
        [1, 0, 0, 1, 0, 1],
    ));
    dp1.scale = 1.5;
    steps.push((
        "output 1 rescaled",
        vec![dp1.clone(), dp2.clone()],
        [1, 0, 1, 0, 0, 1],
    ));
    steps.push((
        "the same list again",
        vec![dp1.clone(), dp2.clone()],
        [0; 6],
    ));
    steps.push((
        "output 3 added",
        vec![dp1.clone(), dp2.clone(), dp3.clone()],
        [1, 1, 0, 0, 0, 1],
    ));
    steps.push((
        "output 2 removed",
        vec![dp1.clone(), dp3.clone()],
        [1, 1, 0, 1, 0, 1],
    ));
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
/// played output, listed second (opened), by the id of one not played
/// (refused as the shell refuses it: never opened, its `on_closed` called
/// once), and by a name no played output has (opened: the compositor
/// chooses).
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
    app.set_outputs([out(3, "DP-3", 1.0), out(1, "DP-1", 1.0)]);
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
/// listed second, not at the first listed's other scale, as the shell
/// measures it for the surface it will get (#2652); a child that is not
/// content-sized starts at the main window's.
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

/// A content-sized window wider than its content may be: on `DP-2` by its
/// `monitor`.
fn wide_on_dp2() -> TestApp {
    TestApp::builder(|__scope: &mut RenderScope| {
        rsx! { div { style: "width: 3000px; height: 50px" } }
    })
    .window_props(WindowProps {
        size_to_content: Some(rinch::ContentSizing::new()),
        monitor: Some("DP-2".to_owned()),
        ..WindowProps::default()
    })
    .mount()
}

/// A content-sized window is as wide as its content up to the width of
/// the output it is on (by its `monitor`, listed second, narrower than the
/// first), and follows that output when it is resized; with no output
/// played its props' width is the limit.
#[test]
fn a_content_sized_window_fits_the_output_it_is_on() {
    let mut app = wide_on_dp2();
    assert_eq!(app.size(), (800, 50), "the props' width, with no output");
    app.set_outputs([
        sized(1, "DP-1", 1920.0, 1080.0),
        sized(2, "DP-2", 1280.0, 1024.0),
    ]);
    assert_eq!(
        app.size(),
        (1280, 50),
        "its output's width, not the first's"
    );
    app.set_outputs([
        sized(1, "DP-1", 1920.0, 1080.0),
        sized(2, "DP-2", 1600.0, 900.0),
    ]);
    assert_eq!(app.size(), (1600, 50), "its output's new width");
}

/// A content-sized window mounted while another app on the thread has
/// outputs played starts at its size on its output, as the shell mounts one
/// at its largest size for the outputs listed then.
#[test]
fn a_content_sized_window_mounted_with_outputs_played_starts_on_its_output() {
    let mut first = counting();
    first.set_outputs([
        sized(1, "DP-1", 1920.0, 1080.0),
        sized(2, "DP-2", 1280.0, 1024.0),
    ]);
    let app = wide_on_dp2();
    assert_eq!(app.size(), (1280, 50), "its output's width from the mount");
    drop(first);
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
/// store per thread: the last `set_outputs` wins for both, and an app
/// mounted while the list is played reads it from its first render.
/// Dropping one leaves the other's; dropping the last forgets them, so a new
/// app on the thread reads none and `NotListed`.
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
    let late = counting();
    assert_eq!(
        count_of(&late),
        "1",
        "a new app reads the list already played"
    );
    drop(late);
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

/// A child's content with a reader of `outputs()` (counted by `c`) and one
/// of `output_ids()` (by `d`).
fn child_readers(
    __scope: &mut RenderScope,
    c: impl Fn() + 'static,
    d: impl Fn() + 'static,
) -> NodeHandle {
    rinch::Effect::new(move || {
        let _ = outputs();
        c()
    });
    rinch::Effect::new(move || {
        let _ = output_ids();
        d()
    });
    rsx! { p { "child" } }
}

/// The last app on a thread forgets the outputs as it goes, and none of its
/// readers, in its own document or in a child's, runs for it: its documents
/// are disposed before the list is emptied, so the shell's rule holds that
/// an app never sees its outputs vanish because it is going.
#[test]
fn the_last_apps_readers_do_not_run_as_it_forgets_the_outputs() {
    let runs: Rc<[Cell<u32>; 4]> = Rc::default();
    let r = runs.clone();
    let open: Out<bool> = Rc::default();
    let o = open.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let count = |at: usize| {
            let r = r.clone();
            move || r[at].set(r[at].get() + 1)
        };
        let (a, b, c, d) = (count(0), count(1), count(2), count(3));
        rinch::Effect::new(move || {
            let _ = outputs();
            a()
        });
        rinch::Effect::new(move || {
            let _ = output_ids();
            b()
        });
        let shown = Signal::new(false);
        o.set(Some(shown));
        rsx! {
            div {
                if shown.get() {
                    Window { title: "child".to_owned(), wayland_layer: on(1),
                        {child_readers(__scope, c.clone(), d.clone())}
                    }
                }
            }
        }
    });
    app.set_outputs([out(1, "DP-1", 1.0)]);
    open.get().expect("the mount made it").set(true);
    app.settle();
    assert_eq!(app.windows().len(), 1);
    let before: [u32; 4] = std::array::from_fn(|i| runs[i].get());
    assert!(
        before.iter().all(|&n| n > 0),
        "every reader ran: {before:?}"
    );
    drop(app);
    let after: [u32; 4] = std::array::from_fn(|i| runs[i].get());
    assert_eq!(
        after, before,
        "outputs() and output_ids() in the app, then in its child: none runs in the drop"
    );
    assert_eq!(outputs(), [], "the list is forgotten all the same");
    assert_eq!(output_tracking(), OutputTracking::NotListed);
}

/// A window that asks to be sized by its content but whose compositor sizes
/// both axes (a layer surface anchored on all four sides, with both axes
/// the compositor's), with an app maximum, mounted at `size` when given
/// (the builder's), else at its props' 800x600. Returns the app.
fn compositor_sized(size: Option<(u32, u32)>) -> TestApp {
    let builder = TestApp::builder(|__scope: &mut RenderScope| {
        rsx! {
            div { id: "full", style: "width: 100%; height: 100%",
                span { id: "physical", {|| format!("{:?}", rinch::window::window_physical_size())} }
            }
        }
    })
    .window_props(WindowProps {
        size_to_content: Some(
            rinch::ContentSizing::new()
                .with_max_width(300)
                .with_max_height(200),
        ),
        wayland_layer: Some(LayerSurface {
            anchor: LayerAnchor::ALL,
            compositor_size: LayerAxes::BOTH,
            ..LayerSurface::default()
        }),
        ..WindowProps::default()
    });
    match size {
        Some((width, height)) => builder.size(width, height),
        None => builder,
    }
    .mount()
}

/// What `app` says of its window's size three ways: its size, its surface's
/// physical size as the document reads it, and a 100% element's box.
fn sizes(app: &TestApp) -> ((u32, u32), String, (f32, f32)) {
    let full = app.bounds("#full");
    (app.size(), app.text("#physical"), (full.width, full.height))
}

/// A window whose compositor sizes both axes is not sized by its content,
/// whatever maximum it asks for: the shell does not size it, and neither
/// does the harness. It is mounted at its props' size, and its size, its
/// surface and its layout agree.
#[test]
fn a_window_whose_compositor_sizes_both_axes_is_mounted_at_its_props_size() {
    let app = compositor_sized(None);
    assert_eq!(
        sizes(&app),
        ((800, 600), "(800, 600)".to_owned(), (800.0, 600.0)),
        "the size, the physical size and a 100% element's box"
    );
}

/// The same window mounted at the builder's size, which differs from its
/// props': its size, its surface and its layout agree at that size.
#[test]
fn a_window_whose_compositor_sizes_both_axes_is_mounted_at_the_builders_size() {
    let app = compositor_sized(Some((640, 480)));
    assert_eq!(
        sizes(&app),
        ((640, 480), "(640, 480)".to_owned(), (640.0, 480.0)),
        "the size, the physical size and a 100% element's box"
    );
}
