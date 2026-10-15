//! The hotkey shortcut grammar: [`Shortcut`], parsed from a string such as
//! `"Ctrl+Alt+Space"`, `"RightCtrl"` or `"Ctrl+RightShift"`.
//!
//! A shortcut names **keys, never characters** — the menu rule (#1160). The
//! key token goes through the same table a menu shortcut does
//! ([`crate::menu::key_spelling_code`]), so `"Ctrl+/"` names the same physical
//! key in both. On top of that table a hotkey adds what a menu has no use
//! for:
//!
//! - **sided modifiers** — `LeftCtrl`/`RightCtrl` (also `LCtrl`, `RCtrl`,
//!   `ControlRight`, …), `RightAlt` (`AltGr`), `LeftShift`, `RightSuper`. A
//!   plain `Ctrl` matches either side.
//! - **modifier-only bindings** — a shortcut whose last token is a modifier:
//!   `"RightCtrl"` (hold Right Ctrl alone), `"Ctrl+Shift"` (Shift pressed
//!   while Ctrl is held).
//! - `Super` / `Meta` / `Win` / `Cmd` as one modifier distinct from Ctrl (a
//!   menu folds Cmd into Ctrl; a system-wide hotkey cannot, because Cmd and
//!   Ctrl are different keys on a Mac). `CmdOrCtrl` is Cmd on macOS and Ctrl
//!   elsewhere.
//! - any W3C UI Events `code` name as the key (`F13`, `Insert`, `Pause`,
//!   `Numpad1`, …).

use std::fmt;
use std::str::FromStr;

use winit::keyboard::KeyCode;

use super::HotkeyError;

/// One of the four modifier keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Modifier {
    /// Control.
    Ctrl,
    /// Alt (Option on a Mac).
    Alt,
    /// Shift.
    Shift,
    /// Super: the Windows key, Command on a Mac, Meta on Linux.
    Super,
}

impl Modifier {
    const ALL: [Modifier; 4] = [
        Modifier::Ctrl,
        Modifier::Alt,
        Modifier::Shift,
        Modifier::Super,
    ];

    fn index(self) -> usize {
        match self {
            Modifier::Ctrl => 0,
            Modifier::Alt => 1,
            Modifier::Shift => 2,
            Modifier::Super => 3,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
            Modifier::Super => "Super",
        }
    }

    /// Which modifier key a physical key is, and on which side.
    pub(crate) fn of_code(code: KeyCode) -> Option<(Modifier, Side)> {
        Some(match code {
            KeyCode::ControlLeft => (Modifier::Ctrl, Side::Left),
            KeyCode::ControlRight => (Modifier::Ctrl, Side::Right),
            KeyCode::AltLeft => (Modifier::Alt, Side::Left),
            KeyCode::AltRight => (Modifier::Alt, Side::Right),
            KeyCode::ShiftLeft => (Modifier::Shift, Side::Left),
            KeyCode::ShiftRight => (Modifier::Shift, Side::Right),
            KeyCode::MetaLeft => (Modifier::Super, Side::Left),
            KeyCode::MetaRight => (Modifier::Super, Side::Right),
            _ => return None,
        })
    }
}

/// Which physical key of a modifier pair a binding asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Either key: `Ctrl`.
    Either,
    /// The left key: `LeftCtrl`.
    Left,
    /// The right key: `RightCtrl`.
    Right,
}

impl Side {
    fn admits(self, held: Side) -> bool {
        self == Side::Either || self == held
    }

    fn prefix(self) -> &'static str {
        match self {
            Side::Either => "",
            Side::Left => "Left",
            Side::Right => "Right",
        }
    }
}

/// What completes a shortcut: an ordinary key, or (for a modifier-only
/// binding) a modifier key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// A non-modifier key, by its W3C `code`.
    Key(KeyCode),
    /// A modifier key: the binding is modifier-only.
    Modifier(Modifier, Side),
}

/// A parsed global hotkey: the modifiers that must be held, and the key that
/// completes it.
///
/// Matching is **exact on modifiers**: `Ctrl+Space` does not fire for
/// `Ctrl+Shift+Space`. Its `Display` is the canonical spelling, which parses
/// back to an equal value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    mods: [Option<Side>; 4],
    trigger: Trigger,
}

