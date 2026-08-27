//! Android's `InputConnection` composition protocol, translated for the shell.
//!
//! Android does not hand an app finished words. A soft keyboard doing
//! autocorrect, swipe-to-type or CJK conversion first *composes*: it calls
//! `setComposingText` once per keystroke with the whole word-so-far, and only
//! later ends the region with `commitText` or `finishComposingText`. The
//! composing text is what the user sees underlined under their finger, and
//! until this module existed rinch threw all of it away — `setComposingText`
//! was a stub that returned `true` and forwarded nothing, so the only text that
//! ever reached the document was what a commit carried.
//!
//! Two things about Android's protocol drive the shape of what follows.
//!
//! **The composing region is replaced, never appended to.** `setComposingText`
//! always carries the entire region, so `h`, `he`, `hel` is three calls, not
//! three characters. That is exactly [`ImeEvent::Preedit`]'s contract — the
//! preedit is set or cleared, never accumulated — so the two line up without
//! any diffing.
//!
//! **Android's composing text is real text; rinch's preedit is not.** In an
//! `EditText` the composing characters are already in the buffer and the
//! composing region is just a pair of spans over them, which is why
//! `finishComposingText` means "keep it, stop composing". Rinch keeps the
//! preedit *outside* the field's value (`data-preedit`, spliced in at paint
//! time only) so an abandoned composition cannot leave debris in the value.
//! The consequence: **finishing a composition has to commit it here.**
//! Treating `finishComposingText` as a clear would delete the word the user
//! just accepted, and accepting a word by tapping away is one of the two ways
//! Gboard ends a composition.
//!
//! [`ImeComposition`] is the one place the region is mirrored. It is a pure
//! state machine over the four calls, which is why it lives here and not in the
//! JNI layer or in the run loop — the run loop only turns [`ImeAction`]s into
//! platform events, and this file is compiled for the host test build.

/// What the run loop must do with one `InputConnection` call.
///
/// Deliberately not `ImeEvent`: a commit is applied as key input on Android
/// (see the run loop's note on why that path is kept), so an action list can
/// mix "clear the inline composition" with "insert this text the way a
/// keystroke would". The order within the list is load-bearing — a commit
/// clears the preedit *before* it inserts, so the composition never lingers
/// beside the text that replaced it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImeAction {
    /// Set (or, with an empty `text`, clear) the inline composition drawn at
    /// the caret. Maps to [`rinch_platform::ImeEvent::Preedit`].
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// Insert committed text at the caret.
    Insert(String),
    /// Delete `before` characters before the caret and `after` after it.
    Delete { before: usize, after: usize },
}

/// The IME's composing region, mirrored on the native thread.
///
/// The mirror exists so a composition can be ended without the IME's help:
/// backgrounding the app and moving focus between two of rinch's fields are
/// both invisible to Android (there is one `RinchInputView`, and it keeps
/// focus throughout), so nobody else is in a position to say what the region
/// held. Every path that ends a composition empties it, which makes a late
/// `finishComposingText` from the framework a no-op rather than a second
/// insertion of the same word.
#[derive(Debug, Default)]
pub(crate) struct ImeComposition {
    /// The composing region as the IME last set it. Empty means not composing.
    composing: String,
}

impl ImeComposition {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `InputConnection.setComposingText(text, newCursorPosition)` — replace
    /// the composing region.
    pub(crate) fn set_composing_text(
        &mut self,
        text: String,
        new_cursor_position: i32,
    ) -> Vec<ImeAction> {
        // An empty region is the *end* of a composition, not a composition with
        // its caret at zero — so it clears with `cursor: None`, exactly like
        // the clear a commit or a finish emits. One representation, one state.
        let cursor = if text.is_empty() {
            None
        } else {
            composing_cursor(&text, new_cursor_position)
        };
        self.composing = text.clone();
        vec![ImeAction::Preedit { text, cursor }]
    }

    /// `InputConnection.finishComposingText()` — the IME is done composing and
    /// the region's text stands. It is not in the value yet (see the module
    /// note), so it is committed here.
    pub(crate) fn finish_composing_text(&mut self) -> Vec<ImeAction> {
        let text = std::mem::take(&mut self.composing);
        if text.is_empty() {
            return Vec::new();
        }
        vec![
            ImeAction::Preedit {
                text: String::new(),
                cursor: None,
            },
            ImeAction::Insert(text),
        ]
    }

