//! The platform's floating text-selection toolbar (issue #813).

use rinch_android::text_action::{
    PhysicalRect, TextAction, TextActionEvent, TextActionItems, ToolbarMirror, ToolbarPush,
};
use rinch_platform::{MouseButton, PlatformEvent};

use super::input::TurnInput;
use super::loop_state::LoopState;
use super::process_actions;
use crate::app::{TextEditAction, TextEditState};

/// The seam's anchor — the selection's real rect, or a 1px caret rect when it
/// is collapsed, in logical window px — in the physical window pixels the
/// decor view measures in. The platform floats the toolbar above that rect
/// (below it near the top of the window).
fn physical_rect(anchor: (f32, f32, f32, f32), scale_factor: f64) -> PhysicalRect {
    let (x, y, w, h) = anchor;
    let px = |v: f32| (v as f64 * scale_factor).round() as i32;
    PhysicalRect {
        left: px(x),
        top: px(y),
        right: px(x + w.max(1.0)),
        bottom: px(y + h),
    }
}

/// The seam's four flags as the toolbar's items. Shown or hidden rather than
/// disabled, which is what the platform's own text views do. Paste is
/// `can_paste` — the field's own answer — and, as in a platform `EditText`
/// (`Editor.canPaste` is `hasPrimaryClip` plus a text description), only
/// while the clipboard holds something. That is `clipboard_has_clip`, from
/// `hasPrimaryClip()`: it reads no clip, so the #149 rule about not *reading*
/// the clipboard to decide is kept, and it raises no Android 12+
/// "… pasted from your clipboard" notice — measured on an API 34 emulator with
/// a clip another app set, where the toolbar's own Paste, which does read it,
/// raises one.
///
/// It is still a binder round trip, so the loop asks it once per long press
/// and hands the answer in, rather than asking on every refresh: measured on
/// that emulator, a call costs 0.56 ms at the median and 10.7 ms at the 99th
/// percentile (worst 64 ms, n = 2475), and the refresh runs every turn of the
/// loop while the toolbar is up — 50 to 60 times a second there. The cost of
/// asking less often: a clip that arrives while a toolbar is up — the app's
/// own code writing one, another app writing one in the background — shows
/// on the next long press, where a platform `EditText` re-decides whenever it
/// re-prepares its toolbar. The routes by which the *user* writes one through
/// rinch end the toolbar first: its own Cut and Copy finish it, an IME's
/// `performContextMenuAction` Cut and Copy do (see
/// `ToolbarMirror::performing`), and a hardware chord is a key press.
fn toolbar_items(state: &TextEditState, clipboard_has_clip: bool) -> TextActionItems {
    TextActionItems {
        cut: state.can_cut,
        copy: state.can_copy,
        paste: state.can_paste && clipboard_has_clip,
        select_all: state.can_select_all,
    }
}

/// Show the toolbar for `state` — the text target holding the keyboard — or
/// bring the one that is up in line with it. The mirror decides whether the
/// platform has to be told at all. Takes the state rather than the app so each
/// turn polls `text_edit_state()` once: the `<input>`/`<textarea>` anchor is
/// measured from the field's text layout.
///
/// **A refresh never starts a toolbar** (PR #819 final review, N1). The push
/// runs on the UI thread later than this decides it, and a tap on an item can
/// finish the mode in between; a refresh allowed to start would put a new mode
/// on screen behind that item, with the mirror taken down by the item's report
/// — an orphan whose Paste did nothing. Measured on an API 34 emulator with an
/// anchor moving every frame: 7 orphans among the 8 item taps that reached the
/// loop with the Java `start` guard deleted, 0 among 16 with it.
/// So the platform may start one only when the mirror believes none is up
/// ([`ToolbarPush::Start`]) or when `long_press` says the user just asked for
/// it — the latter for a long press on a toolbar the mirror still shows,
/// whose mode could be gone without its report drained yet.
pub(super) fn push_text_toolbar(
    mirror: &mut ToolbarMirror,
    state: &TextEditState,
    scale_factor: f64,
    clipboard_has_clip: bool,
    long_press: bool,
) {
    let rect = physical_rect(state.anchor, scale_factor);
    let items = toolbar_items(state, clipboard_has_clip);
    match mirror.request(rect, items) {
        ToolbarPush::Nothing => {}
        push => rinch_android::text_action::show_toolbar(
            rect,
            items,
            push == ToolbarPush::Start || long_press,
        ),
    }
}

