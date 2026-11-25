//! Applying the IME composition translator's actions to the app.

use rinch_platform::{ImeEvent, KeyCode, KeyRepeat, Modifiers, PlatformEvent};

use super::process_actions;
use crate::app::RinchApp;
use crate::shell::android_ime::ImeAction;

/// Apply one [`ImeAction`] from the composition translator.
///
/// A commit is still applied as per-character `KeyDown`s rather than as
/// `ImeEvent::Commit`. That path is the one that has been watched working on a
/// device, and composition does not need it changed: what a preedit requires is
/// that the composition be *cleared before* the text that replaces it arrives,
/// which the action list already orders. Switching commits to one
/// `ImeEvent::Commit` (one edit, better undo grouping) remains worth doing and
/// remains a separate change — it alters what every keystroke on Android does,
/// including the ones that never compose.
///
/// `window_size` is the **physical** surface size, the unit `handle_event`
/// takes — it derives the logical layout viewport from it itself
/// (`RinchApp::layout_viewport`). Handing it the logical size here would divide
/// by the scale factor twice and lay the page out that much too narrow on every
/// IME keystroke.
pub(super) fn apply_ime_action(
    app: &mut RinchApp,
    action: ImeAction,
    window_size: (u32, u32),
    scale_factor: f64,
    running: &mut bool,
) {
    // What the focused field actually holds, as a bound on the delete loops
    // below. Deleting past either end of the buffer is a no-op per character,
    // and `deleteSurroundingText(Integer.MAX_VALUE, 0)` — a documented way for
    // an IME to clear a field — would otherwise spin two billion no-op edit
    // commands and wedge the frame.
    let field_len = if matches!(action, ImeAction::Delete { .. }) {
        app.focused_input_value.chars().count()
    } else {
        0
    };
    // A single-line field folds a committed text's line breaks as a whole
    // (issue #824) before it is split into one key per character below: one
    // character at a time, an inner `\n` would look trailing and vanish.
    let action = match action {
        ImeAction::Insert(text) => ImeAction::Insert(app.insertable_text(text)),
        other => other,
    };
    let mut dispatch = |event: PlatformEvent, running: &mut bool| {
        let actions = app.handle_event(event, window_size, scale_factor);
        process_actions(&actions, running);
    };
    match action {
        ImeAction::Preedit { text, cursor } => {
            dispatch(
                PlatformEvent::Ime(ImeEvent::Preedit { text, cursor }),
                running,
            );
        }
        ImeAction::Insert(text) => {
            for ch in text.chars() {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Other,
                        // A soft-keyboard commit is synthesized text — no key
                        // produced it, so there is no key value to report
                        // (`None` = unknown, per the field's contract).
                        logical_key: None,
                        text: Some(ch.to_string()),
                        modifiers: Modifiers::default(),
                        // Synthesized, one per committed character — never an
                        // OS auto-repeat of a held key (issue #463).
                        repeat: KeyRepeat::Fresh,
                    },
                    running,
                );
            }
        }
        ImeAction::Delete { before, after } => {
            for _ in 0..before.min(field_len) {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Backspace,
                        logical_key: None,
                        text: None,
                        modifiers: Modifiers::default(),
                        repeat: KeyRepeat::Fresh,
                    },
                    running,
                );
            }
            for _ in 0..after.min(field_len) {
                dispatch(
                    PlatformEvent::KeyDown {
                        key: KeyCode::Delete,
                        logical_key: None,
                        text: None,
                        modifiers: Modifiers::default(),
                        repeat: KeyRepeat::Fresh,
                    },
                    running,
                );
            }
        }
    }
}
