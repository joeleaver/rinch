//! Event conversion utilities for winit to blitz event types.

use blitz_traits::events::{BlitzImeEvent, BlitzKeyEvent, KeyState};
use keyboard_types::{Code, Key, Location, Modifiers};
use winit::event::ElementState;
use winit::event::Ime;
use winit::event::KeyEvent as WinitKeyEvent;
use winit::keyboard::Key as WinitKey;
use winit::keyboard::KeyCode as WinitKeyCode;
use winit::keyboard::KeyLocation as WinitKeyLocation;
use winit::keyboard::ModifiersState as WinitModifiers;
use winit::keyboard::NamedKey as WinitNamedKey;
use winit::keyboard::PhysicalKey as WinitPhysicalKey;

pub fn winit_ime_to_blitz(event: Ime) -> BlitzImeEvent {
    match event {
        Ime::Enabled => BlitzImeEvent::Enabled,
        Ime::Disabled => BlitzImeEvent::Disabled,
        Ime::Preedit(text, cursor) => BlitzImeEvent::Preedit(text, cursor),
        Ime::Commit(text) => BlitzImeEvent::Commit(text),
    }
}

pub fn winit_key_event_to_blitz(event: &WinitKeyEvent, mods: WinitModifiers) -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: winit_key_to_kbt_key(&event.logical_key),
        code: winit_physical_key_to_kbt_code(&event.physical_key),
        modifiers: winit_modifiers_to_kbt_modifiers(mods),
        location: winit_key_location_to_kbt_location(event.location),
        is_auto_repeating: event.repeat,
        is_composing: false,
        state: match event.state {
            ElementState::Pressed => KeyState::Pressed,
            ElementState::Released => KeyState::Released,
        },
        // Convert from winit's smol_str 0.2.2 to blitz's smol_str 0.3.5
        text: event.text.as_ref().map(|s| s.as_str().into()),
    }
}

pub fn winit_modifiers_to_kbt_modifiers(winit_modifiers: WinitModifiers) -> Modifiers {
    let mut modifiers = Modifiers::default();
    if winit_modifiers.control_key() {
        modifiers.insert(Modifiers::CONTROL);
    }
    if winit_modifiers.alt_key() {
        modifiers.insert(Modifiers::ALT);
    }
    if winit_modifiers.shift_key() {
        modifiers.insert(Modifiers::SHIFT);
    }
    if winit_modifiers.super_key() {
        modifiers.insert(Modifiers::SUPER);
    }
    modifiers
}

fn winit_key_location_to_kbt_location(location: WinitKeyLocation) -> Location {
    match location {
        WinitKeyLocation::Standard => Location::Standard,
        WinitKeyLocation::Left => Location::Left,
        WinitKeyLocation::Right => Location::Right,
        WinitKeyLocation::Numpad => Location::Numpad,
    }
}

