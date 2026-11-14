//! A live check of how the app ends when its display goes away (#2531) and
//! what a window destroyed from outside does (#2489), for an isolated session:
//!
//! ```text
//! cargo build -p rinch --example display_loss_probe
//! scripts/isolated-session.sh -- sh -c '
//!   kwin_wayland --virtual --socket wayland-9 & kwin=$!; sleep 3
//!   WAYLAND_DISPLAY=wayland-9 RINCH_RENDERER=software \
//!     target/debug/examples/display_loss_probe & app=$!
//!   sleep 4; kill $kwin; wait $app; echo "exit $?"'
//! ```
//!
//! The abort #2531 saw once is a closed stderr: start the app with
//! `2> >(cat >/dev/null)`, kill that `cat` after a few seconds, then the
//! compositor. It must still end with `RUN ENDED` (it was SIGABRT, 134).
//!
//! On X11 (`scripts/isolated-session.sh --kwin --xwayland`, `WAYLAND_DISPLAY`
//! unset): `xdotool windowclose "$(xdotool search --name 'loss probe second')"`.
//!
//! It opens a main window ("loss probe main") and a second one ("loss probe
//! second"), then prints a line every half second saying which are open.
//!
//! - Kill the compositor: the app must end with no panic, `App::try_run`
//!   answering `DisplayLost`, which it prints as `RUN ENDED: …`, and exit with
//!   status 3. (`xdotool windowkill` on X11 kills the X connection instead, and
//!   Xlib exits the process with status 1 before rinch hears of it.)
//! - Destroy the second window from outside (`xdotool windowclose`, an
//!   `XDestroyWindow`): `second=false`, and the app runs on.
//! - Destroy the main window the same way: the app ends as when the main
//!   window closes (`RUN ENDED: ok`, status 0).

use rinch::prelude::*;
use rinch_core::reactive::unowned;

fn props(title: &str) -> WindowProps {
    WindowProps {
        title: title.into(),
        width: 300,
        height: 200,
        ..Default::default()
    }
}

fn second(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.append_child(&scope.create_text("second"));
    root
}

fn beat(n: u32, second: WindowHandle) {
    let main = WindowHandle::main().is_some_and(|m| m.is_open());
    println!("ALIVE {n} main={main} second={}", second.is_open());
    unowned(|| set_timeout(500, move || beat(n + 1, second)));
}

#[component]
fn app() -> NodeHandle {
    unowned(|| {
        set_timeout(300, || {
            let second = open_window(props("loss probe second"), second);
            beat(0, second);
        })
    });
    rsx! { div { "main" } }
}

fn main() {
    let result = App::new(app)
        .title("loss probe main")
        .size(300, 200)
        .try_run();
    match result {
        Ok(()) => println!("RUN ENDED: ok"),
        Err(e) => {
            println!("RUN ENDED: {e}");
            std::process::exit(if e.is_display_lost() { 3 } else { 1 });
        }
    }
}
