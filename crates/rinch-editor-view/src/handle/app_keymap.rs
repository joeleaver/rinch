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

impl EditorHandle {
    /// Bind `chord` (`"Mod-Shift-s"`, `"Mod-Alt-1"`; the syntax of
    /// [`KeyBinding::parse`]) to the command named `command` in this editor —
    /// one of [`EditorState::command`]'s names, such as `"toggleStrike"`, or
    /// a plugin's. The app's binding wins over every plugin's for that chord,
    /// and consumes its key whether or not the command applies, as a plugin's
    /// plain binding does. It lives on the handle: it survives a re-mount, a
    /// load and every plugin change. Answers `false` (and binds nothing) when
    /// `chord` does not parse.
    ///
    /// The default keymap has only the keys a Chrome `contenteditable` has
    /// (#3062), so an app that wants the formatting chords of a word
    /// processor binds them here:
    ///
    /// ```ignore
    /// handle.bind_key("Mod-Shift-s", "toggleStrike");
    /// handle.bind_key("Mod-Shift-8", "toggleBulletList");
    /// handle.bind_key("Mod-Alt-1", "setHeading1");
    /// ```
    pub fn bind_key(&self, chord: &str, command: &str) -> bool {
        let Some(binding) = KeyBinding::parse(chord) else {
            return false;
        };
        let mut core = self.core_mut();
        core.app_keys.unbound.remove(&binding);
        core.app_keys.bound.add(binding, command);
        true
    }

    /// Take `chord` away from this editor, so its key goes on as a key the
    /// editor has no binding for: on desktop to the app's `on_app_key` and
    /// menus, on the web to the page's `keydown` listeners and the browser's
    /// default. It removes the app's own [`bind_key`](Self::bind_key) binding of
    /// the chord and masks every plugin's, the base keymap's included (say,
    /// Mod-Enter, Mod-u or Mod-a), without dropping that plugin or its
    /// commands, which still run by name. A later `bind_key` of the chord binds
    /// it again. Lives on the handle, like `bind_key`. Answers `false` (and
    /// changes nothing) when `chord` does not parse.
    ///
    /// Tab and Shift+Tab are bindings too: unbinding `"Tab"` stops it indenting
    /// a list under [`TabBehavior::IndentInListsAndTables`], so it moves the
    /// focus there instead (it still moves between table cells, which no
    /// binding does).
    pub fn unbind_key(&self, chord: &str) -> bool {
        let Some(binding) = KeyBinding::parse(chord) else {
            return false;
        };
        let mut core = self.core_mut();
        core.app_keys.bound.remove(&binding);
        core.app_keys.unbound.insert(binding);
        true
    }
}
