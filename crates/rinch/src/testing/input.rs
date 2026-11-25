//! Input a test sends the main window: the pointer and the keyboard.

use super::*;

impl TestApp {
    // ── Pointer ─────────────────────────────────────────────────────────

    /// Move the mouse to `(x, y)`.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.send(PlatformEvent::MouseMove { x, y });
    }

    /// Press `button` at `(x, y)`. The pointer moves there first if it is not
    /// there already: a real button is pressed where the pointer is, and the
    /// shell presses at the position its last move left.
    pub fn mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        // The menus a press here closes, closed after it is delivered (#2552).
        let closes = crate::popup::light_dismiss(self.window_id(), Some((x, y)));
        self.send(PlatformEvent::MouseDown {
            x,
            y,
            button: platform_button(button),
        });
        self.close_popups(&closes);
    }

    /// Release `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_up(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseUp {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// A left click at `(x, y)`: press and release.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Left);
        self.mouse_up(x, y, MouseButton::Left);
    }

    /// A right click at `(x, y)` (the context-menu press).
    pub fn right_click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Right);
        self.mouse_up(x, y, MouseButton::Right);
    }

    /// Move the mouse to the centre of `target`'s box.
    pub fn hover(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.mouse_move(x, y);
    }

    /// Press `button` at the centre of `target`'s box.
    ///
    /// The press goes through hit testing like any other: it lands on
    /// whatever is painted at that point, which is `target` unless something
    /// covers it (or `target` has no box of its own).
    pub fn press(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_down(x, y, button);
    }

    /// Release `button` at the centre of `target`'s box.
    pub fn release(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_up(x, y, button);
    }

    /// A left click at the centre of `target`'s box. See [`Self::press`].
    pub fn click_on(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.click(x, y);
    }

    /// A wheel or trackpad scroll at `(x, y)` of `delta_x`/`delta_y` pixels,
    /// with the DOM's sign: positive `delta_y` scrolls the content down (the
    /// `WheelEvent.deltaY` an `onwheel` handler receives), as the MCP
    /// `scroll` tool takes it. Scrolls directly, like a trackpad.
    pub fn wheel(&mut self, x: f32, y: f32, delta_x: f64, delta_y: f64) {
        self.move_unless_there(x, y);
        // A wheel delta is negative toward "down" (winit's sign, which
        // `handle_event`'s `MouseWheel` arm subtracts from the offset).
        self.send(PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: -delta_x,
            delta_y: -delta_y,
        });
    }

    /// A mouse wheel's notches at `(x, y)`, DOM sign as in [`Self::wheel`]:
    /// the scroll glides, as one from a real wheel does (unless the app
    /// turned smooth wheel scrolling off). The glide runs on the virtual
    /// clock, so [`Self::advance_time`] carries it to its end (see
    /// [Time](super#time)).
    pub fn wheel_lines(&mut self, x: f32, y: f32, lines_x: f64, lines_y: f64) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseWheelLines {
            x,
            y,
            lines_x: -lines_x,
            lines_y: -lines_y,
        });
    }

    /// [`Self::wheel`] at the centre of `target`'s box.
    pub fn wheel_on(&mut self, target: impl Target, delta_x: f64, delta_y: f64) {
        let (x, y) = self.center(target);
        self.wheel(x, y, delta_x, delta_y);
    }

    fn move_unless_there(&mut self, x: f32, y: f32) {
        if self.app.cursor_pos != Some((x, y)) {
            self.mouse_move(x, y);
        }
    }

    // ── Keyboard ────────────────────────────────────────────────────────

    /// Hold `modifiers` (and release any not in it), as the shell's
    /// `ModifiersChanged` reports a modifier key. Every later key, click and
    /// wheel sees them held until the next call.
    pub fn set_modifiers(&mut self, modifiers: ModifierState) {
        let ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        } = modifiers;
        self.modifiers = Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        };
        self.send(PlatformEvent::ModifiersChanged(self.modifiers));
    }

    /// The modifiers held.
    pub fn modifiers(&self) -> ModifierState {
        let Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        } = self.modifiers;
        ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        }
    }

    /// Press the key `key` names, with the [held modifiers](Self::set_modifiers).
    ///
    /// Names are the W3C `KeyboardEvent.key` spellings: a single character
    /// (`"a"`, `"A"`, `"/"`) is that character, and `"Enter"`, `"Tab"`,
    /// `"Escape"`, `"Backspace"`, `"Delete"`, `"Space"`, `"ArrowLeft"`,
    /// `"ArrowRight"`, `"ArrowUp"`, `"ArrowDown"`, `"Home"`, `"End"`,
    /// `"PageUp"`, `"PageDown"` and `"F1"`–`"F12"` are those keys — the names
    /// the MCP `key_press` tool takes. With Shift held, a letter types its
    /// capital (`"c"` types `"C"`), as a QWERTY layout does; other characters
    /// type themselves (name `"!"`, not Shift+`"1"`). A menu shortcut matching the chord takes it
    /// first, and the app then sees no key, as with a real keyboard.
    ///
    /// Panics on a name it does not know.
    pub fn key_down(&mut self, key: &str) {
        // While a menu popup is open, its topmost menu takes the keys (#2552).
        if let Some(menu) = self.popup_key_target(self.window_id()) {
            self.window_by_id(menu).key_down(key);
            return;
        }
        let mut actions = Vec::new();
        let size = self.physical_size();
        let outcome =
            self.app
                .dispatch_named_key_down(key, self.modifiers, &mut actions, size, self.scale);
        assert!(
            outcome != NamedKeyDown::UnknownKey,
            "unknown key name {key:?}"
        );
        let asked = self.note(actions);
        self.turn(asked);
        self.serve_window_requests();
    }

    /// Release the key `key` names (see [`Self::key_down`]).
    pub fn key_up(&mut self, key: &str) {
        let code = keyname_to_keycode(key).unwrap_or_else(|| panic!("unknown key name {key:?}"));
        self.send(PlatformEvent::KeyUp {
            key: code,
            logical_key: named_logical_key(key, self.modifiers.shift),
            modifiers: self.modifiers,
        });
    }

    /// A keystroke: [`Self::key_down`] then [`Self::key_up`].
    pub fn press_key(&mut self, key: &str) {
        self.key_down(key);
        self.key_up(key);
    }

    /// Type `text`, one keystroke per character: `'\n'` is Enter, `'\t'` Tab,
    /// any other character its own key. With the held modifiers, like
    /// [`Self::press_key`].
    pub fn type_text(&mut self, text: &str) {
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            let key = match ch {
                '\n' => "Enter",
                '\t' => "Tab",
                c => c.encode_utf8(&mut buf),
            };
            let key = key.to_owned();
            self.press_key(&key);
        }
    }

    /// An input method committing `text` to the focused field, as an IME
    /// (or a dictation or emoji picker) does.
    pub fn ime_commit(&mut self, text: &str) {
        self.send(PlatformEvent::Ime(ImeEvent::Commit(text.to_owned())));
    }

    /// Focus `target` as app code does with [`NodeHandle::focus`], then run
    /// a turn.
    pub fn focus(&mut self, target: impl Target) {
        self.handle(target).focus();
        self.turn(true);
        self.serve_window_requests();
    }
}
