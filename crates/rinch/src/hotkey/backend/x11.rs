//! X11: XInput2 raw key events for press/release of every binding, and
//! `XGrabKey` to reserve a plain chord.
//!
//! - One private connection, shared: the main thread grabs and ungrabs on it
//!   (synchronously, so a `BadAccess` — another client holds the chord — comes
//!   back as [`HotkeyError::Conflict`] from `register`), and a reader thread
//!   blocks in `wait_for_event` on it. `RustConnection` is thread-safe for
//!   exactly that split.
//! - The reader selects `XI_RawKeyPress`/`XI_RawKeyRelease` on the root
//!   window: every key on the system, grabbed or not (XI 2.1+ delivers raw
//!   events regardless of grabs), with no auto-repeat releases. They feed the
//!   shared [`Matcher`], so sided and modifier-only bindings work and are not
//!   swallowed; a plain chord is also grabbed, which swallows it from the
//!   focused window, as a hot key should. While our own grab is active the
//!   server sends core `KeyPress`/`KeyRelease` instead, which feed the same
//!   matcher.
//! - Keys are X keycodes, which are evdev codes + 8 — the physical key, read
//!   through winit's scancode table, never a keysym (a shortcut names a key).
//!
//! In a Wayland session this backend sees only keys typed into XWayland
//! windows; `choose` picks the portal there.

use std::sync::{Arc, Mutex};

use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::scancode::PhysicalKeyExtScancode;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _, GrabMode, ModMask};
use x11rb::protocol::{ErrorKind, Event};
use x11rb::rust_connection::RustConnection;

use super::BackendImpl;
use crate::hotkey::matcher::{BindingKey, Matcher};
use crate::hotkey::{
    Backend, Binding, HotkeyError, Meta, Modifier, Report, Shortcut, Trigger, report,
};

/// Lock modifiers a grab must ignore: Caps Lock (`Lock`) and Num Lock
/// (`Mod2` on every common keymap). One grab per combination.
const IGNORED: [u16; 4] = [0, LOCK, MOD2, LOCK | MOD2];
const LOCK: u16 = 1 << 1;
const MOD2: u16 = 1 << 4;

pub(crate) struct X11Backend {
    conn: Arc<RustConnection>,
    root: xproto::Window,
    matcher: Arc<Mutex<Matcher>>,
    /// The grabs each binding holds: (keycode, modifier mask).
    grabs: Vec<(BindingKey, u8, u16)>,
}

impl X11Backend {
    pub(crate) fn start() -> Result<Self, HotkeyError> {
        let unsupported = |e: &dyn std::fmt::Display| HotkeyError::Unsupported(format!("X11: {e}"));
        let (conn, screen) = RustConnection::connect(None).map_err(|e| unsupported(&e))?;
        let root = conn.setup().roots[screen].root;
        let version = conn
            .xinput_xi_query_version(2, 2)
            .map_err(|e| unsupported(&e))?
            .reply()
            .map_err(|e| unsupported(&e))?;
        if (version.major_version, version.minor_version) < (2, 1) {
            return Err(HotkeyError::Unsupported(format!(
                "X11: XInput {}.{} has no raw events for grabbed keys (2.1 needed)",
                version.major_version, version.minor_version
            )));
        }
        conn.xinput_xi_select_events(
            root,
            &[xinput::EventMask {
                deviceid: 1, // XIAllMasterDevices
                mask: vec![
                    xinput::XIEventMask::RAW_KEY_PRESS | xinput::XIEventMask::RAW_KEY_RELEASE,
                ],
            }],
        )
        .map_err(|e| unsupported(&e))?
        .check()
        .map_err(|e| unsupported(&e))?;
        let conn = Arc::new(conn);
        let matcher = Arc::new(Mutex::new(Matcher::default()));
        let (c, m) = (conn.clone(), matcher.clone());
        std::thread::Builder::new()
            .name("rinch-hotkey-x11".into())
            .spawn(move || reader(&c, &m))
            .map_err(|e| HotkeyError::Platform(e.to_string()))?;
        Ok(X11Backend {
            conn,
            root,
            matcher,
            grabs: Vec::new(),
        })
    }

