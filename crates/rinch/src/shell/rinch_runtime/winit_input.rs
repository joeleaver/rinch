//! Translating winit's input into rinch's: pointer sources, buttons, wheels, keys and modifiers.

use super::*;

// ── Pointer sources ──────────────────────────────────────────────────────────

/// Touch pointers are numbered from here, so a finger never shares an id
/// with the mouse (1) or a tablet tool (2).
pub(super) const TOUCH_POINTER_BASE: u64 = 1 << 32;

/// The [`PlatformEvent`] a winit wheel delta becomes, with the pointer at the
/// logical `cursor` on a window of `scale`: what both windows of the desktop
/// shell send (#2287).
///
/// - A `LineDelta` — a mouse wheel's notches — is a
///   [`PlatformEvent::MouseWheelLines`], whose default scroll glides as
///   Chrome's does.
/// - A `PixelDelta` — a trackpad, a high-resolution wheel — is a
///   [`PlatformEvent::MouseWheel`], which scrolls directly. The delta is a
///   *physical* distance compared against logical scroll extents, so it
///   divides like a position does, or a HiDPI trackpad scrolls `scale` times
///   too far (#299).
/// - Any other kind (`MouseScrollDelta` is non-exhaustive) is `None`: it
///   scrolls nothing.
pub fn wheel_platform_event(
    delta: winit::event::MouseScrollDelta,
    cursor: (f32, f32),
    scale: f64,
) -> Option<PlatformEvent> {
    let (x, y) = cursor;
    match delta {
        winit::event::MouseScrollDelta::LineDelta(lines_x, lines_y) => {
            Some(PlatformEvent::MouseWheelLines {
                x,
                y,
                lines_x: lines_x as f64,
                lines_y: lines_y as f64,
            })
        }
        winit::event::MouseScrollDelta::PixelDelta(pos) => {
            let (delta_x, delta_y) = to_logical_point((pos.x, pos.y), scale);
            Some(PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            })
        }
        _ => None,
    }
}

/// A winit force as a pressure in `0.0..=1.0`; with none measured, the
/// Pointer Events default (`0.5` while down, else `0`).
fn force_pressure(force: Option<&winit::event::Force>, down: bool) -> f32 {
    match force {
        Some(f) => (f.normalized(None) as f32).clamp(0.0, 1.0),
        None if down => 0.5,
        None => 0.0,
    }
}

/// A tablet tool's kind as a surface pointer's: an eraser is an eraser, a
/// puck or lens a mouse, a finger on the tablet a touch, every stylus a pen.
fn tool_kind(kind: winit::event::TabletToolKind) -> SurfacePointerKind {
    match kind {
        winit::event::TabletToolKind::Eraser => SurfacePointerKind::Eraser,
        winit::event::TabletToolKind::Mouse | winit::event::TabletToolKind::Lens => {
            SurfacePointerKind::Mouse
        }
        winit::event::TabletToolKind::Finger => SurfacePointerKind::Touch,
        _ => SurfacePointerKind::Pen,
    }
}

/// The one id a tablet tool has: winit numbers neither tools nor tablets.
const TABLET_POINTER_ID: u64 = 2;

/// [`SurfacePointer::id`] of the pointer a winit source describes.
pub(super) fn pointer_id_of(source: &winit::event::PointerSource) -> u64 {
    match source {
        winit::event::PointerSource::Touch { finger_id, .. } => {
            TOUCH_POINTER_BASE + finger_id.into_raw() as u64
        }
        winit::event::PointerSource::TabletTool { .. } => TABLET_POINTER_ID,
        _ => SurfacePointer::mouse(false).id,
    }
}

/// `os_file_drag`'s steps carried out on winit's event loop (#1962); the
/// events are collected and handed to the app after, in order.
pub(super) struct WinitDragHost<'a> {
    pub(super) event_loop: &'a dyn ActiveEventLoop,
    pub(super) events: Vec<PlatformEvent>,
}

