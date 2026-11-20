//! Issue #2689: an app can turn the built-in DevTools keys off — F12 (the
//! DevTools window, desktop) and Alt+I (inspect mode) — with
//! `App::devtools(false)` / `RinchApp::set_devtools(false)`, and
//! `RINCH_DEVTOOLS=on|off` overrides the app either way.
//!
//! Off, the global key fallback must not consume either key: no
//! `ToggleDevTools` / `ToggleInspectMode`, and the key goes on to the rest of
//! the fallback (an Alt+I that typed a character is text input in a focused
//! field), as any other unclaimed key does. On (the default) nothing changes.
//!
//! Driven through the real `rsx!` output and the real key path
//! (`handle_event`), the way the desktop loop drives them.

use super::devtools_keys::{
    devtools_resolved, parse_devtools_env, set_devtools_default, set_env_override_for_test,
};
use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::events::{KeyEventData, on_app_key};
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

/// Every test starts from "no `RINCH_DEVTOOLS`" and the built-in default, so
/// the host's environment cannot decide a result; and puts both back after.
struct Clean;

impl Clean {
    fn new() -> Self {
        set_env_override_for_test(None);
        set_devtools_default(true);
        Clean
    }
}

impl Drop for Clean {
    fn drop(&mut self) {
        set_env_override_for_test(None);
        set_devtools_default(true);
    }
}

/// An `<input>` and an app-wide key listener that logs every key it hears
/// and consumes none.
struct Fixture {
    app: RinchApp,
    input: NodeHandle,
    heard: Log,
}

fn mount() -> Fixture {
    let input_slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot = input_slot.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = rsx! {
            div {
                input {
                    style: "width: 200px; height: 30px; font-size: 16px; padding: 0 8px; border: 0",
                    oninput: move |_v: String| {},
                }
            }
        };
        *slot.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let heard: Log = Default::default();
    let h = heard.clone();
    on_app_key(move |k: &KeyEventData| {
        h.borrow_mut().push(format!("{:?}:{}", k.kind, k.key));
        false
    });
    let input = input_slot.borrow_mut().take().expect("input");
    Fixture { app, input, heard }
}

impl Fixture {
    fn down(&mut self, key: KeyCode, text: Option<&str>, alt: bool) -> Vec<AppAction> {
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: Modifiers {
                    alt,
                    ..Modifiers::default()
                },
                repeat: KeyRepeat::Fresh,
            },
            VP,
            1.0,
        )
    }

    fn up(&mut self, key: KeyCode, alt: bool) {
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers: Modifiers {
                    alt,
                    ..Modifiers::default()
                },
            },
            VP,
            1.0,
        );
    }

    /// Press and release; answers the press's actions.
    fn press(&mut self, key: KeyCode, text: Option<&str>, alt: bool) -> Vec<AppAction> {
        let actions = self.down(key, text, alt);
        self.up(key, alt);
        actions
    }

    fn f12(&mut self) -> Vec<AppAction> {
        self.press(KeyCode::F12, None, false)
    }

    fn alt_i(&mut self) -> Vec<AppAction> {
        self.press(KeyCode::KeyI, Some("i"), true)
    }

    fn focus_input(&mut self) {
        let (x, y) = {
            let d = self.app.doc.as_ref().unwrap().borrow();
            let (x, y, w, h) = painted_element_box(&d.tree, self.input.node_id().0);
            (x + w.min(6.0) / 2.0, y + h / 2.0)
        };
        for ev in [
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
        ] {
            self.app.handle_event(ev, VP, 1.0);
        }
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        assert!(matches!(self.app.focus_target, FocusTarget::Input(_)));
    }

    fn value(&self) -> String {
        let d = self.app.doc.as_ref().unwrap().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, self.input.node_id().0)
            .unwrap_or_default()
            .into_owned()
    }

    fn heard(&self) -> Vec<String> {
        std::mem::take(&mut *self.heard.borrow_mut())
    }
}

fn toggles_devtools(actions: &[AppAction]) -> bool {
    actions.contains(&AppAction::ToggleDevTools)
}

fn toggles_inspect(actions: &[AppAction]) -> bool {
    actions.contains(&AppAction::ToggleInspectMode)
}

/// The default is unchanged: F12 toggles DevTools (desktop), Alt+I toggles
/// inspect mode and is not typed into a focused field.
#[test]
fn on_by_default_both_keys_toggle_as_before() {
    let _clean = Clean::new();
    let mut f = mount();
    assert!(f.app.devtools_enabled());
    #[cfg(feature = "windowing")]
    assert!(toggles_devtools(&f.f12()));
    f.focus_input();
    let actions = f.alt_i();
    assert!(toggles_inspect(&actions), "{actions:?}");
    assert_eq!(f.value(), "", "inspect mode took Alt+I: nothing typed");
}

