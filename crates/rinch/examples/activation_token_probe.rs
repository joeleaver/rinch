//! Probe for issue #2501: a window that asks for an xdg-activation token
//! when Enter is pressed in it, and hands the token on.
//!
//! Driven by `tests/activation_token_2501/`. Environment:
//!
//! - `PROBE_TOKEN_FILE`: write the token to this file, for a program that
//!   is already running to activate its window with;
//! - `PROBE_CHILD`: start this program (its arguments separated by spaces,
//!   `foot --app-id child sleep 8`) with the token in
//!   `XDG_ACTIVATION_TOKEN`, its standard error in `PROBE_CHILD_LOG`.
//!
//! It prints to stderr `PROBE ready` once its window has the keyboard and
//! its element the focus, then
//! `PROBE token ok` or `PROBE token <error>`, and `PROBE started` when it
//! started the child.

use std::process::{Command, Stdio};

use rinch::prelude::*;

/// Starts `PROBE_CHILD` with `token`.
fn start(token: &ActivationToken) {
    let Ok(child) = std::env::var("PROBE_CHILD") else {
        return;
    };
    let mut words = child.split_whitespace();
    let Some(program) = words.next() else {
        return;
    };
    let mut command = Command::new(program);
    command
        .args(words)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .env("XDG_ACTIVATION_TOKEN", token.as_str());
    if let Some(log) = std::env::var_os("PROBE_CHILD_LOG") {
        match std::fs::File::create(log) {
            Ok(file) => {
                command.stderr(file);
            }
            Err(error) => eprintln!("PROBE not started: no log: {error}"),
        }
    }
    match command.spawn() {
        Ok(_) => eprintln!("PROBE started"),
        Err(error) => eprintln!("PROBE not started: {error}"),
    }
}

/// Hands `token` on: to the file a running program reads, and to a program
/// started with it.
fn hand_on(token: ActivationTokenResult) {
    let token = match token {
        Ok(token) => token,
        Err(error) => {
            eprintln!("PROBE token {error:?}");
            return;
        }
    };
    if let Some(file) = std::env::var_os("PROBE_TOKEN_FILE")
        && let Err(error) = std::fs::write(file, token.as_str())
    {
        eprintln!("PROBE not written: {error}");
        return;
    }
    eprintln!("PROBE token ok");
    start(&token);
}

#[component]
fn app() -> NodeHandle {
    let root = rsx! {
        div {
            tabindex: "0",
            style: "width: 100%; height: 100%; background: #364;",
            onkeydown: move |k| {
                if k.key != "Enter" {
                    return false;
                }
                request_activation_token(hand_on);
                true
            },
            "activation token probe: press Enter"
        }
    };
    // Ready once the element has the focus and the window has the keyboard:
    // an Enter then reaches the handler.
    let element_focused = Signal::new(false);
    let focus = root.clone();
    set_timeout(0, move || {
        focus.focus();
        element_focused.set(true);
    });
    rinch::reactive::Effect::new(move || {
        if element_focused.get() && rinch::window::window_focused() {
            eprintln!("PROBE ready");
        }
    });
    root
}

fn main() {
    App::new(app)
        .title("activation token probe")
        .size(400, 300)
        .run();
}
