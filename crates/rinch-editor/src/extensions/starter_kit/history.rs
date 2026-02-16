//! History extension for undo/redo functionality.

use crate::extensions::{CommandRegistration, Extension};
use crate::input::KeyboardShortcut;

/// History extension for undo/redo functionality.
#[derive(Debug)]
pub struct HistoryExt;

impl Extension for HistoryExt {
    fn name(&self) -> &str {
        "history"
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![
            CommandRegistration::new("undo", |editor| editor.undo().map(|_| ())),
            CommandRegistration::new("redo", |editor| editor.redo().map(|_| ())),
        ]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![
            (KeyboardShortcut::new("Mod-Z", "Undo"), "undo".into()),
            (KeyboardShortcut::new("Mod-Shift-Z", "Redo"), "redo".into()),
        ]
    }
}