fn winit_physical_key_to_kbt_code(physical_key: &WinitPhysicalKey) -> Code {
    match physical_key {
        WinitPhysicalKey::Unidentified(_) => Code::Unidentified,
        WinitPhysicalKey::Code(key_code) => match key_code {
            // Variants that don't match 1:1
            WinitKeyCode::Meta => Code::Super,
            WinitKeyCode::SuperLeft => Code::Super,
            WinitKeyCode::SuperRight => Code::Super,

            WinitKeyCode::Backquote => Code::Backquote,
            WinitKeyCode::Backslash => Code::Backslash,
            WinitKeyCode::BracketLeft => Code::BracketLeft,
            WinitKeyCode::BracketRight => Code::BracketRight,
            WinitKeyCode::Comma => Code::Comma,
            WinitKeyCode::Digit0 => Code::Digit0,
            WinitKeyCode::Digit1 => Code::Digit1,
            WinitKeyCode::Digit2 => Code::Digit2,
            WinitKeyCode::Digit3 => Code::Digit3,
            WinitKeyCode::Digit4 => Code::Digit4,
            WinitKeyCode::Digit5 => Code::Digit5,
            WinitKeyCode::Digit6 => Code::Digit6,
            WinitKeyCode::Digit7 => Code::Digit7,
            WinitKeyCode::Digit8 => Code::Digit8,
            WinitKeyCode::Digit9 => Code::Digit9,
            WinitKeyCode::Equal => Code::Equal,
            WinitKeyCode::IntlBackslash => Code::IntlBackslash,
            WinitKeyCode::IntlRo => Code::IntlRo,
            WinitKeyCode::IntlYen => Code::IntlYen,
            WinitKeyCode::KeyA => Code::KeyA,
            WinitKeyCode::KeyB => Code::KeyB,
            WinitKeyCode::KeyC => Code::KeyC,
            WinitKeyCode::KeyD => Code::KeyD,
            WinitKeyCode::KeyE => Code::KeyE,
            WinitKeyCode::KeyF => Code::KeyF,
            WinitKeyCode::KeyG => Code::KeyG,
            WinitKeyCode::KeyH => Code::KeyH,
            WinitKeyCode::KeyI => Code::KeyI,
            WinitKeyCode::KeyJ => Code::KeyJ,
            WinitKeyCode::KeyK => Code::KeyK,
            WinitKeyCode::KeyL => Code::KeyL,
            WinitKeyCode::KeyM => Code::KeyM,
            WinitKeyCode::KeyN => Code::KeyN,
            WinitKeyCode::KeyO => Code::KeyO,
            WinitKeyCode::KeyP => Code::KeyP,
            WinitKeyCode::KeyQ => Code::KeyQ,
            WinitKeyCode::KeyR => Code::KeyR,
            WinitKeyCode::KeyS => Code::KeyS,
            WinitKeyCode::KeyT => Code::KeyT,
            WinitKeyCode::KeyU => Code::KeyU,
            WinitKeyCode::KeyV => Code::KeyV,
            WinitKeyCode::KeyW => Code::KeyW,
            WinitKeyCode::KeyX => Code::KeyX,
            WinitKeyCode::KeyY => Code::KeyY,
            WinitKeyCode::KeyZ => Code::KeyZ,
            WinitKeyCode::Minus => Code::Minus,
            WinitKeyCode::Period => Code::Period,
            WinitKeyCode::Quote => Code::Quote,
            WinitKeyCode::Semicolon => Code::Semicolon,
            WinitKeyCode::Slash => Code::Slash,
            WinitKeyCode::AltLeft => Code::AltLeft,
            WinitKeyCode::AltRight => Code::AltRight,
            WinitKeyCode::Backspace => Code::Backspace,
            WinitKeyCode::CapsLock => Code::CapsLock,
            WinitKeyCode::ContextMenu => Code::ContextMenu,
            WinitKeyCode::ControlLeft => Code::ControlLeft,
            WinitKeyCode::ControlRight => Code::ControlRight,
            WinitKeyCode::Enter => Code::Enter,
            WinitKeyCode::ShiftLeft => Code::ShiftLeft,
            WinitKeyCode::ShiftRight => Code::ShiftRight,
            WinitKeyCode::Space => Code::Space,
            WinitKeyCode::Tab => Code::Tab,
            WinitKeyCode::Convert => Code::Convert,
            WinitKeyCode::KanaMode => Code::KanaMode,
            WinitKeyCode::Lang1 => Code::Lang1,
            WinitKeyCode::Lang2 => Code::Lang2,
            WinitKeyCode::Lang3 => Code::Lang3,
            WinitKeyCode::Lang4 => Code::Lang4,
            WinitKeyCode::Lang5 => Code::Lang5,
            WinitKeyCode::NonConvert => Code::NonConvert,
            WinitKeyCode::Delete => Code::Delete,
            WinitKeyCode::End => Code::End,
            WinitKeyCode::Help => Code::Help,
            WinitKeyCode::Home => Code::Home,
            WinitKeyCode::Insert => Code::Insert,
            WinitKeyCode::PageDown => Code::PageDown,
            WinitKeyCode::PageUp => Code::PageUp,
            WinitKeyCode::ArrowDown => Code::ArrowDown,
            WinitKeyCode::ArrowLeft => Code::ArrowLeft,
            WinitKeyCode::ArrowRight => Code::ArrowRight,
            WinitKeyCode::ArrowUp => Code::ArrowUp,
            WinitKeyCode::NumLock => Code::NumLock,
            WinitKeyCode::Numpad0 => Code::Numpad0,
            WinitKeyCode::Numpad1 => Code::Numpad1,
            WinitKeyCode::Numpad2 => Code::Numpad2,
            WinitKeyCode::Numpad3 => Code::Numpad3,
            WinitKeyCode::Numpad4 => Code::Numpad4,
            WinitKeyCode::Numpad5 => Code::Numpad5,
            WinitKeyCode::Numpad6 => Code::Numpad6,
            WinitKeyCode::Numpad7 => Code::Numpad7,
            WinitKeyCode::Numpad8 => Code::Numpad8,
            WinitKeyCode::Numpad9 => Code::Numpad9,
            WinitKeyCode::NumpadAdd => Code::NumpadAdd,
            WinitKeyCode::NumpadBackspace => Code::NumpadBackspace,
            WinitKeyCode::NumpadClear => Code::NumpadClear,
            WinitKeyCode::NumpadClearEntry => Code::NumpadClearEntry,
            WinitKeyCode::NumpadComma => Code::NumpadComma,
            WinitKeyCode::NumpadDecimal => Code::NumpadDecimal,
            WinitKeyCode::NumpadDivide => Code::NumpadDivide,
            WinitKeyCode::NumpadEnter => Code::NumpadEnter,
            WinitKeyCode::NumpadEqual => Code::NumpadEqual,
            WinitKeyCode::NumpadHash => Code::NumpadHash,
            WinitKeyCode::NumpadMemoryAdd => Code::NumpadMemoryAdd,
            WinitKeyCode::NumpadMemoryClear => Code::NumpadMemoryClear,
            WinitKeyCode::NumpadMemoryRecall => Code::NumpadMemoryRecall,
            WinitKeyCode::NumpadMemoryStore => Code::NumpadMemoryStore,
            WinitKeyCode::NumpadMemorySubtract => Code::NumpadMemorySubtract,
            WinitKeyCode::NumpadMultiply => Code::NumpadMultiply,
            WinitKeyCode::NumpadParenLeft => Code::NumpadParenLeft,
            WinitKeyCode::NumpadParenRight => Code::NumpadParenRight,
            WinitKeyCode::NumpadStar => Code::NumpadStar,
            WinitKeyCode::NumpadSubtract => Code::NumpadSubtract,
            WinitKeyCode::Escape => Code::Escape,
            WinitKeyCode::Fn => Code::Fn,
            WinitKeyCode::FnLock => Code::FnLock,
            WinitKeyCode::PrintScreen => Code::PrintScreen,
            WinitKeyCode::ScrollLock => Code::ScrollLock,
            WinitKeyCode::Pause => Code::Pause,
            WinitKeyCode::BrowserBack => Code::BrowserBack,
            WinitKeyCode::BrowserFavorites => Code::BrowserFavorites,
            WinitKeyCode::BrowserForward => Code::BrowserForward,
            WinitKeyCode::BrowserHome => Code::BrowserHome,
            WinitKeyCode::BrowserRefresh => Code::BrowserRefresh,
            WinitKeyCode::BrowserSearch => Code::BrowserSearch,
            WinitKeyCode::BrowserStop => Code::BrowserStop,
            WinitKeyCode::Eject => Code::Eject,
            WinitKeyCode::LaunchApp1 => Code::LaunchApp1,
            WinitKeyCode::LaunchApp2 => Code::LaunchApp2,
            WinitKeyCode::LaunchMail => Code::LaunchMail,
            WinitKeyCode::MediaPlayPause => Code::MediaPlayPause,
            WinitKeyCode::MediaSelect => Code::MediaSelect,
            WinitKeyCode::MediaStop => Code::MediaStop,
            WinitKeyCode::MediaTrackNext => Code::MediaTrackNext,
            WinitKeyCode::MediaTrackPrevious => Code::MediaTrackPrevious,
            WinitKeyCode::Power => Code::Power,
            WinitKeyCode::Sleep => Code::Sleep,
            WinitKeyCode::AudioVolumeDown => Code::AudioVolumeDown,
            WinitKeyCode::AudioVolumeMute => Code::AudioVolumeMute,
            WinitKeyCode::AudioVolumeUp => Code::AudioVolumeUp,
            WinitKeyCode::WakeUp => Code::WakeUp,
            WinitKeyCode::Hyper => Code::Hyper,
            WinitKeyCode::Turbo => Code::Turbo,
            WinitKeyCode::Abort => Code::Abort,
            WinitKeyCode::Resume => Code::Resume,
            WinitKeyCode::Suspend => Code::Suspend,
            WinitKeyCode::Again => Code::Again,
            WinitKeyCode::Copy => Code::Copy,
            WinitKeyCode::Cut => Code::Cut,
            WinitKeyCode::Find => Code::Find,
            WinitKeyCode::Open => Code::Open,
            WinitKeyCode::Paste => Code::Paste,
            WinitKeyCode::Props => Code::Props,
            WinitKeyCode::Select => Code::Select,
            WinitKeyCode::Undo => Code::Undo,
            WinitKeyCode::Hiragana => Code::Hiragana,
            WinitKeyCode::Katakana => Code::Katakana,
            WinitKeyCode::F1 => Code::F1,
            WinitKeyCode::F2 => Code::F2,
            WinitKeyCode::F3 => Code::F3,
            WinitKeyCode::F4 => Code::F4,
            WinitKeyCode::F5 => Code::F5,
            WinitKeyCode::F6 => Code::F6,
            WinitKeyCode::F7 => Code::F7,
            WinitKeyCode::F8 => Code::F8,
            WinitKeyCode::F9 => Code::F9,
            WinitKeyCode::F10 => Code::F10,
            WinitKeyCode::F11 => Code::F11,
            WinitKeyCode::F12 => Code::F12,
            WinitKeyCode::F13 => Code::F13,
            WinitKeyCode::F14 => Code::F14,
            WinitKeyCode::F15 => Code::F15,
            WinitKeyCode::F16 => Code::F16,
            WinitKeyCode::F17 => Code::F17,
            WinitKeyCode::F18 => Code::F18,
            WinitKeyCode::F19 => Code::F19,
            WinitKeyCode::F20 => Code::F20,
            WinitKeyCode::F21 => Code::F21,
            WinitKeyCode::F22 => Code::F22,
            WinitKeyCode::F23 => Code::F23,
            WinitKeyCode::F24 => Code::F24,
            WinitKeyCode::F25 => Code::F25,
            WinitKeyCode::F26 => Code::F26,
            WinitKeyCode::F27 => Code::F27,
            WinitKeyCode::F28 => Code::F28,
            WinitKeyCode::F29 => Code::F29,
            WinitKeyCode::F30 => Code::F30,
            WinitKeyCode::F31 => Code::F31,
            WinitKeyCode::F32 => Code::F32,
            WinitKeyCode::F33 => Code::F33,
            WinitKeyCode::F34 => Code::F34,
            WinitKeyCode::F35 => Code::F35,
            _ => Code::Unidentified,
        },
    }
}

