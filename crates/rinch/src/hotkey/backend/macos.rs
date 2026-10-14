//! macOS: Carbon hot keys for plain chords, and a listen-only `CGEventTap`
//! for modifier-only and sided bindings.
//!
//! - **A plain chord** goes to `RegisterEventHotKey` on the application event
//!   target. It needs **no permission**, reports press and release
//!   (`kEventHotKeyPressed`/`Released`), reports a conflict
//!   (`eventHotKeyExistsErr` → [`HotkeyError::Conflict`]), and the system
//!   swallows the chord. Its events arrive on the main thread through the
//!   NSApplication run loop winit drives, and are reported through the main
//!   queue like every other backend's.
//! - **A modifier-only or sided binding** (`RightCtrl`, `LeftCmd+Space`)
//!   cannot be a Carbon hot key, so it needs the event tap, and the tap needs
//!   the **Input Monitoring** permission. The tap is created on its own
//!   thread with its own run loop the first time such a binding is
//!   registered, and only then; without the permission that registration is
//!   [`HotkeyError::PermissionMissing`] and [`super::super::permission`] says
//!   `Missing` until a tap starts. The tap only listens: such a binding is not
//!   swallowed.
//! - Modifier keys come to the tap as `flagsChanged`, and whether one went
//!   down or up is read from the device-dependent (left/right) flag bits, not
//!   toggled, so a lost event does not invert it. A tap the system disabled
//!   (timeout, secure input) is re-enabled and every pressed binding
//!   released.
//!
//! Hand-written FFI (Carbon, CoreGraphics, CoreFoundation): no new crate.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::scancode::PhysicalKeyExtScancode;

use super::BackendImpl;
use crate::hotkey::matcher::{BindingKey, Matcher};
use crate::hotkey::{
    Backend, Binding, HotkeyError, HotkeyState, Meta, Modifier, Permission, Report, Shortcut,
    Trigger, report,
};

type OSStatus = i32;
type Ptr = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type CarbonHandler = extern "C" fn(call: Ptr, event: Ptr, user: Ptr) -> OSStatus;
type TapCallback = extern "C" fn(proxy: Ptr, ty: u32, event: Ptr, user: Ptr) -> Ptr;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> Ptr;
    fn InstallEventHandler(
        target: Ptr,
        handler: CarbonHandler,
        num_types: u32,
        list: *const EventTypeSpec,
        user: Ptr,
        out: *mut Ptr,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: Ptr,
        options: u32,
        out: *mut Ptr,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hotkey: Ptr) -> OSStatus;
    fn GetEventParameter(
        event: Ptr,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: Ptr,
    ) -> OSStatus;
    fn GetEventKind(event: Ptr) -> u32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        cb: TapCallback,
        user: Ptr,
    ) -> Ptr;
    fn CGEventTapEnable(tap: Ptr, enable: bool);
    fn CGEventGetIntegerValueField(event: Ptr, field: u32) -> i64;
    fn CGEventGetFlags(event: Ptr) -> u64;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: Ptr;
    fn CFMachPortCreateRunLoopSource(alloc: Ptr, port: Ptr, order: isize) -> Ptr;
    fn CFRunLoopGetCurrent() -> Ptr;
    fn CFRunLoopAddSource(rl: Ptr, source: Ptr, mode: Ptr);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: Ptr);
    fn CFRelease(obj: Ptr);
}

