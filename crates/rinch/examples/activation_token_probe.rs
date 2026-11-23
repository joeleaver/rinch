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
//!
//! For issue #2817 (`tests/activation_token_2501/raise_2817.rs`), the other direction:
//!
//! - `PROBE_ACTIVATE_FILE`: the running copy. It watches this file; when it
//!   appears it is read and removed, and the window is activated with the
//!   token in it (`activate_current_window_with_token`), or, for the word
//!   `focus`, focused without one (`focus_current_window`), or, for the
//!   word `hide`, hidden (`hide_current_window`). It prints
//!   `PROBE activating`, and `PROBE focus true` / `PROBE focus false` each
//!   time its window gains or loses the keyboard.
//! - `PROBE_APP_TOKEN`: a first launch. It takes the token it was started
//!   with and gives it to its window (`App::activation_token`).
//! - `PROBE_RELAY_TO`: the second launch. It opens no window: it takes the
//!   token it was started with (`take_launch_activation_token`), checks that
//!   `XDG_ACTIVATION_TOKEN` is gone from its environment, writes the token to
//!   this file (by a rename, so the watcher never reads half of it) and
//!   prints `PROBE relayed`, or `PROBE not relayed: <why>` and exits with 1.

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
    watch_for_activation();
    root
}

/// The running copy's watch on `PROBE_ACTIVATE_FILE`, and its focus lines.
fn watch_for_activation() {
    let Some(file) = std::env::var_os("PROBE_ACTIVATE_FILE") else {
        return;
    };
    let file = std::path::PathBuf::from(file);
    let _ = set_interval(20, move || {
        let Ok(text) = std::fs::read_to_string(&file) else {
            return;
        };
        let _ = std::fs::remove_file(&file);
        if text == "hide" {
            hide_current_window();
            return;
        }
        eprintln!("PROBE activating");
        if text == "focus" {
            focus_current_window();
        } else {
            activate_current_window_with_token(&text);
        }
    });
    rinch::reactive::Effect::new(move || {
        eprintln!("PROBE focus {}", rinch::window::window_focused());
    });
}

/// The second launch: hands the token it was started with to `file`.
fn relay(file: std::ffi::OsString) -> std::process::ExitCode {
    // SAFETY: first thing in main; nothing else has started a thread.
    let token = unsafe { take_launch_activation_token() };
    let refuse = |why: &str| {
        eprintln!("PROBE not relayed: {why}");
        std::process::ExitCode::FAILURE
    };
    let Some(token) = token else {
        return refuse("started with no XDG_ACTIVATION_TOKEN");
    };
    if std::env::var_os("XDG_ACTIVATION_TOKEN").is_some() {
        return refuse("XDG_ACTIVATION_TOKEN is still in the environment");
    }
    let partial = std::path::PathBuf::from(format!("{}.partial", file.to_string_lossy()));
    if let Err(error) =
        std::fs::write(&partial, &token).and_then(|()| std::fs::rename(&partial, &file))
    {
        return refuse(&error.to_string());
    }
    eprintln!("PROBE relayed");
    std::process::ExitCode::SUCCESS
}

fn main() -> std::process::ExitCode {
    if let Some(file) = std::env::var_os("PROBE_RELAY_TO") {
        return relay(file);
    }
    let token = if std::env::var_os("PROBE_APP_TOKEN").is_some() {
        // SAFETY: first thing in main; nothing else has started a thread.
        unsafe { take_launch_activation_token() }
    } else {
        None
    };
    App::new(app)
        .activation_token(token)
        .title("activation token probe")
        .size(400, 300)
        .run();
    std::process::ExitCode::SUCCESS
}
