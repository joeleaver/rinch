//! A live check of several windows in one app (#2416 stage 4), for a nested
//! compositor in an isolated session:
//!
//! ```text
//! scripts/isolated-session.sh --kwin -- cargo run -p rinch --example windows_probe
//! ```
//!
//! It opens windows, checks what each one sees, closes them, and exits 0 when
//! every check passed (each prints `PASS`), 1 at the first that failed
//! (`FAIL`). The app runs with `ExitWhen::LastWindowCloses`: closing the main
//! window first must leave it running for the others, and the last window's
//! close must end it.

use std::cell::Cell;

use rinch::prelude::*;
use rinch_core::reactive::{Scope, unowned};

#[derive(Clone)]
struct AppStore(u32);

#[derive(Clone)]
struct SecondStore;

thread_local! {
    static SECOND_SAW_APP_STORE: Cell<Option<u32>> = const { Cell::new(None) };
    static SECOND_EFFECT_RUNS: Cell<u32> = const { Cell::new(0) };
    static SHARED: Cell<Option<Signal<u32>>> = const { Cell::new(None) };
}

fn check(name: &str, ok: bool) {
    if ok {
        println!("PASS {name}");
    } else {
        println!("FAIL {name}");
        std::process::exit(1);
    }
}

fn props(title: &str) -> WindowProps {
    WindowProps {
        title: title.into(),
        width: 300,
        height: 200,
        ..Default::default()
    }
}

/// The second window's content: reads the shared signal in an effect, makes
/// a store of its own, reads the app's, and resizes its own window.
fn second(scope: &mut RenderScope) -> NodeHandle {
    let shared = SHARED.with(|s| s.get()).unwrap();
    create_store(SecondStore);
    SECOND_SAW_APP_STORE.with(|s| s.set(try_use_store::<AppStore>().map(|a| a.0)));
    let root = scope.create_element("div");
    scope.create_effect(move || {
        let _ = shared.get();
        SECOND_EFFECT_RUNS.with(|r| r.set(r.get() + 1));
    });
    set_current_window_size(320.0, 240.0);
    root
}

fn plain(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

#[component]
fn app() -> NodeHandle {
    let shared = unowned(|| Signal::new(0u32));
    SHARED.with(|s| s.set(Some(shared)));
    unowned(|| create_store(AppStore(3)));
    set_timeout(500, move || step_open(shared));
    rsx! { div { "main" } }
}

fn step_open(shared: Signal<u32>) {
    let b = unowned(|| open_window(props("second"), second));
    let owner = Scope::new();
    let c = owner.run(|| open_window(props("owned"), plain));
    let d = unowned(|| open_window(props("unowned"), plain));
    check(
        "a handle is open at once",
        b.is_open() && c.is_open() && d.is_open(),
    );
    set_timeout(800, move || step_check(shared, b, owner, c, d));
}

fn step_check(
    shared: Signal<u32>,
    b: WindowHandle,
    owner: Scope,
    c: WindowHandle,
    d: WindowHandle,
) {
    check(
        "the app's store is seen in a second window",
        SECOND_SAW_APP_STORE.with(|s| s.get()) == Some(3),
    );
    check(
        "a second window's store is not seen in the main window",
        try_use_store::<SecondStore>().is_none(),
    );
    // A timer callback is a batch: its effects run when it returns, every
    // window's in that one flush.
    let before = SECOND_EFFECT_RUNS.with(|r| r.get());
    shared.set(1);
    set_timeout(0, move || {
        check(
            "a signal written in the main window ran the second window's effect once, in that flush",
            SECOND_EFFECT_RUNS.with(|r| r.get()) == before + 1,
        );
    });
    // A tiling compositor (sway) sizes every tiled window itself, the main
    // one included, and refuses a client's size request: there the routing is
    // the unit tests' (`open_windows_tests`).
    if std::env::var("DESKRINCH_TEST_COMPOSITOR").as_deref() == Ok("sway") {
        println!("SKIP window sizes: sway tiles the windows and sizes them itself");
    } else {
        check(
            "set_current_window_size from the second window resized it",
            b.size() == Some((320.0, 240.0)),
        );
        check(
            "and not the main window",
            WindowHandle::main().and_then(|m| m.size()) == Some((300.0, 200.0)),
        );
    }
    check(
        "windows() lists the main window first, then oldest first",
        windows() == vec![WindowHandle::main().unwrap(), b, c, d],
    );
    owner.dispose();
    set_timeout(400, move || {
        check("the owned window closed when its owner went", !c.is_open());
        check("the unowned window is still open", d.is_open());
        // The main window closes first: the app must run on. What runs after
        // it is set outside any window's ownership: a timer the main window's
        // code set goes with the main window's document.
        WindowHandle::main().unwrap().close();
        unowned(|| {
            set_timeout(400, move || {
                check(
                    "the app runs on without its main window",
                    WindowHandle::main().is_none(),
                );
                check(
                    "WindowHandle::current outside any window is none",
                    WindowHandle::current().is_none(),
                );
                b.close();
                d.close();
                // The last close ends the event loop; a run that does not end is
                // a failure the session's timeout reports.
            })
        });
    });
}

fn main() {
    App::new(app)
        .title("windows probe")
        .size(300, 200)
        .exit_when(ExitWhen::LastWindowCloses)
        .run();
    println!("PASS the last window's close ended the app");
}
