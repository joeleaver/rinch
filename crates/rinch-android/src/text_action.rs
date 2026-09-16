//! The platform's floating text-selection toolbar — Cut / Copy / Paste /
//! Select all — over a rinch text field, and the same four actions when the
//! soft keyboard asks for them itself (issue #813).
//!
//! Two producers feed one queue. `RinchActivity.showTextActionMode` starts an
//! `ActionMode.TYPE_FLOATING` on the window's decor view (see the Java for why
//! the decor view and not `RinchInputView`), and a tap on one of its items
//! arrives here as [`TextActionEvent::Perform`]. Separately, an IME that
//! implements its own clipboard UI — Gboard's clipboard panel, the "paste"
//! chip on its suggestion strip — may deliver a paste not as committed text but
//! as `InputConnection.performContextMenuAction(android.R.id.paste)`, and
//! `BaseInputConnection`'s default for that call does **nothing**.
//! `RinchInputConnection` overrides it and the request lands in this same
//! queue, so an IME's paste reaches the field whether or not the toolbar is
//! involved. The frame loop drains the queue once per turn
//! ([`drain_text_action_events`]) and performs each action through the same
//! path the keyboard shortcut takes.
//!
//! Java maps `android.R.id.*` to the small integers in [`TextAction::from_code`]
//! before crossing JNI, so no Android resource id is spelled on this side.
//!
//! **What clears the toolbar.** It is armed by one event — the long press that
//! showed it — and the frame loop finishes it on the next pointer press, on a
//! keystroke, on an IME edit, and when the field loses focus; those are the
//! shell's own conditions and any one of them may be missed (a release nobody
//! saw, a focus move the loop never dispatched). The independent clearing
//! condition is on the Java side and needs no native call: `RinchActivity`
//! finishes the mode in `onPause` and on window-focus loss, and every
//! `showTextActionMode` replaces whatever mode is up. The Java side also
//! reports every dismissal ([`TextActionEvent::ToolbarDismissed`]) — Back,
//! `finish()`, a replacement — so the loop's own "is it showing" mirror can
//! never stay latched on a toolbar that is gone.

use std::sync::Mutex;

/// One of the four platform text actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl TextAction {
    /// The integer `RinchActivity.textActionCode` / `RinchInputConnection`
    /// send for each action. The mapping is the Java side's, restated here;
    /// `tests/java_contract_tests.rs` pins that the two agree.
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Cut),
            1 => Some(Self::Copy),
            2 => Some(Self::Paste),
            3 => Some(Self::SelectAll),
            _ => None,
        }
    }
}

/// Something the platform did with the text actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextActionEvent {
    /// The user tapped a toolbar item, or the IME asked for the action through
    /// `performContextMenuAction`. Either way the action has **not** been
    /// performed yet — the loop performs it.
    Perform(TextAction),
    /// The floating toolbar is gone: Back, `finish()`, the activity pausing, or
    /// a replacement. Sent from `onDestroyActionMode`, so it fires for every
    /// way a mode can end.
    ToolbarDismissed,
}

/// Which items the toolbar shows. Android hides rather than disables: Cut and
/// Copy go with an empty selection, Paste with a read-only field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextActionItems {
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub select_all: bool,
}

/// The rect the toolbar floats beside, in **physical** pixels in window
/// coordinates — the space the decor view measures in, and the same space the
/// shell's touch events arrive in before it divides by the scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

static EVENTS: Mutex<Vec<TextActionEvent>> = Mutex::new(Vec::new());

/// Take everything the platform reported since the last drain, in order.
pub fn drain_text_action_events() -> Vec<TextActionEvent> {
    std::mem::take(&mut *EVENTS.lock().unwrap())
}

/// The host-compiled half of every JNI entry point below: queue the event and
/// wake the frame loop, which since K37 sleeps on the looper with no timeout —
/// a toolbar tap happens on a still screen, so without the wake the paste would
/// wait for the next touch.
#[cfg(any(target_os = "android", test))]
fn queue(event: TextActionEvent) {
    EVENTS.lock().unwrap().push(event);
    crate::wake::wake_main();
}

/// Queue a `Perform` for the code Java sent, or nothing for a code it should
/// not have sent. Shared by the toolbar's item click and the IME's
/// `performContextMenuAction`, which are the same request from two sources.
#[cfg(any(target_os = "android", test))]
fn queue_action_code(code: i32) {
    match TextAction::from_code(code) {
        Some(action) => queue(TextActionEvent::Perform(action)),
        None => log::warn!("text action: Java sent an unknown action code {code}; ignored"),
    }
}

