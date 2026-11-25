//! The soft keyboard and the IME: keeping the keyboard in step with the
//! focused field, and applying the composition translator's actions to the app.

use rinch_platform::{ImeEvent, KeyCode, KeyRepeat, Modifiers, PlatformEvent};

use super::loop_state::LoopState;
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
fn apply_ime_action(
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

impl LoopState {
    /// Tell the soft keyboard what the focused field is: what Enter means in
    /// it, and whether there is one at all.
    pub(super) fn sync_soft_keyboard(&mut self) {
        // What Enter means in the focused field, pushed before the keyboard is
        // raised so the session it opens already draws the right key. A
        // `<textarea>` wants a newline key; everything else wants what it had.
        // Android only reads this when a session starts, and rinch's own
        // field-to-field moves are invisible to it, so the change has to be
        // announced here or not at all.
        let needs_multiline = self.app.focused_input_is_multiline();
        if needs_multiline != self.keyboard_multiline
            && rinch_android::ime::set_multiline(needs_multiline)
        {
            // Advance the mirror only for a push that landed: a dropped one
            // would leave it claiming a kind the keyboard was never told
            // about, and nothing else ever pushes this. A failure retries on
            // the next turn of the loop.
            self.keyboard_multiline = needs_multiline;
        }

        // Show/hide soft keyboard based on input focus
        let needs_keyboard = self.app.has_focused_input() || self.app.has_focused_contenteditable();
        if needs_keyboard != self.keyboard_visible {
            self.keyboard_visible = needs_keyboard;
            if needs_keyboard {
                rinch_android::ime::show_keyboard();
            } else {
                rinch_android::ime::hide_keyboard();
            }
        }
    }

    /// Apply what the IME did since the last frame, in the order it did it.
    pub(super) fn apply_ime_updates(&mut self) {
        // Taken *before* the focus check below, which needs to be able to
        // throw the batch away: every call in it was made against the field
        // that was focused when the keyboard made it.
        let mut updates = rinch_android::ime::drain_updates();

        // The focused field changed while the IME was composing into the old
        // one. The focus arbiter has already committed that composition into
        // the field that lost focus (`set_focus_target_deferred`'s
        // compositionend-before-blur), so nothing is inserted here — but the
        // keyboard still believes it is composing, and would deliver the same
        // word again into the field that just gained focus. Restarting its
        // input session is the only way to say so; Android cannot see the move
        // itself, because one `RinchInputView` holds focus throughout.
        let focused_field = self.app.focused_input_node();
        if focused_field != self.composing_field {
            self.composing_field = focused_field;
            if self.composition.abandon() {
                // Emptying the mirror is not enough on its own: the touch that
                // moved focus was dispatched earlier in *this* iteration, so
                // anything the keyboard queued before it is still in hand and
                // would now be applied to the field that just gained focus — a
                // `setComposingText` re-opening the region there, and the
                // `finishComposingText` that `restart_input` provokes then
                // inserting the same word a second time, into the wrong field.
                // The whole batch belongs to the session that just ended.
                updates.clear();
                rinch_android::ime::restart_input();
            }
        }

        // `android_ime` turns the `InputConnection` call stream into the
        // actions below; it holds the composing region and every rule about
        // when one ends.
        for update in updates {
            let actions = match update {
                rinch_android::ime::ImeUpdate::SetComposingText {
                    text,
                    new_cursor_position,
                } => self
                    .composition
                    .set_composing_text(text, new_cursor_position),
                rinch_android::ime::ImeUpdate::FinishComposingText => {
                    self.composition.finish_composing_text()
                }
                rinch_android::ime::ImeUpdate::CommitText(text) => {
                    self.composition.commit_text(text)
                }
                rinch_android::ime::ImeUpdate::DeleteSurroundingText { before, after } => {
                    self.composition.delete_surrounding_text(before, after)
                }
            };
            // An edit from the soft keyboard takes the toolbar down too:
            // typing collapses the selection it was showing for. Judged by
            // what the IME *did*, not by whether it called: a focus move
            // restarts the input session, the keyboard answers with a
            // `finishComposingText` that composes nothing, and a toolbar
            // that just went up for that very press must survive it.
            let edited = actions.iter().any(|a| match a {
                ImeAction::Insert(text) => !text.is_empty(),
                ImeAction::Preedit { text, .. } => !text.is_empty(),
                ImeAction::Delete { .. } => true,
            });
            if edited && self.text_toolbar.finish() {
                rinch_android::text_action::finish_toolbar();
            }
            for action in actions {
                self.apply_ime_action(action);
            }
        }
    }

    /// Commit the composition if the activity went to the background since
    /// the last turn, and clear that it did. See the two calls in
    /// `LoopState::turn` for why there are two.
    pub(super) fn flush_composition_if_backgrounded(&mut self) {
        if std::mem::take(&mut self.backgrounded) {
            for action in self.composition.finish_composing_text() {
                self.apply_ime_action(action);
            }
        }
    }

    fn apply_ime_action(&mut self, action: ImeAction) {
        apply_ime_action(
            &mut self.app,
            action,
            self.physical_size,
            self.scale_factor,
            &mut self.running,
        );
    }
}