impl super::os_file_drag::DragHost for WinitDragHost<'_> {
    fn accept(
        &mut self,
        id: winit::data_transfer::DataTransferId,
        actions: &[winit::event_loop::DndAction],
    ) -> Result<(), String> {
        self.event_loop
            .set_valid_dnd_actions(id, actions)
            .map_err(|e| e.to_string())
    }

    fn fetch_uri_list(
        &mut self,
        id: winit::data_transfer::DataTransferId,
    ) -> Result<winit::event_loop::AsyncRequestSerial, String> {
        self.event_loop
            .fetch_data_transfer(id, &winit::data_transfer::TypeHint::UriList)
            .map_err(|e| e.to_string())
    }

    fn emit(&mut self, event: PlatformEvent) {
        self.events.push(event);
    }

    fn warn(&mut self, message: String) {
        tracing::warn!("{message}");
    }
}

/// The pointer a winit move came from, as a render surface hears it. `down`:
/// that pointer has a button or contact down, which decides the pressure a
/// device that cannot measure it reports.
pub(super) fn surface_pointer_of(
    source: &winit::event::PointerSource,
    primary: bool,
    down: bool,
) -> SurfacePointer {
    use winit::event::PointerSource;
    match source {
        PointerSource::Mouse | PointerSource::Unknown => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
        PointerSource::Touch { finger_id, force } => SurfacePointer {
            id: TOUCH_POINTER_BASE + finger_id.into_raw() as u64,
            kind: SurfacePointerKind::Touch,
            pressure: force_pressure(force.as_ref(), down),
            primary,
        },
        PointerSource::TabletTool { kind, data } => SurfacePointer {
            id: TABLET_POINTER_ID,
            kind: tool_kind(*kind),
            pressure: force_pressure(data.force.as_ref(), down),
            primary,
        },
        // `PointerSource` is non-exhaustive (winit 0.31.0-beta.3): a source
        // this shell does not know is a mouse, as `Unknown` is.
        _ => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
    }
}

/// [`surface_pointer_of`] for a winit button event. `pressed`: the button
/// went down (a release reports no pressure).
pub(super) fn surface_pointer_of_button(
    button: &winit::event::ButtonSource,
    primary: bool,
    pressed: bool,
) -> SurfacePointer {
    use winit::event::{ButtonSource, PointerSource};
    let source = match button {
        ButtonSource::Mouse(_) | ButtonSource::Unknown(_) => PointerSource::Mouse,
        ButtonSource::Touch { finger_id, force } => PointerSource::Touch {
            finger_id: *finger_id,
            force: *force,
        },
        ButtonSource::TabletTool { kind, data, .. } => PointerSource::TabletTool {
            kind: *kind,
            data: data.clone(),
        },
        // Non-exhaustive (winit 0.31.0-beta.3): an unknown source is a mouse.
        _ => PointerSource::Mouse,
    };
    let mut pointer = surface_pointer_of(&source, primary, pressed);
    if !pressed {
        pointer.pressure = 0.0;
    }
    pointer
}

/// The buttons or contacts down, as `(SurfacePointer::id, button)`: what
/// decides the pressure a move reports for a device that cannot measure it.
/// Per button, so a chord's release (right up while left is held) leaves the
/// pointer down.
#[derive(Debug, Default)]
pub(super) struct PressedButtons(Vec<(u64, Option<MouseButton>)>);

impl PressedButtons {
    /// `button` of pointer `id` went down (`pressed`) or up.
    pub(super) fn note(&mut self, id: u64, button: Option<MouseButton>, pressed: bool) {
        self.0.retain(|k| *k != (id, button));
        if pressed {
            self.0.push((id, button));
        }
    }

    /// Whether pointer `id` has any button or contact down.
    pub(super) fn is_down(&self, id: u64) -> bool {
        self.0.iter().any(|(p, _)| *p == id)
    }

    /// Whether the primary button (a mouse's left, a finger, a pen's tip) of
    /// any pointer is down.
    pub(super) fn primary_down(&self) -> bool {
        self.0.iter().any(|(_, b)| *b == Some(MouseButton::Left))
    }

    /// Forget every button of pointer `id`; answers whether it had any down.
    pub(super) fn lift(&mut self, id: u64) -> bool {
        let was = self.is_down(id);
        self.0.retain(|(p, _)| *p != id);
        was
    }

