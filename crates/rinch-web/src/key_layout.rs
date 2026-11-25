//! What each physical key types without AltGr, for the AltGr question
//! (#1291, #3062).
//!
//! Windows reports AltGr as Ctrl+Alt, so a browser keydown cannot say by its
//! modifiers alone whether right-Alt+7 typed `{` (text, the browser's to
//! insert) or is the app's Mod-Alt-7 chord. Desktop answers with
//! [`rinch::platform::is_alt_graph_text`]: the press is text when it typed a
//! character that is not simply the one the key types without AltGr (winit's
//! `key_without_modifiers`). A browser reports no such field, so this module
//! keeps the page's own answer per physical key (`code`):
//!
//! - **learned** from every plain press (no Ctrl, Alt, Meta, AltGraph, not
//!   composing), and from every Shift-only press for the Shift level — a
//!   learned entry follows a layout switch, so it always wins;
//! - **seeded** once per page, at the first mount, from
//!   `navigator.keyboard.getLayoutMap()` where the browser has it (Chromium;
//!   Firefox and Safari do not). A seed never replaces a learned entry;
//! - else a **US fallback** for letter and digit keys (`KeyQ` → `q`,
//!   `Digit1` → `1`, and Shift+`Digit1` → `!`), right for the letters and
//!   digits of most layouts. The digit fallback is dropped once a plain press
//!   has shown this layout's digit row types something other than digits
//!   (Lithuanian `ą č ę …`, French `& é " …`);
//! - else unknown, and the AltGr text is taken at its word, as desktop does
//!   when it does not know.
//!
//! **Which way an unknown key errs.** Where the fallback is wrong for a key
//! nobody has pressed plainly yet, an AltGr press there is misjudged until the
//! first plain press of that key teaches the page. With the fallback, a layout
//! whose AltGr+N types N itself reads that press as the Mod-Alt-N chord, and an
//! app that binds it swallows the character; without one, the press is text
//! and the app's chord is missed. A missed chord is the safer error: the
//! command still has its menu entry or toolbar button, while a swallowed
//! character has no other way in. So the fallback is kept only where it is
//! right on most layouts and where its miss is the rare one (German and
//! Polish right-Alt+1 report `"1"` and must stay the chord, as on desktop),
//! and dropped as soon as the page has seen it is wrong for this layout.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::{Closure, JsValue};

/// The facts of one keydown the layout table reads: the browser's
/// `KeyboardEvent`, or a test's.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Press<'a> {
    /// The physical key, `KeyboardEvent.code` (`"Digit1"`, `"KeyQ"`).
    pub code: &'a str,
    /// What the browser says the press produced, `KeyboardEvent.key`.
    pub key: &'a str,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    /// `getModifierState("AltGraph")`: how Windows marks an AltGr press.
    pub alt_graph: bool,
    /// `KeyboardEvent.isComposing`: an input method owns the press.
    pub composing: bool,
}

impl<'a> Press<'a> {
    fn of(event: &web_sys::KeyboardEvent, code: &'a str, key: &'a str) -> Self {
        Press {
            code,
            key,
            shift: event.shift_key(),
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
            alt_graph: event.get_modifier_state("AltGraph"),
            composing: event.is_composing(),
        }
    }
}

/// What the keys of the user's layout type without AltGr, as far as this
/// page has seen (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct KeyLayout {
    /// `code` → the character it types with no modifier.
    plain: HashMap<String, String>,
    /// `code` → the character it types with Shift alone.
    shifted: HashMap<String, String>,
}

/// US Shift+digit, `Digit0` through `Digit9`.
const US_SHIFTED_DIGITS: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];

impl KeyLayout {
    /// Learn from `press` what its key types, when the press says so: Shift
    /// at most, not composing, and a `key` that is one printable character
    /// (not `"Dead"`, `"Process"` or a key name). A composing press is the
    /// input method's: its `key` is not what the key types.
    pub(crate) fn learn(&mut self, press: &Press) {
        if press.ctrl || press.alt || press.meta || press.alt_graph || press.composing {
            return;
        }
        if press.code.is_empty() || !is_one_printable_char(press.key) {
            return;
        }
        let level = if press.shift {
            &mut self.shifted
        } else {
            &mut self.plain
        };
        level.insert(press.code.to_owned(), press.key.to_owned());
    }

    /// Seed what `code` types with no modifier from the browser's layout map.
    /// Never replaces an entry a press taught: the press is newer.
    pub(crate) fn seed(&mut self, code: &str, key: &str) {
        if code.is_empty() || !is_one_printable_char(key) {
            return;
        }
        self.plain
            .entry(code.to_owned())
            .or_insert_with(|| key.to_owned());
    }

    /// Whether this layout's digit row may be taken to type digits: no plain
    /// `DigitN` entry, learned or seeded, says otherwise.
    fn digits_are_us(&self) -> bool {
        self.plain.iter().all(|(code, key)| {
            !code.starts_with("Digit") || key.chars().all(|c| c.is_ascii_digit())
        })
    }

