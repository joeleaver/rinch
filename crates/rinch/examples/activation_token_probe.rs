//! Probe for issue #2501: a window that starts a program when Enter is
//! pressed in it, with an xdg-activation token or without one.
//!
//! Driven by `tests/activation_token_2501.rs`. Environment:
//!
//! - `PROBE_CHILD`: the program to start and its arguments, separated by
//!   spaces (`foot --app-id child sleep 60`);
//! - `PROBE_CHILD_LOG`: a file for the program's standard error (its
//!   `WAYLAND_DEBUG` log, which says when its window gets the keyboard);
//! - `PROBE_TOKEN=1`: ask for a token in the Enter handler and start the
//!   program in the callback, with it in `XDG_ACTIVATION_TOKEN`; otherwise
//!   start it in the handler, with none.
//!
//! It prints to stderr `PROBE ready` once its window is focusable, `PROBE
//! token ok` or `PROBE token <error>`, and `PROBE started`.

use std::process::{Command, Stdio};

use rinch::prelude::*;

fn start(token: Option<&ActivationToken>) {
    let child = std::env::var("PROBE_CHILD").unwrap_or_default();
    let mut words = child.split_whitespace();
    let Some(program) = words.next() else {
        eprintln!("PROBE no PROBE_CHILD");
        return;
    };
    let mut command = Command::new(program);
    command
        .args(words)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    // The probe's own token, if it was started with one, is not the child's.
    command.env_remove("XDG_ACTIVATION_TOKEN");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token.as_str());
    }
    if let Some(log) = std::env::var_os("PROBE_CHILD_LOG") {
        match std::fs::File::create(log) {
            Ok(file) => {
                command.stderr(file);
            }
            Err(error) => eprintln!("PROBE no child log: {error}"),
        }
    }
    match command.spawn() {
        Ok(_) => eprintln!("PROBE started"),
        Err(error) => eprintln!("PROBE not started: {error}"),
    }
}

#[component]
fn app() -> NodeHandle {
    let with_token = std::env::var("PROBE_TOKEN").is_ok_and(|v| v == "1");
    let root = rsx! {
        div {
            tabindex: "0",
            style: "width: 100%; height: 100%; background: #364;",
            onkeydown: move |k| {
                if k.key != "Enter" {
                    return false;
                }
                if with_token {
                    request_activation_token(|token| {
                        match &token {
                            Ok(_) => eprintln!("PROBE token ok"),
                            Err(error) => eprintln!("PROBE token {error:?}"),
                        }
                        start(token.as_ref().ok());
                    });
                } else {
                    start(None);
                }
                true
            },
            "activation token probe: press Enter"
        }
    };
    let focus = root.clone();
    set_timeout(200, move || {
        focus.focus();
        eprintln!("PROBE ready");
    });
    root
}

fn main() {
    App::new(app)
        .title("activation token probe")
        .size(400, 300)
        .run();
}
