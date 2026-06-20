//! Islands demo/test for `rinch-web`.
//!
//! Two fully independent rinch roots are hydrated into placeholder elements on
//! an otherwise static HTML page (see `index.html`). Each has its own reactive
//! state; one can be unmounted without affecting the other or the static
//! content. Stable element `id`s make the behavior assertable in a real browser
//! (e.g. Playwright).

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;

thread_local! {
    /// Island B's handle, so island A's "Unmount B" button can tear it down.
    static B_HANDLE: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

#[component]
fn island_a() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div { style: "padding: 10px; background: #e7f5ff; border-radius: 6px;",
            strong { "Island A" }
            p { id: "a-count", "count: " {|| count.get().to_string()} }
            button { id: "a-inc", onclick: move || count.update(|n| *n += 1), "A +1" }
            button {
                id: "unmount-b",
                onclick: move || {
                    if let Some(h) = B_HANDLE.with(|b| b.borrow_mut().take()) {
                        h.unmount();
                    }
                },
                "Unmount B"
            }
        }
    }
}

#[component]
fn island_b() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div { style: "padding: 10px; background: #fff3bf; border-radius: 6px;",
            strong { "Island B" }
            p { id: "b-count", "count: " {|| count.get().to_string()} }
            button { id: "b-inc", onclick: move || count.update(|n| *n += 1), "B +1" }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();

    let theme = ThemeProviderProps::default();

    // Independent root #1 → #island-a.
    rinch_web::mount_selector("#island-a", theme.clone(), island_a);

    // Independent root #2 → #island-b; keep its handle for unmount.
    if let Some(h) = rinch_web::mount_selector("#island-b", theme, island_b) {
        B_HANDLE.with(|b| *b.borrow_mut() = Some(h));
    }
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