impl LoopState {
    /// A long press on a text field asks for the platform's floating
    /// text-selection toolbar (issue #813). The press itself — a right
    /// button, from `tick_long_press` — was dispatched just before this under
    /// `TextContextMenuPresentation::Shell`, so the field it landed on
    /// holds the input claim and its caret is placed by the seam's caret
    /// rule; the toolbar floats beside the state the seam reports. Any
    /// other press, a key and a scroll take an open toolbar down, as they
    /// do over a platform text view.
    pub(super) fn answer_toolbar_request(&mut self, input: &TurnInput) {
        let pressed_elsewhere = input.events.iter().any(|e| {
            matches!(
                e,
                PlatformEvent::MouseDown {
                    button: MouseButton::Left,
                    ..
                } | PlatformEvent::KeyDown { .. }
                    | PlatformEvent::MouseWheel { .. }
            )
        });
        if input.toolbar_requested {
            // The platform convention: a long press selects the word under
            // the finger (a press inside an existing selection keeps it), and
            // an empty field — or a press on whitespace — keeps its caret and
            // offers only Paste.
            self.app.select_word_at_caret();
            self.clipboard_has_clip = rinch_android::clipboard::has_text();
            if let Some(state) = self.app.text_edit_state() {
                push_text_toolbar(
                    &mut self.text_toolbar,
                    &state,
                    self.scale_factor,
                    self.clipboard_has_clip,
                    true,
                );
            }
        } else if pressed_elsewhere && self.text_toolbar.finish() {
            rinch_android::text_action::finish_toolbar();
        }
    }

    /// What the platform did with the text actions since the last turn: a
    /// toolbar item tapped, an IME's own `performContextMenuAction`, or the
    /// toolbar going away. Performed here, on the loop, through the seam —
    /// which runs exactly the code the keyboard shortcut runs (issue #813).
    pub(super) fn perform_text_actions(&mut self, toolbar_requested: bool) {
        for event in rinch_android::text_action::drain_text_action_events() {
            match event {
                TextActionEvent::ToolbarDismissed => self.text_toolbar.dismissed(),
                TextActionEvent::Perform { action, source } => {
                    log::info!("text action: {action:?} from {source:?}");
                    // Cut, Copy and Paste end the toolbar's interaction, from
                    // either source. A toolbar item has finished the mode on
                    // the Java side already — before reporting the item, so
                    // its dismissal is normally drained by now — and the
                    // mirror only has to stop believing in it, or the refresh
                    // after this would re-prepare it and, on the UI thread,
                    // start a new one (PR #819 review, F1). An IME request
                    // finished nothing, so the loop asks. Not when a fresh
                    // toolbar was requested this very turn: that one is newer
                    // than anything this drain holds.
                    if !toolbar_requested && self.text_toolbar.performing(action, source) {
                        rinch_android::text_action::finish_toolbar();
                    }
                    let actions = self.app.perform_text_edit(match action {
                        TextAction::Cut => TextEditAction::Cut,
                        TextAction::Copy => TextEditAction::Copy,
                        TextAction::Paste => TextEditAction::Paste,
                        TextAction::SelectAll => TextEditAction::SelectAll,
                    });
                    process_actions(&actions, &mut self.running);
                }
            }
        }
    }

    /// Keep an open toolbar current: the field's state is re-read every
    /// turn, so Cut and Copy appear the moment Select all made a selection,
    /// the toolbar follows a caret the user moved, and it goes when the
    /// field loses the keyboard. `ToolbarMirror` makes this cost nothing
    /// while nothing changed. A refresh never starts a toolbar (see
    /// `push_text_toolbar`). Skipped on a long-press turn, which pushed the
    /// state it just read.
    pub(super) fn refresh_text_toolbar(&mut self, toolbar_requested: bool) {
        if self.text_toolbar.is_shown() && !toolbar_requested {
            match self.app.text_edit_state() {
                Some(state) => push_text_toolbar(
                    &mut self.text_toolbar,
                    &state,
                    self.scale_factor,
                    self.clipboard_has_clip,
                    false,
                ),
                None => {
                    if self.text_toolbar.finish() {
                        rinch_android::text_action::finish_toolbar();
                    }
                }
            }
        }
    }
}
