//! IME composition and `beforeinput`: what software keyboards, dictation and
//! the edit menu ask the editor to do.

use super::capture::capture_target;
use super::context_menu::end_context_menu_cycle;
use super::mirror::{clear_mirror, reconcile_mirror};
use super::motion::visual_line_bound;
use super::*;

// ── IME composition ────────────────────────────────────────────────────────────
//
// The preedit is a view-local overlay (design A5), never part of the document:
// `compositionupdate` shows it at the caret, `compositionend` commits the final
// text as one ordinary edit (so undo/history treat it like typing).

pub(super) fn on_composition_update(event: &web_sys::CompositionEvent) {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    handle.ime_set_preedit(&event.data().unwrap_or_default(), None);
    refresh_caret();
}

pub(super) fn on_composition_end(event: &web_sys::CompositionEvent) {
    let Some((_, handle)) = focused_handle() else {
        COMPOSING.with(|c| c.set(false));
        return;
    };
    // Drop the overlay first: from here the composed text belongs to the document.
    handle.ime_clear_preedit();
    // The textarea now holds the block with the composition applied — including any
    // text the IME replaced, which is the whole reason the mirror exists. Diff it
    // rather than inserting `event.data()` at the caret: a commit that rewrote the
    // word behind the caret would otherwise be appended to it ("word" + "world"
    // committing as "wordworld").
    let applied = reconcile_mirror(&handle);
    if !applied {
        // No mirror to diff against (composition began before the block was mirrored).
        // Fall back to the plain commit — an append, but better than dropping the text.
        handle.ime_commit(&event.data().unwrap_or_default());
    }
    COMPOSING.with(|c| c.set(false));
    refresh_caret();
}

// ── `beforeinput` (software keyboards, dictation, the edit menu) ──────────────
//
// A physical key never gets here: `handle_keydown` consumes it and `preventDefault`s,
// and a cancelled `keydown` fires no `beforeinput`. What is left is everything a
// `keydown` cannot express — Android's `Unidentified` / 229 keys, swipe typing,
// dictation, the platform edit menu, and the undo/redo gestures — all of which the
// browser describes only by `inputType`.

/// What a `beforeinput` `inputType` asks the model to do.
///
/// Split out from [`on_before_input`] so the mapping is testable without a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EditIntent {
    /// Insert the event's `data` (replacing any selection).
    InsertText,
    /// Run a named editor command.
    Command(&'static str),
    /// Delete to the edge of the **visual** line the caret is on (`forward`: its
    /// end) — `deleteSoftLine*`, what a soft keyboard or Cmd+Backspace asks for
    /// (#301). Resolved from the browser's layout by [`visual_line_bound`]; the
    /// textblock edge without geometry.
    DeleteToVisualLine { forward: bool },
    /// Extend the selection over `motion`, then delete it — `deleteHardLine*`,
    /// which means the textblock's edge. The core has no command for either line
    /// delete; word deletes are core commands (`deleteWordBackward` /
    /// `deleteWordForward`, #303) and name them.
    DeleteTo(CursorMotion),
    /// Let the browser make the edit in the mirrored textarea and recover it by diff
    /// on the following `input` — the only honest way to apply an edit whose extent
    /// lives in the keyboard's replacement range rather than in the event itself.
    Reconcile,
    /// Not ours: already applied by a dedicated handler, owned by the composition
    /// events, or not expressible on the model.
    Ignore,
}

/// Map a `beforeinput` `inputType` onto an [`EditIntent`].
///
/// Names come from the Input Events spec; the set here is what browsers actually emit
/// for an editable host. Anything unlisted is [`EditIntent::Ignore`] — silently doing
/// nothing beats guessing at an edit the user did not ask for.
pub(super) fn edit_intent(input_type: &str) -> EditIntent {
    match input_type {
        // Plain typing, swipe typing, dictation, an emoji picker.
        "insertText" => EditIntent::InsertText,
        // The composition events own the preedit overlay and the commit; applying this
        // as well would type every composed word twice.
        "insertCompositionText" => EditIntent::Ignore,
        // Autocorrect / a suggestion-bar tap: the event says what to insert but not what
        // it replaces (`getTargetRanges()` is empty on a textarea, by spec). Let the
        // browser apply it to the mirror and read the replacement back off the diff.
        "insertReplacementText" => EditIntent::Reconcile,
        "insertParagraph" => EditIntent::Command("enter"),
        // Shift+Enter's command: a newline in a code block, else a hard break
        // (#2993), as the keymap's Shift-Enter.
        "insertLineBreak" => EditIntent::Command("insertLineBreak"),
        // `deleteContent*` is a single char (or the selection, which both commands
        // delete first); `deleteContent` itself only ever means "the selection".
        "deleteContent" => EditIntent::Command("deleteSelection"),
        "deleteContentBackward" => EditIntent::Command("deleteCharBackward"),
        "deleteContentForward" => EditIntent::Command("deleteCharForward"),
        // The core commands carry the block-edge and inline-atom rules (#303).
        "deleteWordBackward" => EditIntent::Command("deleteWordBackward"),
        "deleteWordForward" => EditIntent::Command("deleteWordForward"),
        // A soft line is the visual line the caret is on; a hard line is the
        // textblock (#301).
        "deleteSoftLineBackward" => EditIntent::DeleteToVisualLine { forward: false },
        "deleteSoftLineForward" => EditIntent::DeleteToVisualLine { forward: true },
        "deleteHardLineBackward" => EditIntent::DeleteTo(CursorMotion::LineStart),
        "deleteHardLineForward" => EditIntent::DeleteTo(CursorMotion::LineEnd),
        "historyUndo" => EditIntent::Command("undo"),
        "historyRedo" => EditIntent::Command("redo"),
        // The `cut` / `paste` listeners already ran and applied their own edit.
        "deleteByCut" | "insertFromPaste" | "insertFromPasteAsQuotation" => EditIntent::Ignore,
        _ => EditIntent::Ignore,
    }
}

