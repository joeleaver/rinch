//! Probe for issue #2731: does a key typed the moment a window has the
//! keyboard reach the field the app focused?
//!
//! Driven by `tests/focus_race_2731/`. `PROBE_MODE`:
//!
//! - `start` (the default): a window shown at start whose `<input>` is
//!   focused while the component is built — the issue's labwc report.
//! - `shortcut`: the `<input>` is in a hidden overlay that the key `o` opens,
//!   moving the focus into it with `focus_into`, as `Modal` does; keys typed
//!   right after the `o`, in the same burst, should land in it.
//!
//! It prints to stderr, one line each:
//!
//! - `PROBE window-focused active=<field|none>` when `window_focused()`
//!   turns true, with whether the field was the active element at that
//!   moment, then `PROBE ready`;
//! - `PROBE field-onfocus` when the field's `focus` event fires (written as
//!   its `data-onfocus` attribute, so the probe builds before #2731 too,
//!   where it never fires);
//! - `PROBE key <key>` for every key press the window was given, before the
//!   field's own handling (an app key listener);
//! - `PROBE value <text>` for each change of the field's value.

use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::dom::FocusIntoPolicy;

#[component]
fn app() -> NodeHandle {
    let shortcut = std::env::var("PROBE_MODE").is_ok_and(|m| m == "shortcut");
    let open = Signal::new(!shortcut);
    let root = rsx! {
        div {
            style: "width: 100%; height: 100%; background: #246;",
            div {
                style: {move || if open.get() { "display: block" } else { "display: none" }},
                input {
                    style: "display: block; width: 300px; height: 30px; font-size: 20px",
                    oninput: move |v: String| eprintln!("PROBE value {v}"),
                }
            }
        }
    };
    let overlay = root.children()[0].clone();
    let field = overlay.children()[0].clone();
    let id = rinch_core::register_handler(Rc::new(|| eprintln!("PROBE field-onfocus")));
    field.set_attribute("data-onfocus", &id.0.to_string());
    if !shortcut {
        field.focus();
    }

    let probe = field.clone();
    rinch::reactive::Effect::new(move || {
        if rinch::window::window_focused() {
            let active = probe
                .active_element()
                .is_some_and(|n| n.node_id() == probe.node_id());
            eprintln!(
                "PROBE window-focused active={}",
                if active { "field" } else { "none" }
            );
            eprintln!("PROBE ready");
        }
    });

    // Lives as long as the app: dropping the handle removes nothing.
    let _ = rinch_core::on_app_key(move |k| {
        if !k.is_down() {
            return false;
        }
        eprintln!("PROBE key {}", k.key);
        if shortcut && k.key == "o" && !open.get() {
            open.set(true);
            overlay.focus_into(FocusIntoPolicy::FirstFocusable);
            return true;
        }
        false
    });
    root
}

fn main() {
    App::new(app).title("focus race probe").size(400, 200).run();
}