impl Shortcut {
    /// Parse a shortcut string. See the module docs for the grammar.
    pub fn parse(s: &str) -> Result<Shortcut, HotkeyError> {
        let err = |why: &str| HotkeyError::Parse(format!("{s:?}: {why}"));
        let tokens: Vec<&str> = s.split('+').map(str::trim).collect();
        if tokens.iter().any(|t| t.is_empty()) {
            return Err(err("empty key name (spell `+` as `Plus` or `Equal`)"));
        }
        let (last, init) = tokens.split_last().ok_or_else(|| err("empty"))?;
        let mut mods: [Option<Side>; 4] = [None; 4];
        for tok in init {
            let Some((m, side)) = parse_modifier(tok) else {
                return Err(if parse_key(tok).is_some() {
                    err("only the last key may be a non-modifier")
                } else {
                    err(&format!("{tok:?} is not a modifier"))
                });
            };
            if mods[m.index()].is_some() {
                return Err(err(&format!("{} named twice", m.name())));
            }
            mods[m.index()] = Some(side);
        }
        let trigger = if let Some((m, side)) = parse_modifier(last) {
            if mods[m.index()].is_some() {
                return Err(err(&format!("{} named twice", m.name())));
            }
            Trigger::Modifier(m, side)
        } else if let Some(code) = parse_key(last) {
            Trigger::Key(code)
        } else {
            return Err(match crate::menu::shifted_key_hint(last) {
                Some(base) => err(&format!(
                    "{last:?} is a character, not a key — spell it as the key and Shift (\"Shift+{base}\" on a US layout)"
                )),
                None => err(&format!("{last:?} names no key")),
            });
        };
        Ok(Shortcut { mods, trigger })
    }

    /// The key that completes the shortcut.
    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// Which side of `m` must be held, or `None` when `m` must not be held.
    pub fn modifier(&self, m: Modifier) -> Option<Side> {
        self.mods[m.index()]
    }

    /// Whether the last key is a modifier (`"RightCtrl"`, `"Ctrl+Shift"`).
    pub fn is_modifier_only(&self) -> bool {
        matches!(self.trigger, Trigger::Modifier(..))
    }

    /// Whether any modifier (held or the trigger) names a side.
    pub fn is_sided(&self) -> bool {
        self.mods
            .iter()
            .any(|m| matches!(m, Some(Side::Left | Side::Right)))
            || matches!(self.trigger, Trigger::Modifier(_, Side::Left | Side::Right))
    }

    /// A plain chord: a non-modifier key with unsided modifiers. The only shape
    /// an OS "hot key" API (`RegisterHotKey`, Carbon, `XGrabKey`) can express.
    pub fn is_plain_chord(&self) -> bool {
        !self.is_modifier_only() && !self.is_sided()
    }

    /// The modifiers that must be held, with sides, in canonical order.
    pub fn held_modifiers(&self) -> impl Iterator<Item = (Modifier, Side)> + '_ {
        Modifier::ALL
            .into_iter()
            .filter_map(|m| self.mods[m.index()].map(|s| (m, s)))
    }

    /// Whether this shortcut is completed by a press of `code` while exactly
    /// `held` (the other keys down right now, `code` excluded) are down.
    pub(crate) fn completes(&self, code: KeyCode, held: &[KeyCode]) -> bool {
        let trigger_ok = match self.trigger {
            Trigger::Key(k) => k == code,
            Trigger::Modifier(m, side) => {
                matches!(Modifier::of_code(code), Some((hm, hs)) if hm == m && side.admits(hs))
            }
        };
        if !trigger_ok {
            return false;
        }
        // Exact modifiers: each held modifier must be one the shortcut asks for
        // (on an admitted side), and each one it asks for must be held.
        let mut seen = [false; 4];
        for &h in held {
            if super::matcher::is_lock_key(h) {
                continue;
            }
            match Modifier::of_code(h) {
                Some((m, s)) => match self.mods[m.index()] {
                    Some(want) if want.admits(s) => seen[m.index()] = true,
                    _ => return false,
                },
                // A modifier-only binding is a modifier alone: another key
                // held means the user is typing a chord, not asking for it.
                None if self.is_modifier_only() => return false,
                None => {}
            }
        }
        Modifier::ALL
            .iter()
            .all(|m| self.mods[m.index()].is_none() || seen[m.index()])
    }

    /// Whether letting go of `code` ends this shortcut once it has fired: its
    /// trigger key, and — for a modifier-only binding, which is nothing but
    /// modifiers — any modifier it holds.
    pub(crate) fn released_by(&self, code: KeyCode) -> bool {
        match self.trigger {
            Trigger::Key(k) => k == code,
            Trigger::Modifier(m, side) => match Modifier::of_code(code) {
                Some((hm, hs)) if hm == m && side.admits(hs) => true,
                Some((hm, hs)) => matches!(self.mods[hm.index()], Some(want) if want.admits(hs)),
                None => false,
            },
        }
    }
}

