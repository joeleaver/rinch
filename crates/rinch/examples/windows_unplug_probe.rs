//! A live check of a panel per output when an output goes (#2416 stage 4b),
//! for a nested compositor in an isolated session. The removal is the
//! compositor's own command, given in `UNPLUG_CMD`; `UNPLUG_OUTPUT` names the
//! output it removes:
//!
//! ```text
//! # sway (headless), started inside the session: two outputs, the second
//! # removed. (`swaymsg output … unplug` on sway 1.11 ends the app's
//! # connection after a resolution was set in the session: a wlroots 0.19.2
//! # use-after-free, see #2707. `disable` withdraws the output the same way.)
//! scripts/isolated-session.sh -- sh -c \
//!   'WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway & sleep 1; \
//!   export WAYLAND_DISPLAY=wayland-1 SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.*); \
//!   swaymsg create_output; \
//!   UNPLUG_OUTPUT=HEADLESS-2 UNPLUG_CMD="swaymsg output HEADLESS-2 disable" windows_unplug_probe'
//! # KWin (virtual): two outputs, KScreen's backend started in the session
//! scripts/isolated-session.sh --kwin --output-count 2 -- sh -c \
//!   '/usr/lib/x86_64-linux-gnu/libexec/kf6/kscreen_backend_launcher & sleep 2; \
//!   UNPLUG_OUTPUT=Virtual-1 UNPLUG_CMD="kscreen-doctor output.Virtual-1.disable" windows_unplug_probe'
//! ```
//!
//! Each output gets a layer-surface panel, a `Window` in a keyed `for` over
//! `output_ids()` (#2705: over `outputs()`, a moved or rescaled output's panel
//! was closed and opened again). With `CHANGE_CMD` set, that command (which
//! moves, resizes or rescales outputs) runs first, and every panel must stay
//! the one it was: open, and no other opened. On sway, add to the line above:
//! `CHANGE_CMD="swaymsg output HEADLESS-1 pos 100 0; swaymsg output HEADLESS-2 scale 2"`.
//!
//! With `WINDOWLESS=1` the app is `App::windowless`, and the
//! panels are its only windows. After the removal, the removed output's panel must be closed,
//! the others open, and nothing reopened; the probe prints which came first
//! (the compositor's close, which calls `on_closed`, or `outputs()` dropping
//! the output, which closes it with its row) and exits 0, or 1 at the first
//! failed check.

use std::cell::RefCell;

use rinch::prelude::*;

thread_local! {
    static PANELS: RefCell<Vec<(String, WindowHandle)>> = const { RefCell::new(Vec::new()) };
    static COMPOSITOR_CLOSED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn check(name: &str, ok: bool) {
    if ok {
        println!("PASS {name}");
    } else {
        println!("FAIL {name}");
        std::process::exit(1);
    }
}

fn panel_for(name: &str) -> Option<WindowHandle> {
    PANELS.with(|p| {
        p.borrow()
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, h)| *h)
    })
}

#[component]
fn app() -> NodeHandle {
    rinch_core::reactive::unowned(|| set_timeout(2000, step_change));
    rsx! {
        div {
            // The ids, not the outputs (#2705): a row whose item is the whole
            // `Output` is rendered again when the output moves or is
            // rescaled, closing its panel and opening another.
            for id in output_ids() {
                // Keyed and placed by the output's id, which every output has
                // (#2658): two outputs without a name keyed by
                // `name.unwrap_or_default()` shared one window.
                let name = output(id).and_then(|o| o.name).unwrap_or_default();
                Window {
                    key: id,
                    title: format!("panel {name}"),
                    height: 32,
                    wayland_layer: Some(LayerSurface {
                        layer: LayerShellLayer::Top,
                        anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                        exclusive_zone: LayerExclusiveZone::Reserve(32),
                        output_id: Some(id),
                        ..Default::default()
                    }),
                    on_open: {
                        let name = name.clone();
                        move |h: WindowHandle| PANELS.with(|p| p.borrow_mut().push((name.clone(), h)))
                    },
                    on_closed: {
                        let name = name.clone();
                        move || COMPOSITOR_CLOSED.with(|c| c.borrow_mut().push(name.clone()))
                    },
                    // What the panel shows of its output follows it, read by id.
                    div { {move || output(id).map(|o| format!("{:?} x{}", o.position, o.scale)).unwrap_or_default()} }
                }
            }
        }
    }
}