    /// Forget every press (the window lost focus).
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

/// Resets the pointer `set_current_pointer` named when dropped.
pub(super) struct ForgetCurrentPointer;

impl Drop for ForgetCurrentPointer {
    fn drop(&mut self) {
        crate::render_surface::set_current_pointer(None);
    }
}

impl RinchRuntime {
    /// Convert a platform CursorStyle to a winit CursorIcon.
    pub(super) fn cursor_style_to_winit(
        style: rinch_platform::CursorStyle,
    ) -> winit::cursor::CursorIcon {
        use rinch_platform::CursorStyle as CS;
        use winit::cursor::CursorIcon;
        match style {
            CS::Auto | CS::Default => CursorIcon::Default,
            CS::Pointer => CursorIcon::Pointer,
            CS::Text => CursorIcon::Text,
            CS::Move => CursorIcon::Move,
            CS::NotAllowed => CursorIcon::NotAllowed,
            CS::Crosshair => CursorIcon::Crosshair,
            CS::Grab => CursorIcon::Grab,
            CS::Grabbing => CursorIcon::Grabbing,
            CS::ColResize => CursorIcon::ColResize,
            CS::RowResize => CursorIcon::RowResize,
            CS::NResize => CursorIcon::NResize,
            CS::SResize => CursorIcon::SResize,
            CS::EResize => CursorIcon::EResize,
            CS::WResize => CursorIcon::WResize,
            CS::NeResize => CursorIcon::NeResize,
            CS::NwResize => CursorIcon::NwResize,
            CS::SeResize => CursorIcon::SeResize,
            CS::SwResize => CursorIcon::SwResize,
            CS::EwResize => CursorIcon::EwResize,
            CS::NsResize => CursorIcon::NsResize,
            CS::NeswResize => CursorIcon::NeswResize,
            CS::NwseResize => CursorIcon::NwseResize,
            CS::ZoomIn => CursorIcon::ZoomIn,
            CS::ZoomOut => CursorIcon::ZoomOut,
            CS::Wait => CursorIcon::Wait,
            CS::Progress => CursorIcon::Progress,
            CS::Help => CursorIcon::Help,
            CS::ContextMenu => CursorIcon::ContextMenu,
            CS::Cell => CursorIcon::Cell,
            CS::Copy => CursorIcon::Copy,
            CS::Alias => CursorIcon::Alias,
            CS::NoDrop => CursorIcon::NoDrop,
            CS::AllScroll => CursorIcon::AllScroll,
            CS::None => CursorIcon::Default, // winit doesn't have "none"
        }
    }

    /// The DOM-style key value a winit **logical** key spells — what feeds
    /// `PlatformEvent::KeyDown::logical_key` (and `KeyUp`'s), see the contract
    /// there. winit already speaks the browser's language, so this is nearly a
    /// transcription:
    ///
    /// - `Key::Character` is the layout-produced string, **verbatim**. Case is
    ///   preserved deliberately: `KeyboardEvent.key` in a browser is
    ///   case-*accurate* — measured, Chromium reports `"A"` for Shift+A and
    ///   `"S"` for Ctrl+Shift+S, on the press **and** the release. The old
    ///   lowercased-single-letter form made a release disagree with its own
    ///   press for every capital (a press spelled itself from `text`, which
    ///   kept the capital; a release has no text), which breaks the one thing
    ///   releases exist for — pairing (issue #337). Consumers that want a
    ///   case-insensitive identity fold at the comparison site, as
    ///   `editor_key_binding` does. The spacebar arrives here too, as `" "`,
    ///   the spec's own spelling.
    /// - `Key::Named` is `keyboard_types::NamedKey`, the W3C key-values enum;
    ///   its `Display` **is** the spec string (`"Enter"`, `"ArrowLeft"`,
    ///   `"Shift"`). winit's backends already fold the platform quirks the
    ///   browser folds — X11/Wayland's Super keysym is reported as
    ///   `NamedKey::Meta` "because browsers do" (winit-common's xkb keymap) —
    ///   so no respelling happens here.
    /// - A dead key is `"Dead"`, the value every browser reports for one.
    /// - `Key::Unidentified` is `None`, not the spec's `"Unidentified"`: it
    ///   carries only a native keysym rinch cannot spell, and `None` is what
    ///   lets the consumer fall back to the physical `key` — two different
    ///   unknown keys must not pair with each other by a shared placeholder
    ///   string.
    pub(super) fn winit_logical_key_str(key: &winit::keyboard::Key) -> Option<String> {
        use winit::keyboard::Key;
        match key {
            Key::Character(s) => Some(s.to_string()),
            Key::Named(n) => Some(n.to_string()),
            Key::Dead(_) => Some("Dead".to_string()),
            Key::Unidentified(_) => None,
        }
    }

