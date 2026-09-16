//! The DOM menu bar in a browser.
//!
//! The menus here are ordinary [`Menu`] / [`MenuItem`] values — the same type a
//! desktop build hands to `App::menu`. `mount_with_menu_bar` renders them above
//! the app with the renderer the Linux desktop uses, and arms each item's
//! shortcut against the document.
//!
//! Three things are worth clicking through once it is running:
//!
//! * open **File**, then slide across to **View** — the open menu follows the
//!   pointer, no second click;
//! * press **Ctrl+K** (Cmd+K on a Mac) with the page focused — the log gains a
//!   line and the browser's own Ctrl+K does *not* fire, because a claimed chord
//!   is consumed. **Ctrl+S** is declared on a disabled item, so it stays the
//!   browser's;
//! * click anywhere outside an open menu, or press Escape, to dismiss it.
//!
//! Build it the way the other web examples are built:
//!
//! ```bash
//! cd examples/menu-bar-web && trunk serve --release
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_web::{Menu, MenuItem};
use wasm_bindgen::prelude::*;

const CSS_WEB: &str = r#"
* { box-sizing: border-box; margin: 0; padding: 0; }

html, body {
    height: 100%;
    font-family: var(--rinch-font-family);
    background: var(--rinch-color-body);
    color: var(--rinch-color-text);
}

.page {
    padding: var(--rinch-spacing-xl);
    display: flex;
    flex-direction: column;
    gap: var(--rinch-spacing-md);
    max-width: 640px;
}

.log {
    font-family: var(--rinch-font-family-monospace);
    font-size: var(--rinch-font-size-sm);
    color: var(--rinch-color-dimmed);
    white-space: pre-line;
}
"#;

/// What the menu items did, most recent last.
///
/// The menus are built in `start`, before anything mounts — which is where every
/// in-tree menu is built, and what gives their callbacks the life of the page.
/// So a callback cannot capture state the component creates later; `start` makes
/// both this and the `entries` signal instead, and hands them to the component.
type Journal = Rc<RefCell<Vec<String>>>;

#[component]
fn app(journal: Journal, entries: Signal<String>, dark_mode: Signal<bool>) -> NodeHandle {
    rsx! {
        ThemeProvider {
            dark_mode_fn: Rc::new(move || dark_mode.get()),

            style { {CSS_WEB} }

            div { class: "page",
                Title { order: 2, "Menu bar" }
                Text {
                    "The File and View menus above come from the same \
                     `Menu` / `MenuItem` values a desktop build would hand to \
                     `App::menu`."
                }
                Text { size: "sm", color: "dimmed",
                    "Hover from one open menu to the next. Press Ctrl+K (Cmd+K) \
                     anywhere on the page."
                }
                Button {
                    variant: "light",
                    onclick: move || {
                        journal.borrow_mut().clear();
                        entries.set(String::new());
                    },
                    "Clear log"
                }
                div { class: "log", {|| {
                    let text = entries.get();
                    if text.is_empty() { "(nothing yet)".to_string() } else { text }
                }} }
            }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();

    let journal: Journal = Rc::new(RefCell::new(Vec::new()));
    // Created here rather than inside the component, so the menu callbacks below
    // can capture them: a signal made outside any render has no owner and lives
    // for the page, which is exactly what a menu built from `start` needs.
    let entries: Signal<String> = Signal::new(String::new());
    let dark_mode: Signal<bool> = Signal::new(true);

    let record = {
        let journal = journal.clone();
        move |what: &str| {
            journal.borrow_mut().push(what.to_string());
            entries.set(journal.borrow().join("\n"));
        }
    };

    let file_menu = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click({
            let record = record.clone();
            move || record("File > New  (Ctrl+N)")
        }))
        .item(MenuItem::new("Open...").shortcut("Ctrl+O").on_click({
            let record = record.clone();
            move || record("File > Open  (Ctrl+O)")
        }))
        .separator()
        .submenu(
            "Open Recent",
            Menu::new()
                .item(MenuItem::new("notes.md").on_click({
                    let record = record.clone();
                    move || record("File > Open Recent > notes.md")
                }))
                .item(MenuItem::new("budget.csv").on_click({
                    let record = record.clone();
                    move || record("File > Open Recent > budget.csv")
                })),
        )
        .separator()
        // A disabled item renders greyed out, fires nothing, and — the part that
        // is easy to get wrong — does not arm its chord either.
        .item(MenuItem::new("Save").shortcut("Ctrl+S").enabled(false));

    let view_menu = Menu::new()
        .item(MenuItem::new("Focus Search").shortcut("Ctrl+K").on_click({
            let record = record.clone();
            move || record("View > Focus Search  (Ctrl+K)")
        }))
        // The bar is styled entirely from theme variables, so this repaints it
        // along with the page.
        .item(MenuItem::new("Toggle Dark Mode").shortcut("Ctrl+D").on_click({
            let record = record.clone();
            move || {
                dark_mode.update(|d| *d = !*d);
                record("View > Toggle Dark Mode  (Ctrl+D)");
            }
        }))
        .separator()
        .item(MenuItem::new("Zoom In").shortcut("Ctrl+=").on_click({
            let record = record.clone();
            move || record("View > Zoom In  (Ctrl+=)")
        }))
        .item(MenuItem::new("Zoom Out").shortcut("Ctrl+-").on_click({
            let record = record.clone();
            move || record("View > Zoom Out  (Ctrl+-)")
        }));

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: true,
        ..Default::default()
    };

    rinch_web::mount_with_menu_bar(
        theme,
        vec![("File", file_menu), ("View", view_menu)],
        move |scope| app(scope, journal, entries, dark_mode),
    );

    log::info!("mounted under the DOM menu bar");
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