    /// What `code` types without AltGr, at the Shift level `shift`: learned
    /// or seeded, else the US fallback (see the module docs), else `None`.
    pub(crate) fn base(&self, code: &str, shift: bool) -> Option<String> {
        let known = if shift { &self.shifted } else { &self.plain };
        if let Some(known) = known.get(code) {
            return Some(known.clone());
        }
        if let Some(letter) = code.strip_prefix("Key") {
            // A letter's Shift level is its capital, and the comparison
            // ignores case, so the plain answer serves both levels.
            if shift {
                return self.base(code, false);
            }
            let mut chars = letter.chars();
            let c = chars.next()?;
            return (chars.next().is_none() && c.is_ascii_alphabetic())
                .then(|| c.to_ascii_lowercase().to_string());
        }
        let digit = code.strip_prefix("Digit")?.parse::<usize>().ok()?;
        if digit > 9 || !self.digits_are_us() {
            return None;
        }
        Some(if shift {
            US_SHIFTED_DIGITS[digit].to_string()
        } else {
            digit.to_string()
        })
    }

    /// Whether AltGr typed `press.key` — desktop's
    /// [`rinch::platform::is_alt_graph_text`], asked with the browser's
    /// facts. The text must be one printable character (not a key name) and
    /// differ from what the key types without AltGr *at the press's Shift
    /// level*: a browser's `key` for an AltGr press that types nothing falls
    /// back to the key's own character with Shift applied (German
    /// AltGr+Shift+1 reports `"!"`), and desktop, whose `text` is then empty,
    /// reads that press as the chord.
    pub(crate) fn is_alt_graph_text(&self, press: &Press) -> bool {
        if !is_one_printable_char(press.key) {
            return false;
        }
        let base = self.base(press.code, press.shift);
        rinch::platform::is_alt_graph_text(
            press.alt_graph,
            rinch::platform::Modifiers {
                shift: press.shift,
                ctrl: press.ctrl,
                alt: press.alt,
                meta: press.meta,
            },
            Some(press.key),
            base.as_deref(),
        )
    }
}

pub(crate) fn is_one_printable_char(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| !c.is_control() && chars.next().is_none())
}

thread_local! {
    /// The page's [`KeyLayout`].
    static LAYOUT: RefCell<KeyLayout> = RefCell::new(KeyLayout::default());
    static LAYOUT_MAP_ASKED: Cell<bool> = const { Cell::new(false) };
}

/// Learn from `event` what its key types ([`KeyLayout::learn`]). Every keydown
/// listener that asks [`web_alt_graph_text`] calls this first.
pub(crate) fn note_press(event: &web_sys::KeyboardEvent) {
    let (code, key) = (event.code(), event.key());
    LAYOUT.with(|l| l.borrow_mut().learn(&Press::of(event, &code, &key)));
}

/// Whether AltGr typed `event.key` ([`KeyLayout::is_alt_graph_text`]).
pub(crate) fn web_alt_graph_text(event: &web_sys::KeyboardEvent) -> bool {
    let (code, key) = (event.code(), event.key());
    LAYOUT.with(|l| l.borrow().is_alt_graph_text(&Press::of(event, &code, &key)))
}

/// What the page knows the physical key `code` types with no modifier,
/// learned from a plain press or seeded from the browser's layout map (no
/// US fallback). Test-only.
#[doc(hidden)]
pub fn __known_unmodified_key(code: &str) -> Option<String> {
    LAYOUT.with(|l| l.borrow().plain.get(code).cloned())
}

/// Forget every key the page learned or seeded, so a test's layout starts
/// from the fallbacks. Test-only.
#[doc(hidden)]
pub fn __forget_key_layout() {
    LAYOUT.with(|l| *l.borrow_mut() = KeyLayout::default());
}

/// Seed the page's layout from `navigator.keyboard.getLayoutMap()`, once per
/// page, where the browser has it (Chromium; Firefox and Safari do not).
/// Called at the first mount, before any key, so the map has usually
/// arrived by the page's first press: the first AltGr press on a layout whose
/// keys differ from US (French `Digit1` types `&`) is then judged right before
/// any plain press taught it.
pub(crate) fn ask_layout_map_once() {
    if LAYOUT_MAP_ASKED.with(|a| a.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let get = |target: &JsValue, name: &str| js_sys::Reflect::get(target, &name.into()).ok();
    let Some(keyboard) = get(&window.into(), "navigator")
        .and_then(|navigator| get(&navigator, "keyboard"))
        .filter(|k| k.is_object())
    else {
        return;
    };
    let Some(get_layout_map) =
        get(&keyboard, "getLayoutMap").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    let Ok(promise) = get_layout_map
        .call0(&keyboard)
        .and_then(|p| p.dyn_into::<js_sys::Promise>())
    else {
        return;
    };
    let on_map = Closure::once(move |map: JsValue| {
        let Some(for_each) =
            get(&map, "forEach").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        else {
            return;
        };
        let seed = Closure::<dyn FnMut(JsValue, JsValue)>::new(|key: JsValue, code: JsValue| {
            if let (Some(key), Some(code)) = (key.as_string(), code.as_string()) {
                LAYOUT.with(|l| l.borrow_mut().seed(&code, &key));
            }
        });
        let _ = for_each.call1(&map, seed.as_ref());
    });
    // A refusal (a cross-origin frame, a denied permission) leaves the
    // fallbacks; handling it keeps it out of the console.
    let on_refused = Closure::once(|_: JsValue| {});
    let _ = promise.then2(&on_map, &on_refused);
    on_map.forget();
    on_refused.forget();
}

#[cfg(test)]
#[path = "key_layout_tests.rs"]
mod tests;