    /// The [`PlatformEvent::KeyDown`] a winit press translates to.
    ///
    /// Split out of the `WindowEvent::KeyboardInput` arm so it can be tested at
    /// all: `window_event` needs a live `ActiveEventLoop` and a real window, and
    /// the only other statement in that arm is the menu-shortcut consume.
    ///
    /// What that buys is the **one** pin on this backend's `repeat` answer.
    /// Every consumer fixture supplies its own [`KeyRepeat`], so inverting the
    /// mapping here — desktop refusing every fresh press and activating on
    /// every auto-repeat, i.e. #463 made worse than before it — was invisible
    /// to the entire runtime suite (measured: 602 passed, 0 failed, in the
    /// adversarial review of PR #796). The assertion in
    /// `key_repeat_translation` is trivial by construction; the point is that
    /// there was nothing at all.
    pub(super) fn key_down_from_winit(
        key_code: winit::keyboard::KeyCode,
        logical_key: &winit::keyboard::Key,
        text: Option<&str>,
        repeat: bool,
        modifiers: Modifiers,
    ) -> PlatformEvent {
        PlatformEvent::KeyDown {
            key: Self::translate_key(key_code),
            // The layout-mapped key value (so Mod+letter follows the keycap, not
            // the physical position) — winit's logical key carries it even when a
            // modifier suppresses `text`.
            logical_key: Self::winit_logical_key_str(logical_key),
            text: text.map(str::to_string),
            modifiers,
            // winit's `repeat` is `true` if and only if the OS made this press
            // by holding the key down, so desktop never has to infer it from a
            // release that may never arrive (#463).
            repeat: if repeat {
                KeyRepeat::Repeat
            } else {
                KeyRepeat::Fresh
            },
        }
    }