impl FromStr for Shortcut {
    type Err = HotkeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Shortcut::parse(s)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (m, side) in self.held_modifiers() {
            write!(f, "{}{}+", side.prefix(), m.name())?;
        }
        match self.trigger {
            Trigger::Modifier(m, side) => write!(f, "{}{}", side.prefix(), m.name()),
            Trigger::Key(code) => f.write_str(&key_display(code)),
        }
    }
}

/// The canonical spelling of a key: `"A"` for `KeyA`, `"1"` for `Digit1`, the
/// W3C name otherwise. Every one parses back.
pub(crate) fn key_display(code: KeyCode) -> String {
    let name = code.to_string();
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        return digit.to_string();
    }
    name
}

/// The modifier a token names, with its side.
pub(crate) fn parse_modifier(tok: &str) -> Option<(Modifier, Side)> {
    let t = tok.to_ascii_lowercase();
    let (side, base) = if let Some(rest) = t.strip_prefix("left") {
        (Side::Left, rest)
    } else if let Some(rest) = t.strip_prefix("right") {
        (Side::Right, rest)
    } else if let Some(rest) = t.strip_suffix("left") {
        (Side::Left, rest)
    } else if let Some(rest) = t.strip_suffix("right") {
        (Side::Right, rest)
    } else if t.len() > 1 && t.starts_with('l') && parse_bare_modifier(&t[1..]).is_some() {
        (Side::Left, &t[1..])
    } else if t.len() > 1 && t.starts_with('r') && parse_bare_modifier(&t[1..]).is_some() {
        (Side::Right, &t[1..])
    } else if t == "altgr" {
        return Some((Modifier::Alt, Side::Right));
    } else {
        (Side::Either, t.as_str())
    };
    let m = parse_bare_modifier(base)?;
    Some((m, side))
}

fn parse_bare_modifier(base: &str) -> Option<Modifier> {
    Some(match base {
        "ctrl" | "control" => Modifier::Ctrl,
        "alt" | "option" | "opt" => Modifier::Alt,
        "shift" => Modifier::Shift,
        "super" | "meta" | "win" | "windows" | "cmd" | "command" | "os" | "logo" => Modifier::Super,
        "cmdorctrl" | "cmdorcontrol" | "commandorctrl" | "commandorcontrol" => {
            if cfg!(target_os = "macos") {
                Modifier::Super
            } else {
                Modifier::Ctrl
            }
        }
        _ => return None,
    })
}