    fn grab(&mut self, key: BindingKey, sc: &Shortcut) -> Result<(), HotkeyError> {
        let Trigger::Key(code) = sc.trigger() else {
            return Ok(());
        };
        let Some(keycode) = x_keycode(code) else {
            return Err(HotkeyError::Unsupported(format!(
                "X11 has no keycode for {sc}"
            )));
        };
        let mut mods: u16 = 0;
        for (m, _) in sc.held_modifiers() {
            mods |= u16::from(match m {
                Modifier::Ctrl => ModMask::CONTROL,
                Modifier::Shift => ModMask::SHIFT,
                Modifier::Alt => ModMask::M1,
                Modifier::Super => ModMask::M4,
            });
        }
        for extra in IGNORED {
            let mask = mods | extra;
            let res = self
                .conn
                .grab_key(
                    false,
                    self.root,
                    ModMask::from(mask),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(ReplyError::from)
                .and_then(|c| c.check());
            match res {
                Ok(()) => self.grabs.push((key, keycode, mask)),
                Err(e) => {
                    self.ungrab(key);
                    return Err(match e {
                        ReplyError::X11Error(x) if x.error_kind == ErrorKind::Access => {
                            HotkeyError::Conflict(sc.to_string())
                        }
                        other => HotkeyError::Platform(format!("XGrabKey: {other}")),
                    });
                }
            }
        }
        Ok(())
    }

    fn ungrab(&mut self, key: BindingKey) {
        let conn = &self.conn;
        let root = self.root;
        self.grabs.retain(|&(k, keycode, mask)| {
            if k == key {
                if let Ok(c) = conn.ungrab_key(keycode, root, ModMask::from(mask)) {
                    c.ignore_error();
                }
                false
            } else {
                true
            }
        });
        let _ = self.conn.flush();
    }
}

/// The X keycode of a physical key: its evdev code + 8.
pub(crate) fn x_keycode(code: KeyCode) -> Option<u8> {
    let evdev = PhysicalKey::Code(code).to_scancode()?;
    u8::try_from(evdev + 8).ok()
}

/// The physical key of an X keycode.
pub(crate) fn key_of_x_keycode(keycode: u32) -> Option<KeyCode> {
    match PhysicalKey::from_scancode(keycode.checked_sub(8)?) {
        PhysicalKey::Code(c) => Some(c),
        PhysicalKey::Unidentified(_) => None,
    }
}

fn reader(conn: &RustConnection, matcher: &Mutex<Matcher>) {
    let mut out = Vec::new();
    loop {
        let event = match conn.wait_for_event() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("global hotkeys: X11 connection lost ({e}); hotkeys stop");
                let mut m = matcher.lock().unwrap_or_else(|p| p.into_inner());
                m.release_all(&mut out);
                drop(m);
                for (k, s) in out.drain(..) {
                    report(k, Report::State(s));
                }
                return;
            }
        };
        let (detail, down) = match &event {
            Event::XinputRawKeyPress(e) => (e.detail, true),
            Event::XinputRawKeyRelease(e) => (e.detail, false),
            // While one of *our* grabs is active the server sends this client
            // core events instead of raw ones (measured on Xwayland 24): the
            // grabbed key and every key until it is released. The matcher
            // ignores the duplicate where a server sends both.
            Event::KeyPress(e) => (u32::from(e.detail), true),
            Event::KeyRelease(e) => (u32::from(e.detail), false),
            _ => continue,
        };
        let Some(code) = key_of_x_keycode(detail) else {
            continue;
        };
        matcher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .key(code, down, &mut out);
        for (k, s) in out.drain(..) {
            report(k, Report::State(s));
        }
    }
}