    /// `InputConnection.commitText(text, newCursorPosition)` — replace the
    /// composing region, if any, with real text.
    ///
    /// `text` is the committed string, which need not be what was composed:
    /// autocorrect composes `teh` and commits `the`, and a tapped suggestion
    /// commits a word with the trailing space attached. Clearing the preedit
    /// first is what makes the substitution look like one.
    pub(crate) fn commit_text(&mut self, text: String) -> Vec<ImeAction> {
        let was_composing = !std::mem::take(&mut self.composing).is_empty();
        let mut actions = Vec::new();
        if was_composing {
            actions.push(ImeAction::Preedit {
                text: String::new(),
                cursor: None,
            });
        }
        if !text.is_empty() {
            actions.push(ImeAction::Insert(text));
        }
        actions
    }

    /// `InputConnection.deleteSurroundingText(before, after)`.
    ///
    /// Android's surrounding text is the text *around* the composing region, so
    /// the region is deliberately left alone: an IME shortening a word sends a
    /// shorter `setComposingText`, and only reaches for this to edit what is
    /// already committed either side of it.
    ///
    /// **The lengths are UTF-16 code units, and are passed on as if they were
    /// characters.** `ImeEvent::DeleteSurrounding` counts characters (rinch's
    /// model is char-based) and says the platform boundary converts — which
    /// this cannot do, because the connection never sees the field's text. The
    /// two agree for the BMP and disagree by one deletion per astral character:
    /// a word containing an emoji deletes one character too many. Closing it
    /// needs the surrounding-text queries the connection does not yet answer.
    pub(crate) fn delete_surrounding_text(&mut self, before: i32, after: i32) -> Vec<ImeAction> {
        let before = before.max(0) as usize;
        let after = after.max(0) as usize;
        if before == 0 && after == 0 {
            return Vec::new();
        }
        vec![ImeAction::Delete { before, after }]
    }

    /// The focused `<input>` changed while a composition was in flight.
    ///
    /// Nothing is inserted: the focus arbiter has already committed the preedit
    /// into the field that lost focus (`set_focus_target_deferred`'s
    /// compositionend-before-blur), and the field that gained it must not
    /// receive the same word a second time. Returns whether there was a
    /// composition to abandon — the caller restarts the IME only then, so a
    /// focus move with no composition in flight is left entirely alone.
    pub(crate) fn abandon(&mut self) -> bool {
        !std::mem::take(&mut self.composing).is_empty()
    }
}

