//! Review fixtures for PR #1530.
use std::cell::RefCell;
use std::rc::Rc;

use winit::keyboard::KeyCode::{self, *};

use super::backend::fake;
use super::matcher::Matcher;
use super::*;

fn run(m: &mut Matcher, keys: &[(KeyCode, bool)]) -> Vec<(u64, HotkeyState)> {
    let mut out = Vec::new();
    for &(k, d) in keys {
        m.key(k, d, &mut out);
    }
    out
}

/// R1: the natural Ctrl+C order (Right Ctrl first) DOES fire a "RightCtrl"
/// modifier-only binding (documented in the guide; the matcher test's
/// comment "Right Ctrl as part of a chord press nothing" only covers C-first).
#[test]
fn r1_right_ctrl_then_c_fires_the_modifier_only_binding() {
    let mut m = Matcher::default();
    m.add(7, Shortcut::parse("RightCtrl").unwrap());
    let out = run(
        &mut m,
        &[(ControlRight, true), (KeyC, true), (KeyC, false), (ControlRight, false)],
    );
    assert_eq!(out, vec![(7, HotkeyState::Pressed), (7, HotkeyState::Released)]);
}

/// R2: a modifier key-up the stream lost while NO binding was pressed (Alt+Tab
/// into an elevated window on Windows; secure input on macOS) stays in `down`
/// and blocks every exact-modifier chord without that modifier until the user
/// presses and releases that key again. The Windows poll only runs while a
/// binding is pressed, so it never clears this.
#[test]
fn r2_a_lost_modifier_up_with_nothing_pressed_blocks_other_chords() {
    let mut m = Matcher::default();
    m.add(1, Shortcut::parse("Ctrl+Space").unwrap());
    // Alt+Tab; the Alt key-up is lost.
    run(&mut m, &[(AltLeft, true), (Tab, true), (Tab, false)]);
    assert!(!m.any_pressed(), "nothing pressed: the Windows poll is off");
    let out = run(&mut m, &[(ControlLeft, true), (Space, true), (Space, false), (ControlLeft, false)]);
    assert_eq!(out, vec![(1, HotkeyState::Pressed), (1, HotkeyState::Released)], "stale Alt blocks Ctrl+Space");
}

/// R3: a press the backend reported before a rebind, still queued on the
/// main-thread queue when rebind runs, is delivered after the rebind to the
/// same key: the hotkey reads pressed and no release will ever come (the
/// backend's matcher forgot the old binding).
#[test]
fn r3_a_press_queued_before_rebind_is_not_stranded() {
    let _fk = fake::install();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let hk = register(HotkeySpec::new("F8").id("x"), move |e| l.borrow_mut().push(e.state)).unwrap();
    // The backend thread saw F8 go down and queued the report…
    report(hk.key, Report::State(HotkeyState::Pressed));
    // …and the app rebinds before the main loop drained it.
    hk.rebind("F9").unwrap();
    rinch_core::reactive::drain_main_callbacks();
    let pressed = REGISTRY.with(|r| r.borrow().get(&hk.key).unwrap().pressed);
    assert!(!pressed, "log {:?}: the hotkey is stuck pressed after a rebind", log.borrow());
}

/// R4: report() really goes through the main-thread queue (not inline).
#[test]
fn r4_report_is_delivered_by_the_main_queue_not_inline() {
    let _fk = fake::install();
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let hk = register("F7", move |e| l.borrow_mut().push(e.state)).unwrap();
    report(hk.key, Report::State(HotkeyState::Pressed));
    assert!(log.borrow().is_empty(), "inline");
    rinch_core::reactive::drain_main_callbacks();
    assert_eq!(*log.borrow(), vec![HotkeyState::Pressed]);
}

/// R5 (macOS): the tap models Caps Lock (no flag bit in `flag_bit`) by
/// toggling, and macOS sends one flagsChanged per lock-state change, so while
/// Caps Lock is ON the matcher believes a non-modifier is held, and every
/// modifier-only binding is refused ("another key held").
#[test]
fn r5_caps_lock_on_blocks_modifier_only_bindings() {
    let mut m = Matcher::default();
    m.add(7, Shortcut::parse("RightCtrl").unwrap());
    run(&mut m, &[(CapsLock, true)]); // macOS: Caps Lock turned on (one event)
    let out = run(&mut m, &[(ControlRight, true), (ControlRight, false)]);
    assert_eq!(out, vec![(7, HotkeyState::Pressed), (7, HotkeyState::Released)]);
}