impl BackendImpl for X11Backend {
    fn kind(&self) -> Backend {
        Backend::X11
    }

    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        _meta: &Meta,
    ) -> Result<Binding, HotkeyError> {
        let Some(sc) = shortcut else {
            return Err(HotkeyError::Unsupported(
                "X11 has no system dialog to choose a binding; give a shortcut".into(),
            ));
        };
        if sc.is_plain_chord() {
            self.grab(key, sc)?;
        }
        self.matcher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .add(key, *sc);
        Ok(Binding::Exact(*sc))
    }

    fn unregister(&mut self, key: BindingKey) {
        self.matcher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
        self.ungrab(key);
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        let keys: Vec<_> = self.grabs.iter().map(|g| g.0).collect();
        for k in keys {
            self.ungrab(k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes_are_evdev_plus_eight() {
        // evdev KEY_A = 30, KEY_SPACE = 57, KEY_RIGHTCTRL = 97.
        assert_eq!(x_keycode(KeyCode::KeyA), Some(38));
        assert_eq!(x_keycode(KeyCode::Space), Some(65));
        assert_eq!(x_keycode(KeyCode::ControlRight), Some(105));
        assert_eq!(key_of_x_keycode(105), Some(KeyCode::ControlRight));
        assert_eq!(key_of_x_keycode(3), None);
    }

    /// Live: two connections grab the same chord; the second is a conflict.
    /// Needs an X server (`DISPLAY`); under XWayland it grabs only from X11
    /// windows, briefly, and a chord nobody types.
    #[test]
    #[ignore = "needs an X server; run with --ignored"]
    fn a_chord_another_client_grabbed_is_a_conflict() {
        let sc = Shortcut::parse("Ctrl+Alt+Shift+F20").unwrap();
        let mut a = X11Backend::start().expect("X11");
        let mut b = X11Backend::start().expect("X11");
        let meta = Meta {
            id: "t".into(),
            description: "t".into(),
        };
        assert_eq!(a.register(1, Some(&sc), &meta), Ok(Binding::Exact(sc)));
        assert_eq!(
            b.register(1, Some(&sc), &meta),
            Err(HotkeyError::Conflict(sc.to_string()))
        );
        assert!(b.grabs.is_empty(), "a refused grab leaves nothing behind");
        a.unregister(1);
        assert_eq!(
            b.register(1, Some(&sc), &meta),
            Ok(Binding::Exact(sc)),
            "free again"
        );
        b.unregister(1);
    }

    /// Live: real XTEST key presses reach a registered hotkey's callback on
    /// the main thread, press and release, for a grabbed chord and for a
    /// modifier-only binding. Needs a **private** X server with XTEST — a
    /// nested `Xwayland :99 -rootless` or an Xvfb — never the session's own
    /// (`DISPLAY` must be set to it by the runner). Run alone:
    /// `DISPLAY=:99 cargo test -p rinch --features global-hotkeys --lib
    /// hotkey::backend::x11::tests::xtest_ -- --ignored --exact`.
    #[test]
    #[ignore = "needs a private X server; see the doc comment"]
    fn xtest_presses_and_releases_reach_the_callback() {
        use std::cell::RefCell;
        use std::rc::Rc;
        use x11rb::protocol::xtest::ConnectionExt as _;

        use crate::hotkey::{HotkeyState, register};

        // SAFETY: test-only, before any thread of this test reads the env.
        unsafe { std::env::set_var("RINCH_HOTKEY_BACKEND", "x11") };
        let log = Rc::new(RefCell::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());
        let _chord = register("Ctrl+Alt+F19", move |e| {
            l1.borrow_mut().push(("chord", e.state))
        })
        .unwrap();
        let _hold = register("RightCtrl", move |e| {
            l2.borrow_mut().push(("hold", e.state))
        })
        .unwrap();

        let (conn, screen) = RustConnection::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let press = |code: KeyCode, down: bool| {
            let kc = x_keycode(code).unwrap();
            let ty = if down {
                xproto::KEY_PRESS_EVENT
            } else {
                xproto::KEY_RELEASE_EVENT
            };
            conn.xtest_fake_input(ty, kc, 0, root, 0, 0, 0).unwrap();
            conn.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        };
        use KeyCode::*;
        for (k, d) in [
            (ControlLeft, true),
            (AltLeft, true),
            (F19, true),
            (F19, false),
            (AltLeft, false),
            (ControlLeft, false),
            (ControlRight, true),
            (ControlRight, false),
            // Right Ctrl pressed while another key is held is not the
            // modifier-only binding.
            (KeyC, true),
            (ControlRight, true),
            (ControlRight, false),
            (KeyC, false),
        ] {
            press(k, d);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        rinch_core::reactive::drain_main_callbacks();
        assert_eq!(
            *log.borrow(),
            vec![
                ("chord", HotkeyState::Pressed),
                ("chord", HotkeyState::Released),
                ("hold", HotkeyState::Pressed),
                ("hold", HotkeyState::Released),
            ]
        );
    }
}