    /// Translate a winit KeyCode to a platform KeyCode.
    pub(super) fn translate_key(key_code: winit::keyboard::KeyCode) -> KeyCode {
        use winit::keyboard::KeyCode as WK;
        match key_code {
            WK::ArrowLeft => KeyCode::ArrowLeft,
            WK::ArrowRight => KeyCode::ArrowRight,
            WK::ArrowUp => KeyCode::ArrowUp,
            WK::ArrowDown => KeyCode::ArrowDown,
            WK::Home => KeyCode::Home,
            WK::End => KeyCode::End,
            WK::PageUp => KeyCode::PageUp,
            WK::PageDown => KeyCode::PageDown,
            WK::Enter | WK::NumpadEnter => KeyCode::Enter,
            WK::Backspace => KeyCode::Backspace,
            WK::Delete => KeyCode::Delete,
            WK::Tab => KeyCode::Tab,
            WK::Escape => KeyCode::Escape,
            WK::Space => KeyCode::Space,
            WK::KeyA => KeyCode::KeyA,
            WK::KeyB => KeyCode::KeyB,
            WK::KeyC => KeyCode::KeyC,
            WK::KeyD => KeyCode::KeyD,
            WK::KeyE => KeyCode::KeyE,
            WK::KeyF => KeyCode::KeyF,
            WK::KeyG => KeyCode::KeyG,
            WK::KeyH => KeyCode::KeyH,
            WK::KeyI => KeyCode::KeyI,
            WK::KeyJ => KeyCode::KeyJ,
            WK::KeyK => KeyCode::KeyK,
            WK::KeyL => KeyCode::KeyL,
            WK::KeyM => KeyCode::KeyM,
            WK::KeyN => KeyCode::KeyN,
            WK::KeyO => KeyCode::KeyO,
            WK::KeyP => KeyCode::KeyP,
            WK::KeyQ => KeyCode::KeyQ,
            WK::KeyR => KeyCode::KeyR,
            WK::KeyS => KeyCode::KeyS,
            WK::KeyT => KeyCode::KeyT,
            WK::KeyU => KeyCode::KeyU,
            WK::KeyV => KeyCode::KeyV,
            WK::KeyW => KeyCode::KeyW,
            WK::KeyX => KeyCode::KeyX,
            WK::KeyY => KeyCode::KeyY,
            WK::KeyZ => KeyCode::KeyZ,
            WK::Digit0 => KeyCode::Digit0,
            WK::Digit1 => KeyCode::Digit1,
            WK::Digit2 => KeyCode::Digit2,
            WK::Digit3 => KeyCode::Digit3,
            WK::Digit4 => KeyCode::Digit4,
            WK::Digit5 => KeyCode::Digit5,
            WK::Digit6 => KeyCode::Digit6,
            WK::Digit7 => KeyCode::Digit7,
            WK::Digit8 => KeyCode::Digit8,
            WK::Digit9 => KeyCode::Digit9,
            WK::F1 => KeyCode::F1,
            WK::F2 => KeyCode::F2,
            WK::F3 => KeyCode::F3,
            WK::F4 => KeyCode::F4,
            WK::F5 => KeyCode::F5,
            WK::F6 => KeyCode::F6,
            WK::F7 => KeyCode::F7,
            WK::F8 => KeyCode::F8,
            WK::F9 => KeyCode::F9,
            WK::F10 => KeyCode::F10,
            WK::F11 => KeyCode::F11,
            WK::F12 => KeyCode::F12,
            WK::Equal => KeyCode::Equal,
            WK::Minus => KeyCode::Minus,
            WK::ShiftLeft => KeyCode::ShiftLeft,
            WK::ShiftRight => KeyCode::ShiftRight,
            WK::ControlLeft => KeyCode::ControlLeft,
            WK::ControlRight => KeyCode::ControlRight,
            WK::AltLeft => KeyCode::AltLeft,
            WK::AltRight => KeyCode::AltRight,
            _ => KeyCode::Other,
        }
    }

    /// Translate winit modifier state to platform Modifiers.
    pub(super) fn translate_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.main.modifiers.shift_key(),
            ctrl: self.main.modifiers.control_key(),
            alt: self.main.modifiers.alt_key(),
            meta: self.main.modifiers.meta_key(),
        }
    }
}

#[cfg(test)]
mod logical_key_spelling {
    use super::*;
    use winit::keyboard::{Key, NamedKey, SmolStr};

    fn spell(key: Key) -> Option<String> {
        RinchRuntime::winit_logical_key_str(&key)
    }

    /// A character key is passed through verbatim — case included, because a
    /// browser's `KeyboardEvent.key` is case-accurate and a lowercased source
    /// made `Shift+A` release as `"a"` after pressing as `"A"` (issue #337's
    /// review finding). Non-letters ride the same arm now: `Shift+1` on a US
    /// layout is `"!"` down *and* up, where the old single-ASCII-letter filter
    /// dropped it and the release fell back to the physical `"1"`.
    #[test]
    fn a_character_key_is_verbatim_case_included() {
        assert_eq!(
            spell(Key::Character(SmolStr::new("a"))).as_deref(),
            Some("a")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("A"))).as_deref(),
            Some("A"),
            "Shift+A is \"A\" in a browser — no lowercasing at the source"
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("!"))).as_deref(),
            Some("!")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("é"))).as_deref(),
            Some("é")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new(" "))).as_deref(),
            Some(" "),
            "the spacebar's key value is the space character, per spec"
        );
    }

    /// A named key spells its W3C key-values name — `NamedKey` *is*
    /// `keyboard_types::NamedKey` and its `Display` is the spec string, so
    /// this pins the transcription, not a hand-kept table.
    #[test]
    fn a_named_key_spells_its_w3c_name() {
        assert_eq!(spell(Key::Named(NamedKey::Enter)).as_deref(), Some("Enter"));
        assert_eq!(
            spell(Key::Named(NamedKey::ArrowLeft)).as_deref(),
            Some("ArrowLeft")
        );
        assert_eq!(spell(Key::Named(NamedKey::Shift)).as_deref(), Some("Shift"));
        assert_eq!(
            spell(Key::Named(NamedKey::Meta)).as_deref(),
            Some("Meta"),
            "the OS key: winit folds X11's Super into Meta the way browsers do"
        );
    }

    /// A dead key is `"Dead"` (every browser's value for one); an unidentified
    /// key is `None`, so the consumer falls back to the physical `key` instead
    /// of pairing two different unknown keys by a shared placeholder.
    #[test]
    fn dead_spells_dead_and_unidentified_spells_nothing() {
        assert_eq!(spell(Key::Dead(Some('^'))).as_deref(), Some("Dead"));
        assert_eq!(spell(Key::Dead(None)).as_deref(), Some("Dead"));
        assert_eq!(
            spell(Key::Unidentified(winit::keyboard::NativeKey::Unidentified)),
            None
        );
    }
}