// ── JNI entry points ────────────────────────────────────────────────────────

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnTextActionItem(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    code: jni::sys::jint,
) {
    queue_action_code(code);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnTextActionModeFinished(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    queue(TextActionEvent::ToolbarDismissed);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeContextMenuAction(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    code: jni::sys::jint,
) {
    queue_action_code(code);
}

// ── Calls into Java (Android only) ──────────────────────────────────────────

/// Show the floating toolbar beside `rect` with the given items, or move and
/// re-prepare the one already showing. Hops to the UI thread on the Java side.
#[cfg(target_os = "android")]
pub fn show_toolbar(rect: PhysicalRect, items: TextActionItems) {
    use jni::objects::JValue;
    crate::bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "showTextActionMode",
            "(IIIIZZZZ)V",
            &[
                JValue::Int(rect.left),
                JValue::Int(rect.top),
                JValue::Int(rect.right),
                JValue::Int(rect.bottom),
                JValue::Bool(items.cut as jni::sys::jboolean),
                JValue::Bool(items.copy as jni::sys::jboolean),
                JValue::Bool(items.paste as jni::sys::jboolean),
                JValue::Bool(items.select_all as jni::sys::jboolean),
            ],
        ) {
            log::warn!("showTextActionMode failed: {e}");
        }
    });
}

/// Take the toolbar down. A no-op when none is showing.
#[cfg(target_os = "android")]
pub fn finish_toolbar() {
    crate::bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "finishTextActionMode", "()V", &[]) {
            log::warn!("finishTextActionMode failed: {e}");
        }
    });
}

// ── The loop's mirror of the toolbar ────────────────────────────────────────

/// What the frame loop believes about the toolbar, and the one place that
/// decides when the platform has to be told something.
///
/// The truth lives on the Java side and every change to it is asynchronous —
/// a request hops to the UI thread, and the report that a mode ended
/// (`onDestroyActionMode`) comes back through the queue a turn later. Two
/// things follow, and both are this struct's job. A *refresh* (the loop re-reads
/// the field's state every turn) must not cost a JNI call when nothing moved,
/// so the last rect and items pushed are kept and compared. And a finish
/// followed by a fresh request before the old mode's report arrives must not
/// let that report take the mirror down under a toolbar that is up — so each
/// finish the loop asks for is counted, `finishTextActionMode` reports exactly
/// one dismissal per request whether or not it found a mode to finish, and a
/// report that pays off a counted finish says nothing about the current mode.
/// A report with nothing to pay off is the platform's own dismissal — Back,
/// an item that finished the mode, the activity pausing — and takes the
/// mirror down.
#[derive(Debug, Default)]
pub struct ToolbarMirror {
    shown: bool,
    pending_finishes: u32,
    last_pushed: Option<(PhysicalRect, TextActionItems)>,
}

impl ToolbarMirror {
    pub const fn new() -> Self {
        Self {
            shown: false,
            pending_finishes: 0,
            last_pushed: None,
        }
    }

    /// Whether the loop believes a toolbar is up.
    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// The loop wants the toolbar beside `rect` with `items`. Answers whether
    /// the platform has to be told: always for a toolbar that is not up, and
    /// for one that is only when something differs from what was last pushed.
    #[must_use = "the platform is told only if the caller acts on `true`"]
    pub fn request(&mut self, rect: PhysicalRect, items: TextActionItems) -> bool {
        let want = Some((rect, items));
        if self.shown && self.last_pushed == want {
            return false;
        }
        self.shown = true;
        self.last_pushed = want;
        true
    }

    /// The loop wants the toolbar gone. Answers whether the platform has to be
    /// told, which is only when the loop believes one is up.
    #[must_use = "the platform is told only if the caller acts on `true`"]
    pub fn finish(&mut self) -> bool {
        if !self.shown {
            return false;
        }
        self.shown = false;
        self.last_pushed = None;
        self.pending_finishes += 1;
        true
    }

