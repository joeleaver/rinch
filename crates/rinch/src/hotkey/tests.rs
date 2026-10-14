//! The main-thread registry over a fake backend: delivery, conflicts,
//! rebinding, and the #183 lifetime rule.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::reactive::{Scope, Signal};
use winit::keyboard::KeyCode::{AltLeft, ControlLeft, ControlRight, F7, F8, Space};

use super::backend::fake;
use super::*;

type Log = Rc<RefCell<Vec<(String, HotkeyState)>>>;

fn log() -> (Log, impl Fn(&HotkeyEvent) + 'static) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    (log, move |ev: &HotkeyEvent| {
        l.borrow_mut().push((ev.id.clone(), ev.state))
    })
}

#[test]
fn press_and_release_reach_the_callback_and_drop_unregisters() {
    let fk = fake::install();
    let (log, cb) = log();
    let hk = register(HotkeySpec::new("Ctrl+Alt+Space").id("talk"), cb).unwrap();
    assert_eq!(
        hk.binding(),
        Binding::Exact(Shortcut::parse("Ctrl+Alt+Space").unwrap())
    );
    for (k, d) in [
        (ControlLeft, true),
        (AltLeft, true),
        (Space, true),
        (Space, true),
        (Space, false),
    ] {
        fake::key(&fk, k, d);
    }
    assert_eq!(
        *log.borrow(),
        vec![
            ("talk".into(), HotkeyState::Pressed),
            ("talk".into(), HotkeyState::Released)
        ]
    );
    let key = hk.key;
    drop(hk);
    assert_eq!(fk.borrow().unregistered, vec![key]);
    fake::key(&fk, Space, true);
    assert_eq!(log.borrow().len(), 2, "a dropped hotkey fires nothing");
}

#[test]
fn a_callback_can_set_a_signal() {
    let fk = fake::install();
    let held = Signal::new(false);
    let _hk = register("RightCtrl", move |ev| {
        held.set(ev.state == HotkeyState::Pressed)
    })
    .unwrap();
    fake::key(&fk, ControlRight, true);
    assert!(held.get());
    fake::key(&fk, ControlRight, false);
    assert!(!held.get());
}

#[test]
fn conflicts_are_errors_and_name_the_shortcut() {
    let fk = fake::install();
    fk.borrow_mut()
        .taken
        .push(Shortcut::parse("Ctrl+Alt+T").unwrap());
    let err = register("ctrl+alt+t", |_| {}).unwrap_err();
    assert_eq!(err, HotkeyError::Conflict("Ctrl+Alt+T".into()));
    let _a = register("Ctrl+K", |_| {}).unwrap();
    assert!(
        matches!(register("ctrl+k", |_| {}), Err(HotkeyError::Conflict(_))),
        "this app's own"
    );
    assert!(matches!(
        register(HotkeySpec::new("Ctrl+J").id("hotkey-Ctrl+K"), |_| {}),
        Err(HotkeyError::DuplicateId(_))
    ));
    assert!(matches!(
        register("Ctrl+?", |_| {}),
        Err(HotkeyError::Parse(_))
    ));
}

#[test]
fn rebind_moves_the_binding_and_a_failed_rebind_keeps_the_old_one() {
    let fk = fake::install();
    fk.borrow_mut().taken.push(Shortcut::parse("F9").unwrap());
    let (log, cb) = log();
    let hk = register(HotkeySpec::new("F8").id("x"), cb).unwrap();
    hk.rebind("RightCtrl").unwrap();
    assert_eq!(hk.shortcut().unwrap().to_string(), "RightCtrl");
    fake::key(&fk, F8, true);
    fake::key(&fk, F8, false);
    fake::key(&fk, ControlRight, true);
    assert_eq!(log.borrow().len(), 1, "only the new binding fires");

    assert_eq!(hk.rebind("F9"), Err(HotkeyError::Conflict("F9".into())));
    assert_eq!(
        hk.shortcut().unwrap().to_string(),
        "RightCtrl",
        "old binding kept"
    );
    fake::key(&fk, ControlRight, false);
    fake::key(&fk, ControlRight, true);
    assert_eq!(log.borrow().len(), 3, "and it still fires");
}

#[test]
fn a_hotkey_registered_in_a_component_dies_with_it() {
    let fk = fake::install();
    let (log, cb) = log();
    let scope = Scope::new();
    let hk = scope.run(|| register("F7", cb).unwrap());
    fake::key(&fk, F7, true);
    fake::key(&fk, F7, false);
    assert_eq!(log.borrow().len(), 2);
    let key = hk.key;
    scope.dispose();
    assert_eq!(fk.borrow().unregistered, vec![key], "unmount unregisters");
    fake::key(&fk, F7, true);
    assert_eq!(log.borrow().len(), 2, "and the callback is not called");
    drop(hk); // a handle that outlived its component drops quietly
    assert_eq!(fk.borrow().unregistered, vec![key], "no second unregister");
}

#[test]
fn backend_choice_on_linux() {
    use super::backend::linux_choice;
    assert_eq!(linux_choice(None, true, true), Backend::WaylandPortal);
    assert_eq!(linux_choice(None, false, true), Backend::X11);
    assert_eq!(linux_choice(None, false, false), Backend::Unsupported);
    assert_eq!(linux_choice(Some("X11"), true, true), Backend::X11);
    assert_eq!(
        linux_choice(Some("portal"), false, true),
        Backend::WaylandPortal
    );
}

/// `global-hotkeys` adds no package to `Cargo.lock`: every crate its backends
/// use directly is already a dependency of some *other* package in the lock
/// (zbus through ksni, accesskit_unix and rfd's portal; tokio through ksni
/// and ashpd; futures-lite through zbus; x11rb through winit and
/// softbuffer; windows-sys 0.59 through winit and muda). Measured with `cargo tree
/// -e normal`: on Windows and macOS the feature adds tokio and tokio-macros
/// to the tree; on Linux the zbus stack and tokio, two crates more than
/// `file-dialogs` alone. If this fails, a backend started depending on a
/// crate only rinch pulls in: that is a new package
/// for every consumer, which is a decision, not a refactor.
#[test]
fn the_backends_add_no_package_to_the_lock() {
    let lock = include_str!("../../../../Cargo.lock");
    let packages: Vec<&str> = lock.split("[[package]]").skip(1).collect();
    let name_of = |p: &str| {
        p.lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .map(|n| n.trim_end_matches('"').to_string())
    };
    let used_by_another = |dep: &str| {
        packages.iter().any(|p| {
            name_of(p).as_deref() != Some("rinch")
                && p.lines().any(|l| {
                    let l = l.trim().trim_end_matches(',').trim_matches('"');
                    l == dep || l.starts_with(&format!("{dep} "))
                })
        })
    };
    for dep in [
        "zbus",
        "tokio",
        "futures-lite",
        "x11rb",
        "windows-sys 0.59.0",
        "winit",
    ] {
        assert!(used_by_another(dep), "{dep} is in the lock only for rinch");
    }
    // Positive control: a crate only rinch depends on is told apart.
    assert!(!used_by_another("memory-stats"), "the check cannot fail");
}