/// Delete from the caret to `motion`'s target. An already non-empty selection is
/// deleted as it stands, matching what the browser would have done.
fn delete_to(handle: &EditorHandle, motion: CursorMotion) -> bool {
    if handle.selection().is_empty() {
        handle.move_cursor(motion, true);
    }
    delete_extended(handle, !matches!(motion, CursorMotion::LineStart))
}

/// Delete from the caret to the edge of its visual line ([`visual_line_bound`]),
/// or to the textblock's edge when there is no geometry to ask. An already
/// non-empty selection is deleted as it stands.
fn delete_to_visual_line(handle: &EditorHandle, forward: bool) -> bool {
    let sel = handle.selection();
    if sel.is_empty() {
        let head = sel.head();
        match visual_line_bound(handle, head, forward) {
            Some(edge) if edge != head => handle.set_selection(Selection::text(head, edge)),
            Some(_) => {}
            None => {
                handle.move_cursor(
                    if forward {
                        CursorMotion::LineEnd
                    } else {
                        CursorMotion::LineStart
                    },
                    true,
                );
            }
        }
    }
    delete_extended(handle, forward)
}

/// Delete the selection a line delete extended over; if the extension had
/// nowhere to go, delete one character in its direction instead.
fn delete_extended(handle: &EditorHandle, forward: bool) -> bool {
    if handle.selection().is_empty() {
        // The motion had nowhere to go: the caret is already at the line's edge —
        // for a model-line motion, the textblock's. A line delete there means what
        // Backspace and Delete mean — join with the adjacent block, or at a soft
        // wrap take the character there. Without this the gesture is swallowed
        // silently, because the `beforeinput` was already `preventDefault`ed.
        return handle.command(if forward {
            "deleteCharForward"
        } else {
            "deleteCharBackward"
        });
    }
    handle.command("deleteSelection")
}

/// Route a `beforeinput` on the capture textarea into the focused editor's model.
///
/// The textarea is never the document — whatever the browser was about to do to it is
/// wrong — so every event we recognise is `preventDefault`ed and mirrored onto the
/// model instead. An in-flight composition is left entirely alone: cancelling
/// `insertCompositionText` breaks the IME, and the composition events already commit it.
pub(super) fn on_before_input(event: &web_sys::InputEvent) {
    if event.is_composing() || COMPOSING.with(|c| c.get()) {
        return;
    }
    // The menu's Undo / Redo arrive here when Chrome aims them at this field rather
    // than at another one (`on_foreign_history_input`); its Paste as plain text does
    // not, since it fires only `paste` (measured). The cycle ends first so the field
    // the browser is about to edit is the mirror, not the sentinels.
    end_context_menu_cycle();
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    let handled = match edit_intent(&event.input_type()) {
        // Let it land in the textarea; `on_input` diffs the mirror and applies it.
        EditIntent::Reconcile => return,
        // Cancel it anyway: an edit we are not applying must not desync the mirror
        // from the document either.
        EditIntent::Ignore => {
            event.prevent_default();
            return;
        }
        EditIntent::InsertText => {
            event.prevent_default();
            match event.data() {
                Some(text) if !text.is_empty() => handle.insert_text(&text),
                _ => false,
            }
        }
        EditIntent::Command(name) => {
            event.prevent_default();
            handle.command(name)
        }
        EditIntent::DeleteTo(motion) => {
            event.prevent_default();
            delete_to(&handle, motion)
        }
        EditIntent::DeleteToVisualLine { forward } => {
            event.prevent_default();
            delete_to_visual_line(&handle, forward)
        }
    };
    // Typing is a caret move as much as an edit: drop any vertical-motion goal column
    // so a following Up/Down starts from where the text actually landed.
    set_goal_x(None);
    if handled {
        refresh_caret();
    }
}

/// Reconcile the document with whatever the browser just did to the capture textarea.
///
/// Most edits are intercepted at `beforeinput` and never reach the textarea, so this
/// usually finds it already matching the mirror and does nothing. It is the path for
/// the ones we deliberately let through ([`EditIntent::Reconcile`]) and the safety net
/// for a keyboard that edits the field with no `inputType` we know. Composition is
/// exempt — the IME owns the field until `compositionend`, which reconciles it.
pub(super) fn on_input() {
    if COMPOSING.with(|c| c.get()) {
        return;
    }
    end_context_menu_cycle();
    let Some((_, handle)) = focused_handle() else {
        // Nothing to apply it to; just don't leave text for a later `copy` to pick up
        // (and drop the mirror with it, so nothing later diffs against a cleared field).
        if let Some(ta) = capture_target() {
            clear_mirror(&ta);
        }
        return;
    };
    if reconcile_mirror(&handle) {
        refresh_caret();
    }
}