/// The issue's ask, directly: with the keys off, F12 and Alt+I open nothing,
/// an app listener hears both (press and release), and Alt+I goes on to the
/// field's text input instead of being swallowed.
#[test]
fn off_neither_key_toggles_and_both_go_on() {
    let _clean = Clean::new();
    let mut f = mount();
    f.app.set_devtools(false);
    assert!(!f.app.devtools_enabled());

    let actions = f.f12();
    assert!(!toggles_devtools(&actions), "{actions:?}");
    assert!(!toggles_inspect(&actions), "{actions:?}");
    assert_eq!(f.heard(), ["Down:F12", "Up:F12"]);

    f.focus_input();
    let actions = f.alt_i();
    assert!(!toggles_inspect(&actions), "{actions:?}");
    assert!(!toggles_devtools(&actions), "{actions:?}");
    assert_eq!(f.heard(), ["Down:i", "Up:i"]);
    assert_eq!(f.value(), "i", "Alt+I went on to the field's text input");
}

/// Turning it back on restores both keys, in the same app.
#[test]
fn turned_back_on_both_keys_toggle_again() {
    let _clean = Clean::new();
    let mut f = mount();
    f.app.set_devtools(false);
    f.app.set_devtools(true);
    #[cfg(feature = "windowing")]
    assert!(toggles_devtools(&f.f12()));
    assert!(toggles_inspect(&f.alt_i()));
}

/// `App::devtools(false)` reaches the window's app through the thread's
/// default (`set_devtools_default`), as `App::run` sets it before any window
/// exists; an app created after it is turned back on is on.
#[test]
fn the_thread_default_reaches_an_app_created_after_it() {
    let _clean = Clean::new();
    set_devtools_default(false);
    let mut off = mount();
    assert!(!off.app.devtools_enabled());
    assert!(!toggles_inspect(&off.alt_i()));
    #[cfg(feature = "windowing")]
    assert!(!toggles_devtools(&off.f12()));

    set_devtools_default(true);
    let mut on = mount();
    assert!(on.app.devtools_enabled());
    assert!(toggles_inspect(&on.alt_i()));
}

/// `RINCH_DEVTOOLS=on` brings the keys back to an app that turned them off.
#[test]
fn the_environment_turns_them_on_over_the_app() {
    let _clean = Clean::new();
    set_devtools_default(false);
    set_env_override_for_test(Some("on"));
    let mut f = mount();
    f.app.set_devtools(false);
    assert!(f.app.devtools_enabled());
    #[cfg(feature = "windowing")]
    assert!(toggles_devtools(&f.f12()));
    assert!(toggles_inspect(&f.alt_i()));
}

/// `RINCH_DEVTOOLS=off` silences them in an app that left them on.
#[test]
fn the_environment_turns_them_off_over_the_app() {
    let _clean = Clean::new();
    set_env_override_for_test(Some("off"));
    let mut f = mount();
    f.app.set_devtools(true);
    assert!(!f.app.devtools_enabled());
    let actions = f.f12();
    assert!(!toggles_devtools(&actions), "{actions:?}");
    assert!(!toggles_inspect(&f.alt_i()));
    // Control: with the variable gone, the same app answers again.
    set_env_override_for_test(None);
    assert!(toggles_inspect(&f.alt_i()));
}

/// What a `RINCH_DEVTOOLS` value means, and that only on/off values override.
#[test]
fn the_environment_value_parses_on_and_off_and_ignores_the_rest() {
    for on in ["on", "ON", " On ", "1", "true", "yes"] {
        assert_eq!(parse_devtools_env(Some(on)), Some(true), "{on:?}");
    }
    for off in ["off", "OFF", "0", "false", "No"] {
        assert_eq!(parse_devtools_env(Some(off)), Some(false), "{off:?}");
    }
    for none in [None, Some(""), Some("  "), Some("maybe"), Some("2")] {
        assert_eq!(parse_devtools_env(none), None, "{none:?}");
    }
    assert!(devtools_resolved(None, true));
    assert!(!devtools_resolved(None, false));
    assert!(devtools_resolved(Some(true), false));
    assert!(!devtools_resolved(Some(false), true));
}

/// With nothing set, the keys are on in a debug build and off in a release
/// build (the owner's decision on #2689): a fresh thread's default is
/// `cfg!(debug_assertions)`, and so is `App`'s.
#[test]
fn the_default_is_on_in_debug_and_off_in_release() {
    assert_eq!(crate::app::DEVTOOLS_DEFAULT, cfg!(debug_assertions));
    let fresh = std::thread::spawn(super::devtools_keys::thread_default)
        .join()
        .unwrap();
    assert_eq!(fresh, cfg!(debug_assertions));
}
