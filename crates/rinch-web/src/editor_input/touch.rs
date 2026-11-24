//! Touch: taps that focus and place the caret, and the compatibility
//! `mousedown` a serviced tap is followed by.

use super::capture::editor_owns_keyboard;
use super::context_menu::is_context_press;
use super::pointer::handle_mousedown;
use super::*;

/// True when a pointer event lands inside a mounted editor (`[data-pm-editor]`).
/// On `pointerdown` this is what makes the editor consume it in the capture phase
/// before the generic delegation (mirroring `handle_mousedown` returning `true` for an
/// in-editor click); on `pointerup` it gates the touch-tap focus below.
pub(super) fn pointer_targets_editor(event: &web_sys::PointerEvent) -> bool {
    event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| crate::event_delegation::editor_owning(&el))
        .is_some()
}

/// Whether a contact that went down at `start` and came up at `end` was a tap rather
/// than a scroll.
///
/// Defers to the same classifier the generic pointer delegation uses for its deferred
/// touch clicks (`drag_machine::pending_click_is_scroll`, radial within
/// `TOUCH_MOVE_SLOP`), so a gesture cannot be a tap for the editor and a scroll for the
/// rest of the page — which is what a separate per-axis threshold here would produce for
/// a diagonal drift.
pub(super) fn is_tap(start: (f32, f32), end: (f32, f32)) -> bool {
    !drag_machine::pending_click_is_scroll(end.0 - start.0, end.1 - start.1)
}

/// Record a touch/pen contact that went down inside an editor, so its release can be
/// classified as a tap. Mouse pointers are ignored — they get focus from `mousedown`.
pub(super) fn note_touch_down(event: &web_sys::PointerEvent) {
    if event.pointer_type() == "mouse" || !event.is_primary() {
        return;
    }
    TOUCH_TAP.with(|c| {
        c.set(Some((
            event.pointer_id(),
            event.client_x() as f32,
            event.client_y() as f32,
        )))
    });
}

/// Focus an editor from a touch/pen **tap**, placing the caret where it landed.
///
/// The mouse path cannot cover this. A tap's compatibility `mousedown` is synthesized
/// late, after `pointerup` — too late to be the *user gesture* iOS requires before it
/// will raise the on-screen keyboard for a programmatic `.focus()` — and on a page that
/// consumes the pointer sequence it may never be synthesized at all, leaving the editor
/// visibly focused but with no model focus, so every keystroke went nowhere. Running
/// the same handler here, from the real touch event, makes the tap self-sufficient. The
/// compatibility `mousedown` that may follow is then recognised as this tap's and runs
/// nothing ([`is_compat_mousedown_of_serviced_tap`], issue #302) — except a double
/// tap's, whose `detail == 2` still selects the word.
///
/// A contact that drifted past the shared touch slop was a scroll, not a tap, and is
/// dropped — panning a manuscript must not pop the keyboard.
pub(super) fn handle_touch_tap(event: &web_sys::PointerEvent, doc: &web_sys::Document) {
    // Read before clearing, and clear only for the contact we recorded: a second
    // finger's release must not discard the tap the first one is still making.
    let Some((pointer_id, x, y)) = TOUCH_TAP.with(|c| c.get()) else {
        return;
    };
    if event.pointer_id() != pointer_id {
        return;
    }
    TOUCH_TAP.with(|c| c.set(None));
    if !is_tap((x, y), (event.client_x() as f32, event.client_y() as f32)) {
        return;
    }
    if !pointer_targets_editor(event) {
        return;
    }
    // `PointerEvent` *is* a `MouseEvent`, so the click handler takes it as-is. Its
    // `detail` is 0 for touch, which lands on the plain place-the-caret arm; a
    // double-tap's word select still comes from the compatibility `mousedown`.
    SERVICED_TAP.with(|c| c.set(None));
    LINK_CLAIMED_IN.with(|c| c.set(None));
    let serviced = handle_mousedown(event.as_ref(), doc);
    // Drag-select follows `mousemove` with a button held — unreachable from touch — so
    // never leave a touch tap's anchor armed behind it.
    registry::end_drag(None);
    // Remember the tap, so the compatibility `mousedown` that follows it does not
    // run the press a second time (issue #302). Touch only: a pen's compatibility
    // `mousedown` comes *before* its `pointerup` (measured, Chrome 153), so a pen
    // tap's record would never be consumed and would swallow the next press there.
    if !serviced || event.pointer_type() != "touch" {
        return;
    }
    let link_claimed_in = LINK_CLAIMED_IN.with(|c| c.take());
    let tap = match (link_claimed_in, focused_editor()) {
        (Some(nid), _) => Some((nid, true)),
        (None, Some(nid)) => Some((nid, false)),
        (None, None) => None,
    };
    if let Some((container_nid, link_claimed)) = tap {
        SERVICED_TAP.with(|c| {
            c.set(Some(ServicedTap {
                x: event.client_x() as f32,
                y: event.client_y() as f32,
                at_ms: event.time_stamp(),
                container_nid,
                link_claimed,
            }))
        });
    }
}

