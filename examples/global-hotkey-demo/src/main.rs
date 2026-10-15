//! System-wide hotkeys (`rinch::hotkey`, the `global-hotkeys` feature).
//!
//! Registers `Ctrl+Alt+Space` and logs every press and release, including
//! while another application has the focus. "Rebind" captures the next keys
//! pressed in this window — a chord, or a modifier alone such as Right Ctrl —
//! and rebinds the hotkey to them. Run with
//! `cargo run -p global-hotkey-demo`, focus another window, and press the hotkey.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::core::{KeyEventData, clear_keyboard_interceptor, set_keyboard_interceptor};
use rinch::hotkey::{
    self, Binding, CaptureStep, HotkeyCapture, HotkeySpec, HotkeyState, Permission,
};
use rinch::prelude::*;

const START: &str = "Ctrl+Alt+Space";

#[component]
fn app() -> NodeHandle {
    let log = Signal::new(Vec::<String>::new());
    let binding = Signal::new(String::new());
    let status = Signal::new(String::new());
    let capturing = Signal::new(false);

    let push = move |line: String| {
        log.update(|l| {
            l.insert(0, line);
            l.truncate(40);
        })
    };
    let describe = move |b: &Binding| match b {
        Binding::Exact(sc) => sc.to_string(),
        Binding::SystemChosen { description } => {
            format!(
                "chosen by the system: {}",
                description.as_deref().unwrap_or("(no description)")
            )
        }
        Binding::Pending => "waiting for the system dialog…".into(),
        Binding::Declined => "declined".into(),
        _ => format!("{b:?}"),
    };

    let spec = HotkeySpec::new(START)
        .id("example-hotkey")
        .description("Log a press and a release");
    let hk = match hotkey::register(spec, move |ev| {
        let what = match ev.state {
            HotkeyState::Pressed => "pressed",
            HotkeyState::Released => "released",
        };
        let chord = if ev.chorded { " (chorded: another key went down)" } else { "" };
        push(format!("{} {what}{chord}", ev.id));
    }) {
        Ok(hk) => {
            binding.set(describe(&hk.binding()));
            hk.on_binding_change(move |b| binding.set(describe(b)));
            Some(hk)
        }
        Err(e) => {
            status.set(format!("could not register {START}: {e}"));
            None
        }
    };
    status.update(|s| {
        if s.is_empty() {
            *s = format!("backend: {:?}", hotkey::backend());
        }
    });
    let hk = Rc::new(RefCell::new(hk));

    let start_capture = Rc::new({
        let hk = hk.clone();
        move || {
            if hk.borrow().is_none() {
                return;
            }
            capturing.set(true);
            status.set("press the new hotkey (Escape cancels)".into());
            let capture = RefCell::new(HotkeyCapture::new());
            let hk = hk.clone();
            set_keyboard_interceptor(move |k: &KeyEventData| {
                match capture.borrow_mut().feed(k) {
                    CaptureStep::Pending(held) => {
                        status.set(format!("press the new hotkey (Escape cancels): {held}"));
                    }
                    CaptureStep::Cancelled => {
                        capturing.set(false);
                        status.set("rebind cancelled".into());
                        clear_keyboard_interceptor();
                    }
                    CaptureStep::Done(sc) => {
                        capturing.set(false);
                        clear_keyboard_interceptor();
                        if let Some(hk) = hk.borrow().as_ref() {
                            match hk.rebind(&sc.to_string()) {
                                Ok(()) => {
                                    binding.set(describe(&hk.binding()));
                                    status.set(format!("rebound to {sc}"));
                                }
                                Err(e) => status.set(format!("{e}")),
                            }
                        }
                        if hotkey::permission() == Permission::Missing {
                            hotkey::request_permission();
                        }
                    }
                }
                true // the capture takes every key while it runs
            });
        }
    });

    rsx! {
        div { style: "padding: 24px; display: flex; flex-direction: column; gap: 12px; font-family: sans-serif;",
            h2 { "Global hotkey" }
            p { "Bound to: " strong { {|| binding.get()} } }
            p { {|| status.get()} }
            div {
                Button {
                    onclick: { let start_capture = start_capture.clone(); move || start_capture() },
                    disabled: {|| capturing.get()},
                    "Rebind"
                }
            }
            div { style: "font-family: monospace; white-space: pre; border: 1px solid #ccc; padding: 8px; min-height: 200px;",
                {|| log.get().join("\n")}
            }
        }
    }
}

fn main() {
    App::new(app).title("Global hotkey").size(520, 520).run();
}
