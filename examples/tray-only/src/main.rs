//! A tray-only app (issue #1520).
//!
//! The tray icon is the whole UI until "Settings…" opens the window:
//!
//! - the app starts with **no window on screen** (`WindowProps::visible: false`);
//! - a **left click** on the icon toggles listening (`on_activate`) instead of
//!   showing the window;
//! - while listening, the icon pulses between two images several times a
//!   second (`TrayIcon::set_icon` from a timer) and the tooltip says so;
//! - "Start listening" / "Stop listening" is swapped by replacing the menu
//!   (`TrayIcon::set_menu`);
//! - "Play sounds" is a **checkbox** item and "Fast" / "Accurate" a **radio**
//!   group, each bound to a `Signal` the settings window binds too, so a change
//!   in either place shows in both;
//! - closing the settings window hides it again; "Quit" exits;
//! - "Test notification" raises an operating-system notification
//!   (`rinch::notification`, issue #1522); clicking it opens the settings
//!   window, and what became of it is printed to stdout.
//!
//! Run with `cargo run -p tray-only`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::menu::{Menu, MenuItem};
use rinch::notification::Notification;
use rinch::prelude::*;
use rinch::reactive::Effect;
use rinch::tray::{TrayIcon, TrayIconBuilder, TrayIconImage};

/// What the app is doing, which is what the icon shows.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Idle,
    Listening,
}

/// The app's state, created in `main` (so it has app lifetime) and shared with
/// the settings window.
#[derive(Clone, Copy)]
struct TrayStore {
    mode: Signal<Mode>,
    sounds: Signal<bool>,
    accurate: Signal<bool>,
}

impl TrayStore {
    fn toggle_listening(&self) {
        self.mode.update(|m| {
            *m = match m {
                Mode::Idle => Mode::Listening,
                Mode::Listening => Mode::Idle,
            }
        });
    }
}

thread_local! {
    /// The tray, kept on the main thread (a `TrayIcon` is `!Send`).
    static TRAY: RefCell<Option<Rc<TrayIcon>>> = const { RefCell::new(None) };
    /// The state, made in `main` so the tray built there can use it.
    static STORE: std::cell::Cell<Option<TrayStore>> = const { std::cell::Cell::new(None) };
}

fn tray() -> Option<Rc<TrayIcon>> {
    TRAY.with(|t| t.borrow().clone())
}

/// A filled circle of `rgb`, `radius` pixels, on a transparent 32x32 icon.
fn dot(rgb: [u8; 3], radius: f32) -> TrayIconImage {
    let size = 32u32;
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let alpha = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            px.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8]);
        }
    }
    TrayIconImage::from_rgba(px, size, size).expect("32x32 RGBA")
}

/// The tray menu for the current mode. Rebuilt (and handed to `set_menu`)
/// whenever the mode changes, so its first item reads "Start" or "Stop".
fn tray_menu(store: TrayStore, mode: Mode) -> Menu {
    let start_stop = match mode {
        Mode::Idle => "Start listening",
        Mode::Listening => "Stop listening",
    };
    Menu::new()
        .item(MenuItem::new(start_stop).on_click(move || store.toggle_listening()))
        .separator()
        .item(MenuItem::checkbox("Play sounds", store.sounds))
        .submenu(
            "Speed",
            Menu::new()
                .item(MenuItem::radio("Fast", store.accurate, false))
                .item(MenuItem::radio("Accurate", store.accurate, true)),
        )
        .separator()
        .item(MenuItem::new("Test notification").on_click(notify))
        .item(MenuItem::new("Settings…").on_click(show_current_window))
        .item(MenuItem::new("Quit").on_click(close_current_window))
}

/// Raise a notification; a click on it opens the settings window.
fn notify() {
    let shown = Notification::new("Tray-only")
        .body("Click to open the settings.")
        .on_click(show_current_window)
        .on_dismiss(|why| println!("notification went away: {why:?}"))
        .on_error(|e| println!("notification failed: {e}"))
        .show();
    if let Err(e) = shown {
        println!("notification not shown: {e}");
    }
}

/// The settings window. It is mounted at startup even though nothing is on
/// screen, which is why the tray-driving effects below run from the start.
#[component]
fn app() -> NodeHandle {
    let store = STORE.with(|s| s.get()).expect("made in main");
    let idle = dot([140, 140, 150], 11.0);
    let pulse = [dot([220, 40, 40], 12.0), dot([220, 40, 40], 8.0)];

    // Mode → icon, tooltip and menu.
    let first_pulse = pulse[0].clone();
    Effect::new(move || {
        let mode = store.mode.get();
        let Some(tray) = tray() else { return };
        let _ = tray.set_icon(match mode {
            Mode::Idle => &idle,
            Mode::Listening => &first_pulse,
        });
        let _ = tray.set_tooltip(match mode {
            Mode::Idle => "Idle — click to listen",
            Mode::Listening => "Listening…",
        });
        // The menu holds signals of its own; build it untracked so this
        // effect follows the mode alone.
        rinch::reactive::untracked(|| tray.set_menu(Some(tray_menu(store, mode))));
    });

    // While listening, pulse the icon four times a second.
    fn pulse_tick(store: TrayStore, pulse: [TrayIconImage; 2], frame: usize) {
        set_timeout(250, move || {
            if store.mode.get() == Mode::Listening
                && let Some(tray) = tray()
            {
                let _ = tray.set_icon(&pulse[frame % 2]);
            }
            pulse_tick(store, pulse, frame + 1);
        });
    }
    pulse_tick(store, pulse, 1);

    rsx! {
        div { style: "padding: 24px; display: flex; flex-direction: column; gap: 16px;",
            Title { order: 3, "Settings" }
            Text { color: "dimmed",
                "This window is the only one the app has. Closing it hides it; the tray keeps running."
            }
            Checkbox {
                label: "Play sounds",
                checked_fn: move || store.sounds.get(),
                onchange: move || store.sounds.update(|s| *s = !*s),
            }
            Checkbox {
                label: "Accurate (slower) recognition",
                checked_fn: move || store.accurate.get(),
                onchange: move || store.accurate.update(|a| *a = !*a),
            }
            Text { {|| format!("Mode: {:?}", store.mode.get())} }
            Group {
                Button { onclick: move || store.toggle_listening(),
                    {|| if store.mode.get() == Mode::Listening { "Stop listening" } else { "Start listening" }}
                }
                Button { variant: "light", onclick: hide_current_window, "Hide" }
            }
        }
    }
}

fn main() {
    let store = TrayStore {
        mode: Signal::new(Mode::Idle),
        sounds: Signal::new(true),
        accurate: Signal::new(false),
    };
    STORE.with(|s| s.set(Some(store)));

    let tray = TrayIconBuilder::new()
        .with_tooltip("Idle — click to listen")
        .with_icon(dot([140, 140, 150], 11.0))
        .with_menu(tray_menu(store, Mode::Idle))
        .on_activate(move || store.toggle_listening())
        .build()
        .expect("failed to create the tray icon");
    TRAY.with(|t| *t.borrow_mut() = Some(Rc::new(tray)));

    let props = WindowProps {
        title: "Tray-only settings".into(),
        width: 420,
        height: 320,
        visible: false,
        on_close_requested: Some(std::sync::Arc::new(|| {
            hide_current_window();
            false
        })),
        ..Default::default()
    };
    App::new(app).window_props(props).run();

    // Take the tray down with the app.
    TRAY.with(|t| t.borrow_mut().take());
}