/// A touch tap [`handle_touch_tap`] has already run the press for: where it came up,
/// when, and which editor it focused.
#[derive(Clone, Copy)]
pub(super) struct ServicedTap {
    x: f32,
    y: f32,
    /// The `pointerup`'s `timeStamp`, in the page's time origin (as every event's).
    at_ms: f64,
    container_nid: u64,
    /// The tap was a press on a link that the app's `on_link_click` claimed. Its
    /// compatibility `mousedown` is consumed whatever the focus state then: a claim
    /// commonly moves focus (a link popover's input), and running the press again
    /// would offer the link a second time.
    link_claimed: bool,
}

/// How long after a serviced tap its compatibility `mousedown` is still recognised
/// as that tap's. Browsers synthesize it right after `pointerup`, or after a
/// double-tap-to-zoom delay of roughly 300 ms on a page that allows zooming; a
/// `mousedown` later than this is treated as a press of its own. In Chrome 153 the
/// compatibility `mousedown` carries the `pointerup`'s own `timeStamp` (elapsed 0).
const TAP_COMPAT_WINDOW_MS: f64 = 800.0;

/// Whether `event` is the compatibility `mousedown` of a touch tap that
/// [`handle_touch_tap`] already serviced, and the press it would run has nothing
/// left to do (issue #302).
///
/// The tap's `pointerup` ran [`handle_mousedown`] in full: focus, the capture
/// field emptied and re-mirrored, the caret placed. Running it again from the
/// compatibility event redoes all of that — a second empty-and-refill of the field
/// under an attached soft keyboard, a second selection transaction, a second
/// `on_link_click` offer. So the first `mousedown` after a serviced tap **consumes**
/// the record, whatever it is, and answers `true` only when it is a single primary
/// press (`detail <= 1`, no modifier) at the tap's point, inside the window, with
/// that tap's editor still focused and the capture textarea still holding the
/// browser's focus — i.e. when the press would leave everything as it already is.
/// (For a tap an app's `on_link_click` claimed, the focus is not asked; see
/// [`ServicedTap::link_claimed`].) The record is also dropped by the tap's `click`,
/// which fires even when a page's `preventDefault` on `pointerdown` suppressed the
/// compatibility `mousedown`, so a tap that never gets one cannot swallow a later
/// press.
///
/// A double tap's compatibility `mousedown` carries `detail == 2` (the browser
/// counts clicks across taps) and is never matched, so its word select still runs.
pub(super) fn is_compat_mousedown_of_serviced_tap(
    event: &web_sys::MouseEvent,
    doc: &web_sys::Document,
) -> bool {
    if event.type_() != "mousedown" {
        return false;
    }
    let Some(tap) = SERVICED_TAP.with(|c| c.take()) else {
        return false;
    };
    let elapsed = event.time_stamp() - tap.at_ms;
    event.button() == 0
        && event.detail() <= 1
        && !event.shift_key()
        && !event.alt_key()
        && !is_context_press(event)
        && (0.0..=TAP_COMPAT_WINDOW_MS).contains(&elapsed)
        && is_tap(
            (tap.x, tap.y),
            (event.client_x() as f32, event.client_y() as f32),
        )
        && (tap.link_claimed
            || (focused_editor() == Some(tap.container_nid)
                // Not implied by the line above: `on_capture_blur` keeps the
                // focused editor across a *window* blur (#226), which leaves the
                // capture textarea inactive with the claim still standing — and
                // then the press has focus to restore.
                && editor_owns_keyboard(doc)))
}
