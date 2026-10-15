//! Operating-system notifications (issue #1522).
//!
//! `cargo run -p notifications`, then press "Notify". A notification appears
//! (KDE/GNOME on Linux, a toast on Windows; on macOS run it from an `.app`
//! bundle, see `docs/src/guide/platform.md#notifications`). Click it: the log
//! says "clicked" and the window comes forward. Let one expire, dismiss one,
//! or press "Close the last one" while it is up: the log says why it went.
//! With no notification server the log shows the error instead.

use rinch::notification::{Notification, NotificationHandle};
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let log = Signal::new(Vec::<String>::new());
    let last = Signal::new(None::<NotificationHandle>);
    let count = Signal::new(0u32);
    let say = move |line: String| log.update(|l| l.push(line));

    let notify = move || {
        count.update(|n| *n += 1);
        let n = count.get();
        let result = Notification::new(format!("Notification {n}"))
            .body("Click me, dismiss me, or let me expire.")
            .on_click(move || {
                say(format!("{n}: clicked"));
                show_current_window();
            })
            .on_dismiss(move |why| say(format!("{n}: went away ({why:?})")))
            .on_error(move |e| say(format!("{n}: error: {e}")))
            .show();
        match result {
            Ok(handle) => {
                last.set(Some(handle));
                say(format!("{n}: shown"));
            }
            Err(e) => say(format!("{n}: not shown: {e}")),
        }
    };

    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 12px; padding: 20px; \
                    font-family: sans-serif; font-size: 14px;",
            div {
                style: "display: flex; gap: 8px;",
                button { onclick: notify, "Notify" }
                button {
                    onclick: move || {
                        if let Some(h) = last.get() {
                            h.close();
                        }
                    },
                    "Close the last one"
                }
            }
            div {
                style: "display: flex; flex-direction: column; gap: 4px;",
                for line in log.get() {
                    div { {line.clone()} }
                }
            }
        }
    }
}

fn main() {
    App::new(app).title("Notifications").size(480, 360).run();
}
