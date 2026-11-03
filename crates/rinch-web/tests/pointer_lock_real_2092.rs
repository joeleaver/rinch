//! A real pointer lock in Chrome (issue #2092): a trusted click on a
//! `RenderSurface` whose `MouseDown` asks for the lock, trusted mouse moves
//! while it holds, and the release by `exit_pointer_lock` and by the
//! surface's unmount. The input is sent over the DevTools
//! protocol by `support/real_keys.mjs`; a script's own events carry no user
//! activation, and the browser grants a lock only with one.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver crates/rinch-web/tests/support/run_real_keys.sh
//! ```
//!
//! Chrome only: Firefox has no CDP input here (`pointer_lock_2092.rs` covers
//! rinch's half in both engines, with the browser's half played). Escape is
//! not here: a CDP key reaches the page without passing the browser's own
//! Escape handling, so in headless Chrome it does not exit the lock (measured:
//! the lock stayed), and the browser's Escape is not something a test can
//! send.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::render_surface::{RenderSurface, SurfaceEvent, create_render_surface};
use rinch::window::{PointerLockMode, exit_pointer_lock, pointer_lock};
use rinch_core::Component;
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen_test::*;

#[path = "support/real_input.rs"]
mod real_input;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

thread_local! {
    static N: Cell<u32> = const { Cell::new(0) };
}

fn next() -> u32 {
    N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

async fn real_click(x: f64, y: f64) {
    let n = next();
    real_input::ask_helper(
        &format!("RINCH-REAL-CLICK:{n}:{},{}", x.round(), y.round()),
        &format!("RINCH-REAL-CLICK-SENT:{n}"),
        &format!("click #{n}"),
    )
    .await;
}

async fn real_moves(points: &[(f64, f64)]) {
    let n = next();
    let path: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("{},{}", x.round(), y.round()))
        .collect();
    real_input::ask_helper(
        &format!("RINCH-REAL-MOVE:{n}:{}", path.join(";")),
        &format!("RINCH-REAL-MOVE-SENT:{n}"),
        &format!("moves #{n}"),
    )
    .await;
}

/// Wait up to a second for `pointer_lock()` to answer `want`.
async fn until_lock(want: Option<PointerLockMode>) -> Option<PointerLockMode> {
    for _ in 0..50 {
        if pointer_lock() == want {
            break;
        }
        sleep(20).await;
    }
    pointer_lock()
}

#[wasm_bindgen_test]
#[ignore = "needs trusted input: run tests/support/run_real_keys.sh"]
async fn a_trusted_click_locks_moves_arrive_as_motion_and_exit_and_unmount_release() {
    assert!(
        option_env!("RINCH_REAL_KEYS").is_some(),
        "built without RINCH_REAL_KEYS: run tests/support/run_real_keys.sh, which sends the input"
    );
    let surface = create_render_surface();
    let asked: Rc<RefCell<Vec<String>>> = Rc::default();
    let motion = Rc::new(Cell::new((0.0_f32, 0.0_f32)));
    let (s, a, m) = (surface.clone(), asked.clone(), motion.clone());
    surface.set_event_handler(move |e| match e {
        SurfaceEvent::MouseDown { .. } => {
            a.borrow_mut()
                .push(format!("{:?}", s.request_pointer_lock()));
        }
        SurfaceEvent::PointerMotion { dx, dy } => {
            let (x, y) = m.get();
            m.set((x + dx, y + dy));
        }
        _ => {}
    });
    let host = document().create_element("div").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 40px; top: 40px; width: 300px; height: 200px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let sf = surface.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            RenderSurface {
                surface: Some(sf.clone()),
            }
            .render(scope, &[])
        },
    );
    sleep(50).await;

    real_click(190.0, 140.0).await;
    let granted = until_lock(Some(PointerLockMode::Locked)).await;
    let canvas_locked = document()
        .pointer_lock_element()
        .is_some_and(|e| e.id() == format!("rinch-surface-{}", surface.id()));

    real_moves(&[(200.0, 140.0), (230.0, 120.0), (260.0, 100.0)]).await;
    sleep(50).await;
    let moved = motion.get();

    // A real exit: rinch asks, the browser lets go.
    exit_pointer_lock();
    let after_exit = until_lock(None).await;
    let browser_after_exit = document().pointer_lock_element().is_none();

    // A second gesture locks again; the surface's unmount gives it back.
    real_click(190.0, 140.0).await;
    let regranted = until_lock(Some(PointerLockMode::Locked)).await;
    root.unmount();
    let after_unmount = until_lock(None).await;
    let browser_after_unmount = document().pointer_lock_element().is_none();
    host.remove();

    web_sys::console::log_1(&format!("pointer_lock_real_2092: motion {moved:?}").into());
    assert_eq!(
        *asked.borrow(),
        ["Ok(())", "Ok(())"],
        "each press asked once"
    );
    assert_eq!(
        granted,
        Some(PointerLockMode::Locked),
        "the trusted click locked"
    );
    assert!(canvas_locked, "the surface's canvas holds the lock");
    assert!(
        moved.0 > 0.0 && moved.1 < 0.0,
        "trusted moves right and up reach the surface as motion: {moved:?}"
    );
    assert_eq!(after_exit, None, "exit_pointer_lock released it");
    assert!(browser_after_exit, "and the browser let go");
    assert_eq!(
        regranted,
        Some(PointerLockMode::Locked),
        "a second click locks again"
    );
    assert_eq!(after_unmount, None, "the unmount released it");
    assert!(browser_after_unmount, "and the browser let go");
}