    /// Java reported a mode ended (or a finish request found none).
    pub fn dismissed(&mut self) {
        if self.pending_finishes > 0 {
            self.pending_finishes -= 1;
        } else {
            self.shown = false;
            self.last_pushed = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Java side's `textActionCode` and this side's `from_code` are one
    /// table written twice; this is the Rust half of the pin (the Java half is
    /// in `tests/java_contract_tests.rs`).
    #[test]
    fn the_four_codes_decode_and_nothing_else_does() {
        assert_eq!(TextAction::from_code(0), Some(TextAction::Cut));
        assert_eq!(TextAction::from_code(1), Some(TextAction::Copy));
        assert_eq!(TextAction::from_code(2), Some(TextAction::Paste));
        assert_eq!(TextAction::from_code(3), Some(TextAction::SelectAll));
        assert_eq!(TextAction::from_code(-1), None);
        assert_eq!(TextAction::from_code(4), None);
    }

    /// A toolbar tap and an IME `performContextMenuAction` both land in the one
    /// queue, in order, and an unknown code lands nowhere rather than as a
    /// mis-decoded action.
    #[test]
    fn producers_share_one_queue_in_order_and_an_unknown_code_is_dropped() {
        let _serial = crate::test_serial();
        drain_text_action_events();

        queue_action_code(3);
        queue_action_code(99);
        queue(TextActionEvent::ToolbarDismissed);
        queue_action_code(2);

        assert_eq!(
            drain_text_action_events(),
            vec![
                TextActionEvent::Perform(TextAction::SelectAll),
                TextActionEvent::ToolbarDismissed,
                TextActionEvent::Perform(TextAction::Paste),
            ]
        );
        assert!(
            drain_text_action_events().is_empty(),
            "a drain empties the queue"
        );
    }

    /// **Every producer the frame loop only drains has to wake it** — the
    /// `callback.rs` rule. A toolbar tap is the still-screen case exactly:
    /// the finger is on a popup window, not on the surface.
    #[test]
    fn a_queued_text_action_wakes_the_frame_loop() {
        let _serial = crate::test_serial();
        drain_text_action_events();

        let before = crate::wake::wake_count();
        queue_action_code(2);
        assert!(
            crate::wake::wake_count() > before,
            "a paste tap must wake the loop"
        );

        let before = crate::wake::wake_count();
        queue(TextActionEvent::ToolbarDismissed);
        assert!(
            crate::wake::wake_count() > before,
            "a dismissal must wake it too, or the mirror stays latched until the next touch"
        );
        drain_text_action_events();
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;

    const RECT: PhysicalRect = PhysicalRect {
        left: 10,
        top: 20,
        right: 12,
        bottom: 60,
    };
    const PASTE_ONLY: TextActionItems = TextActionItems {
        cut: false,
        copy: false,
        paste: true,
        select_all: true,
    };
    const ALL: TextActionItems = TextActionItems {
        cut: true,
        copy: true,
        paste: true,
        select_all: true,
    };

    #[test]
    fn a_request_pushes_once_and_a_refresh_pushes_only_what_changed() {
        let mut m = ToolbarMirror::new();
        assert!(!m.is_shown());
        assert!(m.request(RECT, PASTE_ONLY), "first request always pushes");
        assert!(m.is_shown());
        assert!(
            !m.request(RECT, PASTE_ONLY),
            "the same rect and items again is a no-op, not a JNI call per frame"
        );
        assert!(
            m.request(RECT, ALL),
            "a selection appearing changes the items"
        );
        let moved = PhysicalRect { left: 11, ..RECT };
        assert!(m.request(moved, ALL), "a caret move changes the rect");
    }

    #[test]
    fn a_finish_is_told_once_and_its_report_does_not_count_as_a_dismissal() {
        let mut m = ToolbarMirror::new();
        assert!(!m.finish(), "nothing up, nothing to tell");
        assert!(m.request(RECT, ALL));
        assert!(m.finish());
        assert!(!m.is_shown());
        assert!(!m.finish(), "already asked; not told twice");
        m.dismissed();
        assert!(!m.is_shown());
        // The next request is a fresh push, whatever was pushed before.
        assert!(m.request(RECT, ALL));
    }

    /// The race this struct exists for: finish, then a new long press before
    /// the old mode's report has come back through the queue. The report pays
    /// off the finish and must leave the new toolbar shown.
    #[test]
    fn a_late_report_for_a_finished_mode_does_not_take_down_the_next_one() {
        let mut m = ToolbarMirror::new();
        assert!(m.request(RECT, PASTE_ONLY));
        assert!(m.finish());
        assert!(m.request(RECT, ALL), "a request while finishing pushes");
        m.dismissed(); // the report for the mode finished above
        assert!(m.is_shown(), "the new toolbar is still up");
        m.dismissed(); // now the platform's own dismissal (Back, an item)
        assert!(!m.is_shown());
    }

    #[test]
    fn a_platform_dismissal_takes_the_mirror_down() {
        let mut m = ToolbarMirror::new();
        assert!(m.request(RECT, ALL));
        m.dismissed();
        assert!(!m.is_shown());
        assert!(!m.finish(), "and nothing is left to tell");
    }
}
