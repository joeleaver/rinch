//! Issue #2817 on a live compositor: a running rinch window raised with the
//! token a second launch was started with.
//!
//! The probe (`examples/activation_token_probe.rs` with
//! `PROBE_ACTIVATE_FILE`) is the running copy, mapped first. The waiter maps
//! over it and takes the keyboard, as the program the user clicks a link in
//! does. Enter is pressed into the waiter, which asks the compositor for a
//! token naming that press, as a launcher does, and starts the second launch
//! with it in `XDG_ACTIVATION_TOKEN`: the probe again, with `PROBE_RELAY_TO`,
//! which takes the token (`take_launch_activation_token`), checks the
//! variable is gone, and hands the token to the running copy through a file.
//! The running copy calls `activate_current_window_with_token` with it.
//!
//! The two controls are what an app could do before: `focus_current_window`
//! with no token, and a token not tied to any input (the waiter's own,
//! asked with no serial). Neither takes the keyboard from the waiter.
//!
//! Run as the rest of this binary is (see `main.rs`).

use super::*;

/// The probe's `PROBE focus <bool>` lines: whether its window gets the
/// keyboard (`true`) within `limit`. Lines for losing it are passed over.
fn probe_focused_within(probe: &Probe, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match probe.lines.recv_timeout(left) {
            Ok(line) if line == "PROBE focus true" => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

/// Waits for the probe to say its window lost the keyboard.
fn probe_unfocused(probe: &Probe) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match probe.lines.recv_timeout(left) {
            Ok(line) if line == "PROBE focus false" => return,
            Ok(_) => {}
            Err(_) => panic!("the probe never lost the keyboard to the waiter"),
        }
    }
}

/// The running probe, behind the waiter, which has the keyboard and has
/// been pressed Enter in: the waiter, the probe, the serial of that press,
/// and the keyboard (kept: on a headless sway it is the seat's only one).
fn probe_behind_the_waiter(file: &Path) -> (waiter::Waiter, Probe, u32, keys::Keyboard) {
    let runtime = runtime_dir();
    let mut keyboard = keys::Keyboard::new(&socket(), &runtime);
    let _ = std::fs::remove_file(file);
    let probe = Probe::start(&[("PROBE_ACTIVATE_FILE", file)]);
    let waiter = waiter::Waiter::start(&socket(), &runtime);
    assert!(
        wait_until(PATIENCE, || waiter.mapped() && waiter.focused()),
        "the waiter's new window took the keyboard"
    );
    probe_unfocused(&probe);
    keyboard.press_enter();
    let serial = waiter
        .wait_for_key(PATIENCE)
        .expect("Enter reached the waiter");
    (waiter, probe, serial, keyboard)
}

/// The second launch: the probe started with `token` in
/// `XDG_ACTIVATION_TOKEN`, which hands it on to `file` and exits.
fn second_launch(token: &str, file: &Path) {
    let output = Command::new(probe_binary())
        .env("WAYLAND_DISPLAY", display())
        .env("XDG_ACTIVATION_TOKEN", token)
        .env("PROBE_RELAY_TO", file)
        .stdout(Stdio::null())
        .output()
        .expect("run the second launch");
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && said.contains("PROBE relayed"),
        "the second launch did not hand the token on: {said}"
    );
}

/// Hands `what` (a token, or `focus`) straight to the running probe.
fn tell_probe(what: &str, file: &Path) {
    let partial = file.with_extension("partial");
    std::fs::write(&partial, what).expect("write the request");
    std::fs::rename(&partial, file).expect("hand the request over");
}

#[test]
#[ignore = "needs a compositor: run inside scripts/isolated-session.sh"]
fn a_running_window_given_the_second_launchs_token_is_raised_2817() {
    let file = session_dir().join("activate-2817");
    let (waiter, probe, serial, _keyboard) = probe_behind_the_waiter(&file);
    let token = waiter.token(Some(serial), PATIENCE);
    second_launch(&token, &file);
    probe.wait_for("PROBE activating");
    assert!(
        probe_focused_within(&probe, PATIENCE),
        "activated with the second launch's token, the probe did not get the keyboard"
    );
    assert!(
        wait_until(PATIENCE, || !waiter.focused()),
        "the waiter kept the keyboard"
    );
}

#[test]
#[ignore = "needs a compositor: run inside scripts/isolated-session.sh"]
fn a_running_window_focused_without_a_token_is_not_raised_2817() {
    let file = session_dir().join("activate-2817");
    let (waiter, probe, _serial, _keyboard) = probe_behind_the_waiter(&file);
    tell_probe("focus", &file);
    probe.wait_for("PROBE activating");
    assert!(
        !probe_focused_within(&probe, QUIET),
        "focus_current_window with no token took the keyboard"
    );
    assert!(waiter.focused());
}

#[test]
#[ignore = "needs a compositor: run inside scripts/isolated-session.sh"]
fn a_running_window_given_a_token_tied_to_no_input_is_not_raised_2817() {
    let file = session_dir().join("activate-2817");
    let (waiter, probe, _serial, _keyboard) = probe_behind_the_waiter(&file);
    let token = waiter.token(None, PATIENCE);
    tell_probe(&token, &file);
    probe.wait_for("PROBE activating");
    assert!(
        !probe_focused_within(&probe, QUIET),
        "a token tied to no input took the keyboard"
    );
    assert!(waiter.focused());
}