/// The desktop backend's `KeyRepeat` answer (issue #463).
///
/// The one producer pin on this backend. Every `key_repeat_tests` fixture hands
/// the runtime a `KeyRepeat` of its own, so nothing there can tell whether the
/// shell reports winit's flag, its negation, or a constant — inverting it
/// survived all 602 tests of `cargo test -p rinch --lib --features debug` in the
/// adversarial review of PR #796. That is the hole these two assertions close,
/// and it is worth knowing they are the *only* thing closing it.
#[cfg(test)]
mod key_repeat_translation {
    use super::*;
    use winit::keyboard::{Key, KeyCode as WK, SmolStr};

    fn press(repeat: bool) -> PlatformEvent {
        // The spacebar's logical key is its character, not a named key — the
        // W3C spells it `" "`, and so does winit.
        RinchRuntime::key_down_from_winit(
            WK::Space,
            &Key::Character(SmolStr::new(" ")),
            Some(" "),
            repeat,
            Modifiers::default(),
        )
    }

    fn repeat_of(event: &PlatformEvent) -> KeyRepeat {
        match event {
            PlatformEvent::KeyDown { repeat, .. } => *repeat,
            other => panic!("expected a KeyDown, got {other:?}"),
        }
    }

    /// winit's `repeat` is documented `true` **iff** the OS produced this press
    /// by holding the key, so the mapping is the identity and not a negation.
    /// Getting it backwards would make desktop refuse every real press and
    /// activate on every auto-repeat — #463 inverted, which is worse than #463.
    #[test]
    fn winits_flag_is_reported_not_inverted() {
        assert_eq!(repeat_of(&press(false)), KeyRepeat::Fresh);
        assert_eq!(repeat_of(&press(true)), KeyRepeat::Repeat);
    }

    /// And never `Unknown`: this backend always knows, so it must never fall
    /// back to the latch — which is the whole of #463 on desktop. Kills a
    /// shell that quietly answers `KeyRepeat::Unknown` (the `Default`, and so
    /// the shape a careless refactor reaches for).
    #[test]
    fn the_desktop_shell_never_answers_unknown() {
        for r in [false, true] {
            assert_ne!(repeat_of(&press(r)), KeyRepeat::Unknown);
        }
    }

    /// The extraction moved the whole press translation, not just the flag, so
    /// the rest of it is pinned here too — the physical key, the layout-mapped
    /// `logical_key` and the inserted `text` all still ride along.
    #[test]
    fn the_rest_of_the_press_survives_the_split() {
        let PlatformEvent::KeyDown {
            key,
            logical_key,
            text,
            ..
        } = press(false)
        else {
            panic!("expected a KeyDown");
        };
        assert_eq!(key, KeyCode::Space);
        assert_eq!(logical_key.as_deref(), Some(" "));
        assert_eq!(text.as_deref(), Some(" "));
    }
}

/// winit's pointer sources as a render surface hears them
/// (`SurfacePointer`): which device, which id, how hard.
#[cfg(test)]
mod pointer_source_tests {
    use super::*;
    use winit::event::{
        ButtonSource, FingerId, Force, PointerSource, TabletToolButton, TabletToolData,
        TabletToolKind,
    };