/// `CHANGE_CMD`: move, resize or rescale outputs, and check every panel is
/// still the one it was (#2705); then the unplug step.
fn step_change() {
    let Ok(cmd) = std::env::var("CHANGE_CMD") else {
        return step_unplug();
    };
    let before = PANELS.with(|p| p.borrow().clone());
    let outputs_before = outputs();
    println!("OUTPUTS BEFORE {outputs_before:?}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status();
    check("the change command ran", status.is_ok_and(|s| s.success()));
    rinch_core::reactive::unowned(|| {
        set_timeout(2000, move || {
            let outputs_after = outputs();
            println!("OUTPUTS AFTER {outputs_after:?}");
            check("the outputs changed", outputs_after != outputs_before);
            check(
                "the same outputs are connected",
                outputs_after.iter().map(|o| o.id).collect::<Vec<_>>()
                    == outputs_before.iter().map(|o| o.id).collect::<Vec<_>>(),
            );
            let after = PANELS.with(|p| p.borrow().clone());
            check("no panel opened by the change", after.len() == before.len());
            check(
                "every panel is still open",
                before.iter().all(|(_, h)| h.is_open()),
            );
            step_unplug();
        })
    });
}

fn step_unplug() {
    let gone = std::env::var("UNPLUG_OUTPUT").expect("UNPLUG_OUTPUT");
    let cmd = std::env::var("UNPLUG_CMD").expect("UNPLUG_CMD");
    let names: Vec<String> = outputs().into_iter().filter_map(|o| o.name).collect();
    println!("OUTPUTS {names:?}");
    check("two outputs or more", names.len() >= 2);
    check("the output to remove is there", names.contains(&gone));
    check(
        "a panel is open on every output",
        names
            .iter()
            .all(|n| panel_for(n).is_some_and(|h| h.is_open())),
    );
    if std::env::var("WINDOWLESS").as_deref() == Ok("1") {
        check(
            "windowless: no main window, and the panels are the app's windows",
            WindowHandle::main().is_none() && windows().len() == names.len(),
        );
    }
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status();
    check("the removal command ran", status.is_ok_and(|s| s.success()));
    rinch_core::reactive::unowned(|| set_timeout(3000, move || step_after(gone, names)));
}

fn step_after(gone: String, names: Vec<String>) {
    let order = if COMPOSITOR_CLOSED.with(|c| c.borrow().contains(&gone)) {
        "the compositor's close came first"
    } else {
        "outputs() dropped the output first"
    };
    println!("ORDER {order}");
    check(
        "the removed output's panel is closed",
        panel_for(&gone).is_some_and(|h| !h.is_open()),
    );
    check(
        "the other panels are open",
        names
            .iter()
            .filter(|n| **n != gone)
            .all(|n| panel_for(n).is_some_and(|h| h.is_open())),
    );
    let open_before = windows().len();
    rinch_core::reactive::unowned(|| {
        set_timeout(2000, move || {
            check("nothing reopened", windows().len() == open_before);
            check(
                "one panel was ever opened for the removed output",
                PANELS.with(|p| p.borrow().iter().filter(|(n, _)| *n == gone).count()) == 1,
            );
            exit();
        })
    });
}

fn main() {
    // `WINDOWLESS=1`: the panels are the app's only windows (#2416 stage 4c).
    if std::env::var("WINDOWLESS").as_deref() == Ok("1") {
        App::windowless(app).run();
    } else {
        App::new(app).title("unplug probe").size(200, 100).run();
    }
}
