//! The browser halves of rinch's runtime (issue #1956): what makes
//! `set_timeout`, `spawn_local` and `poll_signal` run on wasm32.
//!
//! On native targets rinch-core supplies these itself (a shared timer thread,
//! the main-thread callback queue the host drains, the host's per-frame poll
//! drain). A browser has no threads and no rinch frame loop, so the web
//! runtime schedules them on the page instead: `window.setTimeout` for timers,
//! a microtask for each task poll, and a frame or timeout for each poll drain.
//!
//! Here rather than in `rinch-web` so that a page that mounts no rinch DOM —
//! a game that draws its HUD through [`embed::RinchContext`](crate::embed) and
//! depends on `rinch` alone — reaches them too. `rinch_web::mount*` and
//! `rinch_web::install_runtime` call [`install_web_runtime`]; a `RinchContext`
//! created on wasm32 installs the scheduling half itself.

mod polls;

use std::cell::Cell;

thread_local! {
    static SCHEDULING_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static POLLS_INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The browser's handle for each timer armed by [`web_set_timeout`] that
    /// has not fired yet, by timer id.
    static TIMEOUTS: std::cell::RefCell<std::collections::HashMap<u64, i32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Install the web runtime: the timer backend, the task scheduler and the
/// poll drain. Idempotent; builds no DOM and mounts nothing.
///
/// Call it once at startup in a page that never calls `rinch_web::mount*`
/// (which installs it itself), before arming timers: a `set_timeout` armed
/// earlier never fires, while a task spawned earlier is picked up by the
/// install.
///
/// An app whose only rinch host is an embedded
/// [`RinchContext`](crate::embed::RinchContext) does not need this: the
/// context installs the timer backend and task scheduler when it is created,
/// and its `update()` drains polls once per frame. Calling it there as well
/// is harmless; it adds a poll drain between frames, so a
/// [`PollRate::EveryFrame`](rinch_core::reactive::PollRate) poll is then
/// sampled on the browser's animation frame too.
pub fn install_web_runtime() {
    install_web_scheduling();
    if !POLLS_INSTALLED.with(|f| f.replace(true)) {
        // Sample `poll_signal`s: the native hosts drain them once per frame,
        // and the browser has no rinch frame loop to do it (#1773).
        polls::install();
    }
}

/// The scheduling half of [`install_web_runtime`]: timers and tasks, without
/// the poll drain. Idempotent.
pub(crate) fn install_web_scheduling() {
    if SCHEDULING_INSTALLED.with(|f| f.replace(true)) {
        return;
    }
    // Drive `rinch_core::set_timeout` off `window.setTimeout`. The browser has
    // no threads, so rinch-core's built-in shared-thread scheduler cannot run
    // here.
    rinch_core::set_timer_backend_with_cancel(web_set_timeout, web_clear_timeout);
    // `yield_now` (work in slices, #2862) through a `MessageChannel`: a
    // nested `setTimeout(0)` waits at least 4 ms, a posted message does not.
    rinch_core::set_yield_backend(web_yield);
    // And `rinch_core::spawn_local`'s polls off microtasks: nothing drains the
    // main-thread callback queue in a browser unless an embedded context's
    // `update()` does, once a frame.
    rinch_core::task::set_task_scheduler(web_schedule_task);
}

/// The web timer backend: schedule `fire_timeout(id)` via `window.setTimeout`.
///
/// Only `id` is captured, so the parked callback stays in rinch-core's
/// main-thread registry. The closure is created with `Closure::once_into_js`,
/// which hands ownership to the JS GC and drops the boxed `FnOnce` after it fires
/// — so, unlike `Closure::forget()`, arming thousands of timers over a long
/// session does not leak a closure per timer. The browser's handle is kept in
/// `TIMEOUTS` until the timer fires, so a `clear_timeout` can withdraw it
/// ([`web_clear_timeout`], #2832).
fn web_set_timeout(id: u64, delay_ms: u32) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let cb = Closure::once_into_js(move || {
        TIMEOUTS.with(|t| t.borrow_mut().remove(&id));
        rinch_core::fire_timeout(id)
    });
    if let Some(window) = web_sys::window()
        && let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.unchecked_ref(),
            delay_ms as i32,
        )
    {
        TIMEOUTS.with(|t| t.borrow_mut().insert(id, handle));
    }
}

/// The web `yield_now` backend: `fire_timeout(id)` in the task a
/// `MessageChannel` message starts. One channel for the page; the ids wait in
/// order in `YIELDS`, one message each. Falls back to a 0 ms timeout where
/// the channel cannot be made.
fn web_yield(id: u64) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    thread_local! {
        static YIELDS: std::cell::RefCell<std::collections::VecDeque<u64>> =
            const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
        static CHANNEL: Option<web_sys::MessageChannel> = {
            let channel = web_sys::MessageChannel::new().ok()?;
            let on_message = Closure::<dyn FnMut()>::new(|| {
                if let Some(id) = YIELDS.with(|y| y.borrow_mut().pop_front()) {
                    rinch_core::fire_timeout(id);
                }
            });
            channel
                .port1()
                .set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            // The page's one listener, for the page's lifetime.
            on_message.forget();
            Some(channel)
        };
    }
    let posted = CHANNEL.with(|c| {
        c.as_ref().is_some_and(|c| {
            YIELDS.with(|y| y.borrow_mut().push_back(id));
            c.port2().post_message(&wasm_bindgen::JsValue::NULL).is_ok()
        })
    });
    if !posted {
        YIELDS.with(|y| y.borrow_mut().retain(|&i| i != id));
        web_set_timeout(id, 0);
    }
}

/// The web timer backend's cancel: `window.clearTimeout` for a timer that has
/// not fired, so a cleared timeout does not wake the page at its deadline. A
/// cleared closure is dropped by the JS GC with the browser's reference to it.
fn web_clear_timeout(id: u64) {
    let handle = TIMEOUTS.with(|t| t.borrow_mut().remove(&id));
    if let Some(handle) = handle
        && let Some(window) = web_sys::window()
    {
        window.clear_timeout_with_handle(handle);
    }
}

/// Poll a `spawn_local` task in a microtask (issue #1767): as soon as the
/// current JS task is done, as `wasm_bindgen_futures::spawn_local` does, with
/// no thread and no frame to wait for.
fn web_schedule_task(id: u64) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let poll = Closure::once_into_js(move || rinch_core::task::poll_task(id));
    let global = js_sys::global();
    if let Ok(queue) = js_sys::Reflect::get(&global, &"queueMicrotask".into())
        && let Some(queue) = queue.dyn_ref::<js_sys::Function>()
    {
        let _ = queue.call1(&global, &poll);
    }
}
