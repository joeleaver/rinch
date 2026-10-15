//! Windows: one hook thread running a `WH_KEYBOARD_LL` low-level keyboard
//! hook and a message pump, plus `RegisterHotKey` to reserve plain chords.
//!
//! - **The hook only posts.** Windows removes a low-level hook that does not
//!   return within `LowLevelHooksTimeout` (about 1 s, less on recent builds),
//!   silently. So `hook_proc` reads the key, posts it to its own thread's
//!   queue with `PostThreadMessageW` and returns; the matching and the
//!   main-thread report happen after it returned, from the pump.
//! - **Press/release of every binding comes from the hook** and the shared
//!   [`Matcher`], which is what gives release events, left/right keys and
//!   modifier-only bindings (`RegisterHotKey` has none of the three).
//! - **A plain chord is also registered with `RegisterHotKey`** on the hook
//!   thread (`MOD_NOREPEAT`), which reserves it: another application holding it
//!   is `ERROR_HOTKEY_ALREADY_REGISTERED` → [`HotkeyError::Conflict`], and the
//!   system then **swallows** the chord — the focused application does not
//!   receive it. A sided or modifier-only binding is the hook alone: it is not
//!   swallowed (Right Ctrl still reaches the focused application) and there is
//!   no conflict check.
//! - **A key-up the hook never sees cannot strand a binding.** While an
//!   elevated (administrator) window has the focus the hook sees no input at
//!   all (UIPI), so a release can be lost. Every key-down first asks
//!   `GetAsyncKeyState` about each key the matcher believes is down and drops
//!   the ones that are up (`Matcher::key_reconciled`), so a lost Alt cannot
//!   refuse the next chord; and while any key is believed down, a 100 ms
//!   timer does the same, so a pressed binding is released without waiting
//!   for another key.
//! - The fake Left Ctrl an AltGr layout sends with Right Alt is skipped, so
//!   `RightAlt` is a modifier-only binding there too.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MAPVK_VSC_TO_VK_EX, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    MapVirtualKeyW, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, KillTimer,
    LLKHF_EXTENDED, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_QUIT,
    WM_SYSKEYDOWN, WM_TIMER,
};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::scancode::PhysicalKeyExtScancode;

use super::BackendImpl;
use crate::hotkey::matcher::{BindingKey, Matcher};
use crate::hotkey::{
    Backend, Binding, HotkeyError, Meta, Modifier, Shortcut, Trigger, report_matched,
};

const WM_HOTKEY_KEY: u32 = WM_APP + 0x31;
const WM_HOTKEY_CMD: u32 = WM_APP + 0x32;
const POLL_MS: u32 = 100;

/// The hook thread's id, read by `hook_proc` (a bare `extern fn` with no
/// context argument). One hook thread per process.
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

enum Cmd {
    Register(
        BindingKey,
        Shortcut,
        mpsc::SyncSender<Result<(), HotkeyError>>,
    ),
    Unregister(BindingKey),
}

pub(crate) struct WinBackend {
    thread: u32,
}

impl WinBackend {
    pub(crate) fn start() -> Result<Self, HotkeyError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("rinch-hotkey-hook".into())
            .spawn(move || pump(ready_tx))
            .map_err(|e| HotkeyError::Platform(e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(thread)) => Ok(WinBackend { thread }),
            Ok(Err(e)) => Err(HotkeyError::Platform(e)),
            Err(_) => Err(HotkeyError::Platform(
                "the hook thread did not start".into(),
            )),
        }
    }

    fn send(&self, cmd: Cmd) -> bool {
        let ptr = Box::into_raw(Box::new(cmd));
        // SAFETY: on success the pump takes the box back (`Box::from_raw`) and
        // owns it; on failure nobody received it and it is reclaimed here.
        let ok = unsafe { PostThreadMessageW(self.thread, WM_HOTKEY_CMD, 0, ptr as LPARAM) } != 0;
        if !ok {
            drop(unsafe { Box::from_raw(ptr) });
        }
        ok
    }
}

