//! The app's own key bindings over the plugins' keymap (#3062):
//! [`EditorHandle::bind_key`] and [`EditorHandle::unbind_key`].
//!
//! The plugins' keymap is aggregated into the editor state when it is built
//! ([`EditorState::keymap`]), and rebuilt by every plugin change
//! (`add_plugin`, `remove_plugin`, the placeholder, markdown shortcuts). The
//! app's layer lives on the handle instead, beside it, so a binding or an
//! unbinding survives those rebuilds and needs no plugin of its own, and
//! [`EditorHandle::dispatch_key`] asks it first.

use std::collections::HashSet;

use rinch_editor_core::Keymap;

use super::*;

/// The app's layer: chords it bound, and chords it took away from the plugins.
/// A chord is in at most one of the two.
#[derive(Debug, Default)]
pub(super) struct AppKeymap {
    /// [`EditorHandle::bind_key`]'s bindings; they consume their key, as a
    /// plugin's plain binding does.
    bound: Keymap,
    /// [`EditorHandle::unbind_key`]'s chords: looked up as unbound whatever a
    /// plugin binds them to.
    unbound: HashSet<KeyBinding>,
}

/// What the app's layer says about a chord.
pub(super) enum AppKey {
    /// The app bound it to this command.
    Bound(String),
    /// The app unbound it: the editor has no binding for it.
    Unbound,
    /// The app said nothing about it: the plugins' keymap decides.
    Plugins,
}

impl AppKeymap {
    pub(super) fn lookup(&self, binding: &KeyBinding) -> AppKey {
        if let Some(command) = self.bound.command_for(binding) {
            AppKey::Bound(command.to_string())
        } else if self.unbound.contains(binding) {
            AppKey::Unbound
        } else {
            AppKey::Plugins
        }
    }
}

/// Why [`EditorHandle::bind_key`] or [`EditorHandle::unbind_key`] refused a
/// call. A refused call changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyBindError {
    /// The chord does not parse ([`KeyBinding::parse`]'s syntax:
    /// `"Mod-Shift-s"`, `"Mod-Alt-1"`, `"Tab"`).
    Chord(String),
    /// The editor has no command of that name: not one of
    /// [`EditorState::command`]'s names, nor a command of a plugin the editor
    /// has. A plugin's command can be bound once the plugin is added.
    UnknownCommand(String),
}

impl std::fmt::Display for KeyBindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chord(chord) => write!(f, "`{chord}` is not a key chord"),
            Self::UnknownCommand(name) => write!(f, "the editor has no command named `{name}`"),
        }
    }
}

impl std::error::Error for KeyBindError {}

fn parse_chord(chord: &str) -> Result<KeyBinding, KeyBindError> {
    KeyBinding::parse(chord).ok_or_else(|| KeyBindError::Chord(chord.to_string()))
}

impl EditorHandle {
    /// Bind `chord` (`"Mod-Shift-s"`, `"Mod-Alt-1"`; the syntax of
    /// [`KeyBinding::parse`]) to the command named `command` in this editor —
    /// one of [`EditorState::command`]'s names, such as `"toggleStrike"`, or
    /// a plugin's. The app's binding wins over every plugin's for that chord,
    /// and consumes its key whether or not the command applies, as a plugin's
    /// plain binding does. It lives on the handle: it survives a re-mount, a
    /// load and every plugin change.
    ///
    /// A chord that does not parse is refused with [`KeyBindError::Chord`],
    /// and a command the editor does not have with
    /// [`KeyBindError::UnknownCommand`] (a misspelt name would otherwise bind
    /// a key that silently does nothing); either way nothing is bound. The
    /// command is looked up when it is bound, so bind a plugin's command after
    /// adding the plugin. A binding whose command later leaves with its plugin
    /// ([`remove_plugin`](Self::remove_plugin)) still owns its key and does
    /// nothing, as a plugin's binding of a missing command does.
    ///
    /// `"Tab"` and `"Shift-Tab"` can be bound too, and the app's binding is
    /// asked before the editor's [`TabBehavior`]: bound, the key runs the
    /// command and is the editor's under [`TabBehavior::MoveFocus`] as well.
    /// That includes a table cell under
    /// [`TabBehavior::IndentInListsAndTables`]: a bound Tab runs its command
    /// there instead of moving to the next cell. To keep cell navigation and
    /// give Tab another job elsewhere, leave Tab unbound and claim it in
    /// [`on_key`](Self::on_key) only where you want it, trying
    /// [`tab_cell`](Self::tab_cell) first:
    ///
    /// ```ignore
    /// let h = handle.clone();
    /// handle.on_key(move |key| {
    ///     if key.key != "Tab" || key.primary || key.alt {
    ///         return false;
    ///     }
    ///     // In a table, the next (previous) cell; elsewhere a command of the
    ///     // app's own plugin. `false` leaves the key to the `TabBehavior`.
    ///     h.tab_cell(key.shift) || (!key.shift && h.command("insertTabCharacter"))
    /// });
    /// ```
    ///
    /// The default keymap has only the keys a Chrome `contenteditable` has
    /// (#3062), so an app that wants the formatting chords of a word
    /// processor binds them here:
    ///
    /// ```ignore
    /// handle.bind_key("Mod-Shift-s", "toggleStrike")?;
    /// handle.bind_key("Mod-Shift-8", "toggleBulletList")?;
    /// handle.bind_key("Mod-Alt-1", "setHeading1")?;
    /// ```
    pub fn bind_key(&self, chord: &str, command: &str) -> Result<(), KeyBindError> {
        let binding = parse_chord(chord)?;
        let mut core = self.core_mut();
        if core.state.command(command).is_none() {
            return Err(KeyBindError::UnknownCommand(command.to_string()));
        }
        core.app_keys.unbound.remove(&binding);
        core.app_keys.bound.add(binding, command);
        Ok(())
    }

    /// The command the app bound `binding` to with [`bind_key`](Self::bind_key),
    /// if any (Tab's lookup, which comes before the [`TabBehavior`]).
    pub(super) fn app_bound_command(&self, binding: &KeyBinding) -> Option<String> {
        match self.core().app_keys.lookup(binding) {
            AppKey::Bound(name) => Some(name),
            AppKey::Unbound | AppKey::Plugins => None,
        }
    }

    /// Take `chord` away from this editor, so its key goes on as a key the
    /// editor has no binding for: on desktop to the app's `on_app_key` and
    /// menus, on the web to the page's `keydown` listeners and the browser's
    /// default. It removes the app's own [`bind_key`](Self::bind_key) binding of
    /// the chord and masks every plugin's, the base keymap's included (say,
    /// Mod-Enter, Mod-u or Mod-a), without dropping that plugin or its
    /// commands, which still run by name. A later `bind_key` of the chord binds
    /// it again. Lives on the handle, like `bind_key`. A chord that does not
    /// parse is refused with [`KeyBindError::Chord`] and changes nothing.
    ///
    /// Tab and Shift+Tab are bindings too: unbinding `"Tab"` stops it indenting
    /// a list under [`TabBehavior::IndentInListsAndTables`], so it moves the
    /// focus there instead (it still moves between table cells, which no
    /// binding does).
    pub fn unbind_key(&self, chord: &str) -> Result<(), KeyBindError> {
        let binding = parse_chord(chord)?;
        let mut core = self.core_mut();
        core.app_keys.bound.remove(&binding);
        core.app_keys.unbound.insert(binding);
        Ok(())
    }
}