    fn tool(kind: TabletToolKind, force: Option<f64>) -> PointerSource {
        PointerSource::TabletTool {
            kind,
            data: TabletToolData {
                force: force.map(Force::Normalized),
                ..Default::default()
            },
        }
    }

    /// A chord's release leaves the held button down; a window blur forgets
    /// every press, since the release of a button held across it may go to
    /// another window (review of #1502, N3).
    #[test]
    fn pressed_buttons_track_chords_and_a_blur_forgets_them() {
        let mut p = PressedButtons::default();
        p.note(1, Some(MouseButton::Left), true);
        p.note(1, Some(MouseButton::Right), true);
        p.note(1, Some(MouseButton::Right), false);
        assert!(p.is_down(1), "the left button is still held");
        p.note(1, Some(MouseButton::Left), false);
        assert!(!p.is_down(1));
        p.note(1, Some(MouseButton::Left), true);
        p.note(TOUCH_POINTER_BASE, None, true);
        p.clear();
        assert!(!p.is_down(1) && !p.is_down(TOUCH_POINTER_BASE));
        p.note(TOUCH_POINTER_BASE, None, true);
        assert!(p.lift(TOUCH_POINTER_BASE) && !p.lift(TOUCH_POINTER_BASE));
    }

    #[test]
    fn a_pen_reports_its_force_and_an_eraser_says_so() {
        let p = surface_pointer_of(&tool(TabletToolKind::Pen, Some(0.3)), true, true);
        assert_eq!(p.kind, SurfacePointerKind::Pen);
        assert!((p.pressure - 0.3).abs() < 1e-6);
        assert_eq!(p.id, TABLET_POINTER_ID);
        let e = surface_pointer_of(&tool(TabletToolKind::Eraser, Some(1.5)), true, true);
        assert_eq!(e.kind, SurfacePointerKind::Eraser);
        assert_eq!(e.pressure, 1.0, "clamped");
        // A tool without force sensing reports the Pointer Events default.
        assert_eq!(
            surface_pointer_of(&tool(TabletToolKind::Pen, None), true, true).pressure,
            0.5
        );
        assert_eq!(
            surface_pointer_of(&tool(TabletToolKind::Pen, None), true, false).pressure,
            0.0
        );
    }

    #[test]
    fn each_finger_is_its_own_pointer_and_never_the_mouse() {
        let finger = |raw: usize| PointerSource::Touch {
            finger_id: FingerId::from_raw(raw),
            force: None,
        };
        let a = surface_pointer_of(&finger(0), true, true);
        let b = surface_pointer_of(&finger(1), false, true);
        assert_eq!(a.kind, SurfacePointerKind::Touch);
        assert_ne!(a.id, b.id);
        assert!(a.id != SurfacePointer::mouse(true).id && a.id != TABLET_POINTER_ID);
        // No finger's id is the mouse's or the tablet's, the first two
        // included (a touch base of 0 gives finger 1 the mouse's id).
        for raw in 0..4 {
            let id = surface_pointer_of(&finger(raw), false, true).id;
            assert!(id != SurfacePointer::mouse(true).id && id != TABLET_POINTER_ID);
        }
        assert_eq!(a.id, pointer_id_of(&finger(0)));
        assert!(a.primary && !b.primary);
        assert_eq!(a.pressure, 0.5, "no force measured, contact down");
    }

    #[test]
    fn the_mouse_is_pointer_one_and_a_release_has_no_pressure() {
        let m = surface_pointer_of(&PointerSource::Mouse, true, true);
        assert_eq!(m, SurfacePointer::mouse(true));
        let pressed = surface_pointer_of_button(
            &ButtonSource::TabletTool {
                kind: TabletToolKind::Pen,
                button: TabletToolButton::Contact,
                data: TabletToolData {
                    force: Some(Force::Normalized(0.8)),
                    ..Default::default()
                },
            },
            true,
            true,
        );
        assert!((pressed.pressure - 0.8).abs() < 1e-6);
        let released =
            surface_pointer_of_button(&ButtonSource::Mouse(MouseButton::Left), true, false);
        assert_eq!(released.pressure, 0.0);
        assert_eq!(released.id, 1);
    }
}