impl Drop for WinBackend {
    fn drop(&mut self) {
        // SAFETY: plain message post; the pump unhooks and exits on WM_QUIT.
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
    }
}

impl BackendImpl for WinBackend {
    fn kind(&self) -> Backend {
        Backend::Windows
    }

    fn register(
        &mut self,
        key: BindingKey,
        shortcut: Option<&Shortcut>,
        _meta: &Meta,
    ) -> Result<Binding, HotkeyError> {
        let Some(sc) = shortcut else {
            return Err(HotkeyError::Unsupported(
                "Windows has no system dialog to choose a binding; give a shortcut".into(),
            ));
        };
        let (tx, rx) = mpsc::sync_channel(1);
        if !self.send(Cmd::Register(key, *sc, tx)) {
            return Err(HotkeyError::Platform("the hook thread has stopped".into()));
        }
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(r) => r.map(|()| Binding::Exact(*sc)),
            Err(_) => Err(HotkeyError::Platform(
                "the hook thread did not answer".into(),
            )),
        }
    }

    fn unregister(&mut self, key: BindingKey) {
        self.send(Cmd::Unregister(key));
    }
}

/// The physical key of a hook event: the scan code, `0xE0`-prefixed when
/// extended, in winit's Windows scancode table.
fn key_of(scan: u32, extended: bool) -> Option<KeyCode> {
    let sc = if extended { scan | 0xE000 } else { scan };
    match PhysicalKey::from_scancode(sc) {
        PhysicalKey::Code(c) => Some(c),
        PhysicalKey::Unidentified(_) => None,
    }
}