/// Android's `newCursorPosition`, resolved to a byte offset inside `text`.
///
/// `InputConnection`: "If > 0, this is relative to the end of the text - 1; if
/// <= 0, this is relative to the start of the text." Both count *characters*,
/// and both can name a position in the surrounding text, which rinch does not
/// model — so a position outside the composing region clamps to the nearest
/// end of it. [`rinch_platform::ImeEvent::Preedit`]'s cursor is a **byte**
/// range within `text`, hence the conversion; an empty range is a caret rather
/// than a selection, which is what a composing caret is.
///
/// The paint path currently ignores this and draws the caret at the end of the
/// composition unconditionally (`paint_input_value`), so today this is carried
/// rather than obeyed. It is mapped properly anyway because the alternative is
/// to have to work out what an IME meant by it later, from a log.
fn composing_cursor(text: &str, new_cursor_position: i32) -> Option<(usize, usize)> {
    let chars = text.chars().count() as i64;
    let index = if new_cursor_position > 0 {
        chars + new_cursor_position as i64 - 1
    } else {
        new_cursor_position as i64
    }
    .clamp(0, chars) as usize;
    let byte = text
        .char_indices()
        .nth(index)
        .map(|(b, _)| b)
        .unwrap_or(text.len());
    Some((byte, byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preedit(text: &str, cursor: usize) -> ImeAction {
        ImeAction::Preedit {
            text: text.to_string(),
            cursor: Some((cursor, cursor)),
        }
    }

    fn clear() -> ImeAction {
        ImeAction::Preedit {
            text: String::new(),
            cursor: None,
        }
    }

    // ── The composing region ─────────────────────────────────────────────

    /// The stub this replaces returned `true` and forwarded nothing, so a
    /// composing keyboard drew nothing until it committed.
    #[test]
    fn a_composing_call_becomes_a_preedit() {
        let mut c = ImeComposition::new();
        assert_eq!(
            c.set_composing_text("hel".to_string(), 1),
            vec![preedit("hel", 3)]
        );
    }

    /// Android sends the whole region every time. Appending instead would spell
    /// `hhehel`.
    #[test]
    fn each_composing_call_replaces_the_region_rather_than_extending_it() {
        let mut c = ImeComposition::new();
        c.set_composing_text("h".to_string(), 1);
        c.set_composing_text("he".to_string(), 1);
        assert_eq!(
            c.set_composing_text("hel".to_string(), 1),
            vec![preedit("hel", 3)],
            "the third call carries the whole word, not the third letter"
        );
    }

    /// Backspacing to nothing: an empty composing call ends the region.
    #[test]
    fn an_empty_composing_call_clears_the_preedit_and_ends_the_composition() {
        let mut c = ImeComposition::new();
        c.set_composing_text("h".to_string(), 1);
        assert_eq!(c.set_composing_text(String::new(), 1), vec![clear()]);
        assert!(
            c.finish_composing_text().is_empty(),
            "nothing is left composing to finish"
        );
    }

    // ── Ending a composition ─────────────────────────────────────────────

    /// The order is the point: the composition is cleared before the text that
    /// replaces it is inserted, so `teh` never appears beside `the`.
    #[test]
    fn a_commit_clears_the_composition_before_it_inserts() {
        let mut c = ImeComposition::new();
        c.set_composing_text("teh".to_string(), 1);
        assert_eq!(
            c.commit_text("the".to_string()),
            vec![clear(), ImeAction::Insert("the".to_string())]
        );
    }

    /// A keyboard that is not composing — a plain punctuation key, `adb shell
    /// input text` — commits with no region open, and must not pay for a
    /// preedit event it never had.
    #[test]
    fn a_commit_with_nothing_composed_only_inserts() {
        let mut c = ImeComposition::new();
        assert_eq!(
            c.commit_text("a".to_string()),
            vec![ImeAction::Insert("a".to_string())]
        );
    }

    /// Committing the empty string is how an IME throws a composing region
    /// away. It clears, and inserts nothing.
    #[test]
    fn an_empty_commit_clears_the_composition_and_inserts_nothing() {
        let mut c = ImeComposition::new();
        c.set_composing_text("hel".to_string(), 1);
        assert_eq!(c.commit_text(String::new()), vec![clear()]);
    }

    /// Accepting a word by tapping away: Gboard ends the region with
    /// `finishComposingText` and no commit. Discarding it here would delete the
    /// word the user just typed.
    #[test]
    fn finishing_a_composition_commits_what_was_composed() {
        let mut c = ImeComposition::new();
        c.set_composing_text("wagon".to_string(), 1);
        assert_eq!(
            c.finish_composing_text(),
            vec![clear(), ImeAction::Insert("wagon".to_string())]
        );
    }

    #[test]
    fn finishing_with_nothing_composed_does_nothing() {
        let mut c = ImeComposition::new();
        assert!(c.finish_composing_text().is_empty());
    }

    /// The framework finishes the composition on the connection after a commit
    /// has already ended it. Without the mirror being emptied by the commit,
    /// that would insert the word twice.
    #[test]
    fn a_finish_after_a_commit_does_not_insert_the_word_twice() {
        let mut c = ImeComposition::new();
        c.set_composing_text("hello".to_string(), 1);
        c.commit_text("hello ".to_string());
        assert!(c.finish_composing_text().is_empty());
    }

    // ── Abandoning one ───────────────────────────────────────────────────

    /// Focus moved to another of rinch's fields. The blurred field already got
    /// the text from the focus arbiter, so nothing is inserted here — but the
    /// IME has to be told, which is what the `true` is for.
    #[test]
    fn abandoning_reports_whether_there_was_a_composition() {
        let mut c = ImeComposition::new();
        assert!(!c.abandon(), "no composition, no restart");
        c.set_composing_text("hel".to_string(), 1);
        assert!(c.abandon());
        assert!(!c.abandon(), "and it is gone");
    }

    /// The word must not follow focus into the next field.
    #[test]
    fn a_composition_abandoned_at_a_focus_change_is_not_committed_again() {
        let mut c = ImeComposition::new();
        c.set_composing_text("hel".to_string(), 1);
        c.abandon();
        assert!(c.finish_composing_text().is_empty());
        assert_eq!(
            c.commit_text("x".to_string()),
            vec![ImeAction::Insert("x".to_string())],
            "and the next commit is not preceded by a stale clear"
        );
    }

    // ── The caret inside the region ──────────────────────────────────────

    /// `1` is what every keyboard sends for "caret after the composing text",
    /// and it is the only value this has been seen to carry.
    #[test]
    fn the_usual_cursor_position_is_the_end_of_the_region() {
        assert_eq!(composing_cursor("hel", 1), Some((3, 3)));
    }

    #[test]
    fn a_cursor_position_at_or_before_the_region_clamps_to_its_start() {
        assert_eq!(composing_cursor("hel", 0), Some((0, 0)));
        assert_eq!(composing_cursor("hel", -3), Some((0, 0)));
        assert_eq!(
            composing_cursor("hel", -99),
            Some((0, 0)),
            "a position in surrounding text rinch does not model"
        );
    }

    #[test]
    fn a_cursor_position_past_the_region_clamps_to_its_end() {
        assert_eq!(composing_cursor("hel", 2), Some((3, 3)));
        assert_eq!(composing_cursor("hel", 99), Some((3, 3)));
    }

    /// `ImeEvent::Preedit`'s cursor is a byte range. Three CJK characters are
    /// nine bytes, and reporting `3` would land mid-character.
    #[test]
    fn the_cursor_is_a_byte_offset_not_a_character_index() {
        assert_eq!(composing_cursor("日本語", 1), Some((9, 9)));
        assert_eq!(composing_cursor("日本語", 0), Some((0, 0)));
        assert_eq!(
            composing_cursor("日本語", -1),
            Some((0, 0)),
            "clamped, and still on a character boundary"
        );
    }

    #[test]
    fn an_empty_region_has_its_caret_at_zero() {
        assert_eq!(composing_cursor("", 1), Some((0, 0)));
    }

    // ── Deletions ────────────────────────────────────────────────────────

    #[test]
    fn a_deletion_passes_through_and_leaves_the_composition_alone() {
        let mut c = ImeComposition::new();
        c.set_composing_text("hel".to_string(), 1);
        assert_eq!(
            c.delete_surrounding_text(2, 0),
            vec![ImeAction::Delete {
                before: 2,
                after: 0
            }]
        );
        assert_eq!(
            c.finish_composing_text(),
            vec![clear(), ImeAction::Insert("hel".to_string())],
            "the region survived the surrounding-text edit"
        );
    }

    #[test]
    fn a_deletion_of_nothing_is_nothing() {
        let mut c = ImeComposition::new();
        assert!(c.delete_surrounding_text(0, 0).is_empty());
        assert!(
            c.delete_surrounding_text(-1, -1).is_empty(),
            "a negative length is not a deletion in the other direction"
        );
    }

    // ── Whole gestures ───────────────────────────────────────────────────

    /// Swipe-to-type, as Gboard drives it: one composition that grows, then one
    /// commit carrying the word and its trailing space.
    #[test]
    fn a_swiped_word_draws_as_it_grows_and_commits_once() {
        let mut c = ImeComposition::new();
        let mut drawn: Vec<String> = Vec::new();
        for step in ["w", "wa", "wag", "wago", "wagon"] {
            for action in c.set_composing_text(step.to_string(), 1) {
                if let ImeAction::Preedit { text, .. } = action {
                    drawn.push(text);
                }
            }
        }
        assert_eq!(drawn, vec!["w", "wa", "wag", "wago", "wagon"]);
        assert_eq!(
            c.commit_text("wagon ".to_string()),
            vec![clear(), ImeAction::Insert("wagon ".to_string())]
        );
    }

    /// A CJK conversion: the reading is composed, the chosen candidate is
    /// committed, and the reading never reaches the value.
    #[test]
    fn a_cjk_conversion_commits_the_candidate_not_the_reading() {
        let mut c = ImeComposition::new();
        c.set_composing_text("に".to_string(), 1);
        c.set_composing_text("にほん".to_string(), 1);
        assert_eq!(
            c.commit_text("日本".to_string()),
            vec![clear(), ImeAction::Insert("日本".to_string())]
        );
    }
}