const fn four(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}
const K_EVENT_CLASS_KEYBOARD: u32 = four(b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_HOT_KEY_RELEASED: u32 = 6;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = four(b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = four(b"hkid");
const SIGNATURE: u32 = four(b"rnch");
const EVENT_HOT_KEY_EXISTS_ERR: OSStatus = -9878;
const CMD_KEY: u32 = 1 << 8;
const SHIFT_KEY: u32 = 1 << 9;
const OPTION_KEY: u32 = 1 << 11;
const CONTROL_KEY: u32 = 1 << 12;

const K_CG_SESSION_EVENT_TAP: u32 = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
const K_CG_KEY_DOWN: u32 = 10;
const K_CG_KEY_UP: u32 = 11;
const K_CG_FLAGS_CHANGED: u32 = 12;
const K_CG_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const K_CG_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

pub(crate) struct MacBackend {
    handler_installed: bool,
    carbon: BTreeMap<BindingKey, Ptr>,
    tap: Option<Tap>,
    tap_keys: Vec<BindingKey>,
    /// A tap binding was refused for want of the permission, and no tap has
    /// started since.
    refused_for_permission: bool,
}

struct Tap {
    shared: Arc<TapShared>,
    run_loop: Arc<AtomicUsize>,
}

struct TapShared {
    matcher: Mutex<Matcher>,
    port: AtomicUsize,
}

impl MacBackend {
    pub(crate) fn new() -> Self {
        MacBackend {
            handler_installed: false,
            carbon: BTreeMap::new(),
            tap: None,
            tap_keys: Vec::new(),
            refused_for_permission: false,
        }
    }

    fn ensure_handler(&mut self) -> Result<(), HotkeyError> {
        if self.handler_installed {
            return Ok(());
        }
        let types = [
            EventTypeSpec {
                event_class: K_EVENT_CLASS_KEYBOARD,
                event_kind: K_EVENT_HOT_KEY_PRESSED,
            },
            EventTypeSpec {
                event_class: K_EVENT_CLASS_KEYBOARD,
                event_kind: K_EVENT_HOT_KEY_RELEASED,
            },
        ];
        let mut out = std::ptr::null_mut();
        // SAFETY: Carbon calls on the main thread with valid pointers.
        let st = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                carbon_handler,
                2,
                types.as_ptr(),
                std::ptr::null_mut(),
                &mut out,
            )
        };
        if st != 0 {
            return Err(HotkeyError::Platform(format!(
                "InstallEventHandler failed ({st})"
            )));
        }
        self.handler_installed = true;
        Ok(())
    }

    fn register_carbon(&mut self, key: BindingKey, sc: &Shortcut) -> Result<(), HotkeyError> {
        self.ensure_handler()?;
        let Trigger::Key(code) = sc.trigger() else {
            unreachable!("a plain chord has a key trigger");
        };
        let Some(vk) = mac_keycode(code) else {
            return Err(HotkeyError::Unsupported(format!(
                "no macOS key code for {sc}"
            )));
        };
        let mut mods = 0;
        for (m, _) in sc.held_modifiers() {
            mods |= match m {
                Modifier::Ctrl => CONTROL_KEY,
                Modifier::Alt => OPTION_KEY,
                Modifier::Shift => SHIFT_KEY,
                Modifier::Super => CMD_KEY,
            };
        }
        let id = EventHotKeyID {
            signature: SIGNATURE,
            id: u32::try_from(key).map_err(|_| HotkeyError::Platform("too many hotkeys".into()))?,
        };
        let mut out = std::ptr::null_mut();
        // SAFETY: Carbon call on the main thread.
        let st =
            unsafe { RegisterEventHotKey(vk, mods, id, GetApplicationEventTarget(), 0, &mut out) };
        match st {
            0 => {
                self.carbon.insert(key, out);
                Ok(())
            }
            EVENT_HOT_KEY_EXISTS_ERR => Err(HotkeyError::Conflict(sc.to_string())),
            e => Err(HotkeyError::Platform(format!(
                "RegisterEventHotKey failed ({e})"
            ))),
        }
    }

    fn ensure_tap(&mut self) -> Result<&Tap, HotkeyError> {
        if self.tap.is_none() {
            // SAFETY: a plain query.
            if !unsafe { CGPreflightListenEventAccess() } {
                self.refused_for_permission = true;
                return Err(HotkeyError::PermissionMissing);
            }
            let shared = Arc::new(TapShared {
                matcher: Mutex::new(Matcher::default()),
                port: AtomicUsize::new(0),
            });
            let run_loop = Arc::new(AtomicUsize::new(0));
            let (tx, rx) = mpsc::channel::<bool>();
            let (s, r) = (shared.clone(), run_loop.clone());
            std::thread::Builder::new()
                .name("rinch-hotkey-tap".into())
                .spawn(move || tap_thread(s, r, tx))
                .map_err(|e| HotkeyError::Platform(e.to_string()))?;
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(true) => {}
                Ok(false) => {
                    self.refused_for_permission = true;
                    return Err(HotkeyError::PermissionMissing);
                }
                Err(_) => return Err(HotkeyError::Platform("the event tap did not start".into())),
            }
            self.refused_for_permission = false;
            self.tap = Some(Tap { shared, run_loop });
        }
        Ok(self.tap.as_ref().expect("just set"))
    }
}

/// The macOS virtual key code of a physical key (winit's macOS scancode).
fn mac_keycode(code: KeyCode) -> Option<u32> {
    PhysicalKey::Code(code).to_scancode()
}

fn key_of_mac_keycode(vk: u32) -> Option<KeyCode> {
    match PhysicalKey::from_scancode(vk) {
        PhysicalKey::Code(c) => Some(c),
        PhysicalKey::Unidentified(_) => None,
    }
}

extern "C" fn carbon_handler(_call: Ptr, event: Ptr, _user: Ptr) -> OSStatus {
    let mut id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    // SAFETY: reading the hot-key id parameter of a hot-key event.
    let st = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            std::ptr::null_mut(),
            (&mut id as *mut EventHotKeyID).cast(),
        )
    };
    if st != 0 || id.signature != SIGNATURE {
        return -9874; // eventNotHandledErr: not ours
    }
    // SAFETY: a plain accessor.
    let state = match unsafe { GetEventKind(event) } {
        K_EVENT_HOT_KEY_PRESSED => HotkeyState::Pressed,
        K_EVENT_HOT_KEY_RELEASED => HotkeyState::Released,
        _ => return -9874,
    };
    report(BindingKey::from(id.id), Report::State(state));
    0
}

/// The device-dependent flag bit for each sided modifier key.
fn flag_bit(code: KeyCode) -> Option<u64> {
    Some(match code {
        KeyCode::ControlLeft => 0x1,
        KeyCode::ShiftLeft => 0x2,
        KeyCode::ShiftRight => 0x4,
        KeyCode::MetaLeft => 0x8,
        KeyCode::MetaRight => 0x10,
        KeyCode::AltLeft => 0x20,
        KeyCode::AltRight => 0x40,
        KeyCode::ControlRight => 0x2000,
        _ => return None,
    })
}