/// The virtual-key code of a physical key under the current layout — what
/// `RegisterHotKey` and `GetAsyncKeyState` take.
fn vk_of(code: KeyCode) -> Option<u32> {
    let sc = PhysicalKey::Code(code).to_scancode()?;
    // SAFETY: a pure table lookup.
    let vk = unsafe { MapVirtualKeyW(sc, MAPVK_VSC_TO_VK_EX) };
    (vk != 0).then_some(vk)
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points at a KBDLLHOOKSTRUCT.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // 0x21D: the fake Left Ctrl an AltGr layout sends with Right Alt.
        if kb.scanCode & 0x200 == 0 {
            let down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
            let extended = kb.flags & LLKHF_EXTENDED != 0;
            let packed =
                (kb.scanCode & 0xFFFF) | ((extended as u32) << 16) | ((kb.vkCode & 0xFF) << 17);
            // SAFETY: a post to this very thread's queue; nothing else here.
            unsafe {
                PostThreadMessageW(
                    HOOK_THREAD.load(Ordering::Relaxed),
                    WM_HOTKEY_KEY,
                    packed as WPARAM,
                    down as LPARAM,
                )
            };
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn pump(ready: mpsc::Sender<Result<u32, String>>) {
    // SAFETY: Win32 calls on this thread; `msg` outlives every use.
    unsafe {
        let me = GetCurrentThreadId();
        let mut msg: MSG = std::mem::zeroed();
        // Create this thread's message queue before anyone posts to it.
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        HOOK_THREAD.store(me, Ordering::Relaxed);
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        );
        if hook.is_null() {
            let _ = ready.send(Err(format!(
                "SetWindowsHookExW failed ({})",
                GetLastError()
            )));
            return;
        }
        let _ = ready.send(Ok(me));

        let mut matcher = Matcher::default();
        let mut vks: BTreeMap<KeyCode, u32> = BTreeMap::new();
        let mut hotkey_ids: BTreeMap<BindingKey, i32> = BTreeMap::new();
        let mut next_id: i32 = 1;
        let mut timer: usize = 0;
        let mut out = Vec::new();

        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            match msg.message {
                WM_HOTKEY_KEY => {
                    let packed = msg.wParam as u32;
                    let scan = packed & 0xFFFF;
                    let extended = packed & (1 << 16) != 0;
                    let vk = (packed >> 17) & 0xFF;
                    if let Some(code) = key_of(scan, extended) {
                        vks.insert(code, vk);
                        // A key-up the hook never saw (UIPI) is dropped here,
                        // before it can refuse this chord.
                        let still_down = |k: KeyCode| {
                            let vk = vks.get(&k).copied().unwrap_or(0);
                            vk == 0 || GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0
                        };
                        matcher.key_reconciled(code, msg.lParam != 0, still_down, &mut out);
                    }
                }
                WM_TIMER if msg.wParam == timer && timer != 0 => {
                    for code in matcher.down_keys().to_vec() {
                        let vk = vks.get(&code).copied().unwrap_or(0);
                        if vk != 0 && GetAsyncKeyState(vk as i32) as u16 & 0x8000 == 0 {
                            matcher.forget_key(code, &mut out);
                        }
                    }
                }
                WM_HOTKEY_CMD => {
                    let cmd = *Box::from_raw(msg.lParam as *mut Cmd);
                    match cmd {
                        Cmd::Register(key, sc, reply) => {
                            let mut result = Ok(());
                            if sc.is_plain_chord() {
                                let id = next_id;
                                next_id = if next_id >= 0xBFFF { 1 } else { next_id + 1 };
                                result = register_chord(id, &sc);
                                if result.is_ok() {
                                    hotkey_ids.insert(key, id);
                                }
                            }
                            if result.is_ok() {
                                matcher.add(key, sc);
                            }
                            let _ = reply.send(result);
                        }
                        Cmd::Unregister(key) => {
                            matcher.remove(key);
                            if let Some(id) = hotkey_ids.remove(&key) {
                                UnregisterHotKey(std::ptr::null_mut(), id);
                            }
                        }
                    }
                }
                _ => {
                    DispatchMessageW(&msg);
                }
            }
            report_matched(&matcher, &mut out);
            // Poll for lost key-ups while any key is believed down, so a
            // pressed binding is released even if no other key comes.
            let held = !matcher.down_keys().is_empty();
            if held && timer == 0 {
                timer = SetTimer(std::ptr::null_mut(), 0, POLL_MS, None);
            } else if !held && timer != 0 {
                KillTimer(std::ptr::null_mut(), timer);
                timer = 0;
            }
        }
        // WM_QUIT: release what is held and leave.
        matcher.release_all(&mut out);
        report_matched(&matcher, &mut out);
        for id in hotkey_ids.values() {
            UnregisterHotKey(std::ptr::null_mut(), *id);
        }
        UnhookWindowsHookEx(hook);
    }
}

fn register_chord(id: i32, sc: &Shortcut) -> Result<(), HotkeyError> {
    let Trigger::Key(code) = sc.trigger() else {
        return Ok(());
    };
    let Some(vk) = vk_of(code) else {
        return Err(HotkeyError::Unsupported(format!(
            "no virtual-key code for {sc} on this layout"
        )));
    };
    let mut mods = MOD_NOREPEAT;
    for (m, _) in sc.held_modifiers() {
        mods |= match m {
            Modifier::Ctrl => MOD_CONTROL,
            Modifier::Alt => MOD_ALT,
            Modifier::Shift => MOD_SHIFT,
            Modifier::Super => MOD_WIN,
        };
    }
    // SAFETY: registers against this (the hook) thread's queue.
    if unsafe { RegisterHotKey(std::ptr::null_mut(), id, mods, vk) } != 0 {
        return Ok(());
    }
    // SAFETY: reads this thread's last error.
    match unsafe { GetLastError() } {
        ERROR_HOTKEY_ALREADY_REGISTERED => Err(HotkeyError::Conflict(sc.to_string())),
        e => Err(HotkeyError::Platform(format!(
            "RegisterHotKey failed ({e})"
        ))),
    }
}