use rinch_core::events::KeyEventData;

/// Convert a winit key event + modifiers into a KeyEventData for the keyboard interceptor.
pub fn winit_key_to_key_event_data(event: &WinitKeyEvent, mods: WinitModifiers) -> KeyEventData {
    let key = match &event.logical_key {
        WinitKey::Character(c) => c.to_string(),
        WinitKey::Named(n) => format!("{:?}", n),
        _ => "Unidentified".to_string(),
    };
    let code = match &event.physical_key {
        WinitPhysicalKey::Code(c) => format!("{:?}", c),
        _ => "Unidentified".to_string(),
    };
    KeyEventData {
        key,
        code,
        ctrl: mods.control_key(),
        shift: mods.shift_key(),
        alt: mods.alt_key(),
        meta: mods.super_key(),
    }
}

fn winit_key_to_kbt_key(winit_key: &WinitKey) -> Key {
    match winit_key {
        WinitKey::Character(c) => Key::Character(c.to_string()),
        WinitKey::Unidentified(_) => Key::Unidentified,
        WinitKey::Dead(_) => Key::Dead,
        WinitKey::Named(named_key) => match named_key {
            WinitNamedKey::Alt => Key::Alt,
            WinitNamedKey::AltGraph => Key::AltGraph,
            WinitNamedKey::CapsLock => Key::CapsLock,
            WinitNamedKey::Control => Key::Control,
            WinitNamedKey::Fn => Key::Fn,
            WinitNamedKey::FnLock => Key::FnLock,
            WinitNamedKey::NumLock => Key::NumLock,
            WinitNamedKey::ScrollLock => Key::ScrollLock,
            WinitNamedKey::Shift => Key::Shift,
            WinitNamedKey::Symbol => Key::Symbol,
            WinitNamedKey::SymbolLock => Key::SymbolLock,
            WinitNamedKey::Meta => Key::Meta,
            WinitNamedKey::Hyper => Key::Hyper,
            WinitNamedKey::Super => Key::Super,
            WinitNamedKey::Enter => Key::Enter,
            WinitNamedKey::Tab => Key::Tab,
            WinitNamedKey::Space => Key::Character(" ".to_string()),
            WinitNamedKey::ArrowDown => Key::ArrowDown,
            WinitNamedKey::ArrowLeft => Key::ArrowLeft,
            WinitNamedKey::ArrowRight => Key::ArrowRight,
            WinitNamedKey::ArrowUp => Key::ArrowUp,
            WinitNamedKey::End => Key::End,
            WinitNamedKey::Home => Key::Home,
            WinitNamedKey::PageDown => Key::PageDown,
            WinitNamedKey::PageUp => Key::PageUp,
            WinitNamedKey::Backspace => Key::Backspace,
            WinitNamedKey::Clear => Key::Clear,
            WinitNamedKey::Copy => Key::Copy,
            WinitNamedKey::CrSel => Key::CrSel,
            WinitNamedKey::Cut => Key::Cut,
            WinitNamedKey::Delete => Key::Delete,
            WinitNamedKey::EraseEof => Key::EraseEof,
            WinitNamedKey::ExSel => Key::ExSel,
            WinitNamedKey::Insert => Key::Insert,
            WinitNamedKey::Paste => Key::Paste,
            WinitNamedKey::Redo => Key::Redo,
            WinitNamedKey::Undo => Key::Undo,
            WinitNamedKey::Accept => Key::Accept,
            WinitNamedKey::Again => Key::Again,
            WinitNamedKey::Attn => Key::Attn,
            WinitNamedKey::Cancel => Key::Cancel,
            WinitNamedKey::ContextMenu => Key::ContextMenu,
            WinitNamedKey::Escape => Key::Escape,
            WinitNamedKey::Execute => Key::Execute,
            WinitNamedKey::Find => Key::Find,
            WinitNamedKey::Help => Key::Help,
            WinitNamedKey::Pause => Key::Pause,
            WinitNamedKey::Play => Key::Play,
            WinitNamedKey::Props => Key::Props,
            WinitNamedKey::Select => Key::Select,
            WinitNamedKey::ZoomIn => Key::ZoomIn,
            WinitNamedKey::ZoomOut => Key::ZoomOut,
            WinitNamedKey::F1 => Key::F1,
            WinitNamedKey::F2 => Key::F2,
            WinitNamedKey::F3 => Key::F3,
            WinitNamedKey::F4 => Key::F4,
            WinitNamedKey::F5 => Key::F5,
            WinitNamedKey::F6 => Key::F6,
            WinitNamedKey::F7 => Key::F7,
            WinitNamedKey::F8 => Key::F8,
            WinitNamedKey::F9 => Key::F9,
            WinitNamedKey::F10 => Key::F10,
            WinitNamedKey::F11 => Key::F11,
            WinitNamedKey::F12 => Key::F12,
            _ => Key::Unidentified,
        },
    }
}