extern "C" fn tap_callback(_proxy: Ptr, ty: u32, event: Ptr, user: Ptr) -> Ptr {
    // SAFETY: `user` is the `TapShared` the tap thread keeps alive for as long
    // as the tap exists.
    let shared = unsafe { &*(user as *const TapShared) };
    let mut out = Vec::new();
    {
        let mut m = shared.matcher.lock().unwrap_or_else(|p| p.into_inner());
        match ty {
            K_CG_TAP_DISABLED_BY_TIMEOUT | K_CG_TAP_DISABLED_BY_USER_INPUT => {
                m.release_all(&mut out);
                let port = shared.port.load(Ordering::Relaxed) as Ptr;
                if !port.is_null() {
                    // SAFETY: the tap's own port.
                    unsafe { CGEventTapEnable(port, true) };
                }
            }
            K_CG_KEY_DOWN | K_CG_KEY_UP | K_CG_FLAGS_CHANGED => {
                // SAFETY: plain accessors on the event we were handed.
                let vk = unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) }
                    as u32;
                if let Some(code) = key_of_mac_keycode(vk) {
                    let down = if ty == K_CG_FLAGS_CHANGED {
                        match flag_bit(code) {
                            // SAFETY: as above.
                            Some(bit) => (unsafe { CGEventGetFlags(event) } & bit) != 0,
                            None => !m.down_keys().contains(&code), // Caps Lock, Fn
                        }
                    } else {
                        ty == K_CG_KEY_DOWN
                    };
                    m.key(code, down, &mut out);
                }
            }
            _ => {}
        }
    }
    for (k, s) in out {
        report(k, Report::State(s));
    }
    event
}

fn tap_thread(shared: Arc<TapShared>, run_loop: Arc<AtomicUsize>, ready: mpsc::Sender<bool>) {
    let mask = (1u64 << K_CG_KEY_DOWN) | (1u64 << K_CG_KEY_UP) | (1u64 << K_CG_FLAGS_CHANGED);
    // SAFETY: CoreGraphics/CoreFoundation calls on this thread; `shared`
    // outlives the tap (this thread holds it until its run loop returns).
    unsafe {
        let port = CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            tap_callback,
            Arc::as_ptr(&shared) as Ptr,
        );
        if port.is_null() {
            let _ = ready.send(false);
            return;
        }
        shared.port.store(port as usize, Ordering::Relaxed);
        let source = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), port, 0);
        let rl = CFRunLoopGetCurrent();
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);
        run_loop.store(rl as usize, Ordering::Relaxed);
        let _ = ready.send(true);
        CFRunLoopRun();
        CGEventTapEnable(port, false);
        CFRelease(source);
        CFRelease(port);
    }
}

impl BackendImpl for MacBackend {
    fn kind(&self) -> Backend {
        Backend::MacOs
    }

    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        _meta: &Meta,
    ) -> Result<Binding, HotkeyError> {
        let Some(sc) = shortcut else {
            return Err(HotkeyError::Unsupported(
                "macOS has no system dialog to choose a binding; give a shortcut".into(),
            ));
        };
        if sc.is_plain_chord() {
            self.register_carbon(key, sc)?;
        } else {
            let tap = self.ensure_tap()?;
            tap.shared
                .matcher
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .add(key, *sc);
            self.tap_keys.push(key);
        }
        Ok(Binding::Exact(*sc))
    }

    fn unregister(&mut self, key: BindingKey) {
        if let Some(hk) = self.carbon.remove(&key) {
            // SAFETY: a hot key this backend registered.
            unsafe { UnregisterEventHotKey(hk) };
        }
        if let Some(tap) = &self.tap {
            tap.shared
                .matcher
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(key);
        }
        self.tap_keys.retain(|k| *k != key);
    }

    fn permission(&self) -> Permission {
        if !self.tap_keys.is_empty() {
            Permission::Granted
        } else if self.refused_for_permission {
            Permission::Missing
        } else {
            Permission::NotNeeded
        }
    }

    fn request_permission(&mut self) -> Permission {
        if self.refused_for_permission {
            // SAFETY: shows the system prompt (or opens the settings pane).
            if unsafe { CGRequestListenEventAccess() } {
                // Granted: the app registers its tap bindings again.
                self.refused_for_permission = false;
            }
        }
        self.permission()
    }
}

impl Drop for MacBackend {
    fn drop(&mut self) {
        for (_, hk) in std::mem::take(&mut self.carbon) {
            // SAFETY: hot keys this backend registered.
            unsafe { UnregisterEventHotKey(hk) };
        }
        if let Some(tap) = &self.tap {
            let rl = tap.run_loop.load(Ordering::Relaxed) as Ptr;
            if !rl.is_null() {
                // SAFETY: stops the tap thread's run loop; it then releases
                // the tap.
                unsafe { CFRunLoopStop(rl) };
            }
        }
    }
}