/// The non-modifier key a token names.
pub(crate) fn parse_key(tok: &str) -> Option<KeyCode> {
    if let Some(code) = crate::menu::key_spelling_code(tok) {
        return KeyCode::from_str(code).ok();
    }
    // Any W3C `code` name, as written or with its first letter capitalised
    // (`f13`, `insert`, `numpad1`).
    let mut cap = tok.to_string();
    if let Some(first) = cap.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let code = KeyCode::from_str(tok)
        .ok()
        .or_else(|| KeyCode::from_str(&cap).ok())?;
    if Modifier::of_code(code).is_some()
        || matches!(code, KeyCode::Fn | KeyCode::FnLock | KeyCode::Unidentified)
    {
        return None;
    }
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Shortcut {
        Shortcut::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn plain_chords_parse_and_print_canonically() {
        assert_eq!(p("ctrl+alt+space").to_string(), "Ctrl+Alt+Space");
        assert_eq!(p("Shift+Ctrl+k").to_string(), "Ctrl+Shift+K");
        assert_eq!(p("Super+1").to_string(), "Super+1");
        assert_eq!(p("Win+F13").to_string(), "Super+F13");
        assert_eq!(p("Ctrl+/").to_string(), "Ctrl+Slash");
        assert_eq!(p("Alt+numpad1").to_string(), "Alt+Numpad1");
        assert!(p("Ctrl+Alt+Space").is_plain_chord());
    }

    #[test]
    fn sided_and_modifier_only_bindings() {
        let r = p("RightCtrl");
        assert_eq!(r.trigger(), Trigger::Modifier(Modifier::Ctrl, Side::Right));
        assert!(r.is_modifier_only() && r.is_sided() && !r.is_plain_chord());
        for spelling in [
            "rctrl",
            "ControlRight",
            "right ctrl".replace(' ', "").as_str(),
            "RIGHTCONTROL",
        ] {
            assert_eq!(p(spelling), r, "{spelling}");
        }
        assert_eq!(p("AltGr"), p("RightAlt"));
        assert_eq!(p("ctrl+shift").to_string(), "Ctrl+Shift");
        assert_eq!(p("LeftShift+Space").to_string(), "LeftShift+Space");
        assert!(!p("LeftShift+Space").is_plain_chord());
    }

    #[test]
    fn every_canonical_spelling_round_trips() {
        for s in [
            "Ctrl+Alt+Space",
            "RightCtrl",
            "LeftAlt+RightShift",
            "Super+F24",
            "Ctrl+Backquote",
            "Shift+Equal",
            "Alt+PageDown",
            "Ctrl+Insert",
            "Pause",
            "F5",
            "Ctrl+Super",
        ] {
            assert_eq!(p(s).to_string(), s);
            assert_eq!(p(&p(s).to_string()), p(s));
        }
    }

    #[test]
    fn malformed_shortcuts_are_errors() {
        for bad in [
            "",
            "Ctrl+",
            "Ctrl++",
            "A+Ctrl",
            "Ctrl+Ctrl",
            "Ctrl+RightCtrl",
            "Hyper+N",
            "Ctrl+?",
            "Ctrl+A+B",
            "Fn",
        ] {
            assert!(Shortcut::parse(bad).is_err(), "{bad:?} parsed");
        }
        let msg = Shortcut::parse("Ctrl+?").unwrap_err().to_string();
        assert!(msg.contains("Shift+/"), "{msg}");
    }

    /// The hotkey grammar reads keys through the menu's table, so every key a
    /// menu shortcut can name is a key a hotkey can name, and to the same
    /// physical key — pinned for every menu spelling, not a sample.
    #[test]
    fn every_menu_key_spelling_names_the_same_key_here() {
        let mut checked = 0;
        let source = include_str!("../menu/mod.rs");
        let body = source
            .split("pub(crate) fn key_spelling_code")
            .nth(1)
            .and_then(|rest| rest.split("_ => return None").next())
            .expect("key_spelling_code in menu/mod.rs");
        let spellings = body
            .lines()
            .filter_map(|line| line.split_once(" => "))
            .flat_map(|(lhs, _)| {
                lhs.split('|')
                    .map(|t| t.trim().trim_matches('"').replace("\\\\", "\\"))
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
            });
        for tok in spellings {
            let tok = tok.as_str();
            let menu = crate::menu::key_spelling_code(tok)
                .unwrap_or_else(|| panic!("menu spelling {tok:?}"));
            let here = parse_key(tok).unwrap_or_else(|| panic!("hotkey cannot name {tok:?}"));
            assert_eq!(here.to_string(), menu, "{tok}");
            checked += 1;
        }
        assert!(checked > 90, "the spelling list was not read ({checked})");
    }

    #[test]
    fn completion_is_exact_on_modifiers_and_sides() {
        use KeyCode::*;
        let chord = p("Ctrl+Space");
        assert!(chord.completes(Space, &[ControlLeft]));
        assert!(chord.completes(Space, &[ControlRight]));
        assert!(!chord.completes(Space, &[]));
        assert!(!chord.completes(Space, &[ControlLeft, ShiftLeft]));
        assert!(
            chord.completes(Space, &[ControlLeft, KeyA]),
            "a non-modifier held does not block a chord"
        );

        let sided = p("RightCtrl+Space");
        assert!(sided.completes(Space, &[ControlRight]));
        assert!(!sided.completes(Space, &[ControlLeft]));

        let rctrl = p("RightCtrl");
        assert!(rctrl.completes(ControlRight, &[]));
        assert!(!rctrl.completes(ControlLeft, &[]));
        assert!(!rctrl.completes(ControlRight, &[ShiftLeft]));
        assert!(
            !rctrl.completes(ControlRight, &[KeyC]),
            "typing a chord is not a modifier-only press"
        );

        let cs = p("Ctrl+Shift");
        assert!(cs.completes(ShiftRight, &[ControlLeft]));
        assert!(
            !cs.completes(ControlLeft, &[ShiftRight]),
            "the trigger is the last modifier, Shift"
        );
        assert!(
            cs.released_by(ControlLeft) && cs.released_by(ShiftLeft) && !cs.released_by(AltLeft)
        );
        assert!(chord.released_by(Space) && !chord.released_by(ControlLeft));
    }
}
