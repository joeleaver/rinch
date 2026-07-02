//! Global browser event delegation for the browser-native DOM backend.
//!
//! Installs document-level listeners (pointerdown/pointermove/pointerup/
//! pointercancel/keydown/input) that delegate to rinch's event-handler registry,
//! drag system, and render-surface focus routing. Using Pointer Events (instead
//! of raw mouse events) means the same code path covers mouse, touch, and pen —
//! so the element drag-and-drop suite works on touch devices, where no synthetic
//! mouse-move stream arrives during a finger drag. Pointer events resolve
//! text-hit positions via `caretRangeFromPoint` so contenteditable apps get
//! accurate caret placement (a no-op for apps without `data-block-index` blocks).

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_core::events;

use crate::web_document::WebDocument;

thread_local! {
    /// The element currently under the pointer that carries a `data-onenter`/
    /// `data-onleave` handler. Used to implement non-bubbling
    /// `mouseenter`/`mouseleave` semantics: enter/leave fire exactly once when
    /// the resolved handler element under the pointer changes (moving between
    /// descendants of the same handler element fires nothing). This is the
    /// correct hover model; note it does not byte-for-byte match the desktop
    /// backend, which keys on the raw hit-test node and may re-fire.
    static LAST_HOVER: std::cell::RefCell<Option<web_sys::Element>> =
        const { std::cell::RefCell::new(None) };
}

/// Map a browser `MouseEvent.button` index onto the core button enum.
fn mouse_button_from_event(event: &web_sys::MouseEvent) -> events::MouseButton {
    match event.button() {
        1 => events::MouseButton::Middle,
        2 => events::MouseButton::Right,
        _ => events::MouseButton::Left,
    }
}

/// Read the keyboard modifier state carried by a browser mouse event.
fn modifiers_from_event(event: &web_sys::MouseEvent) -> events::ModifierState {
    events::ModifierState {
        shift: event.shift_key(),
        ctrl: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

/// Current visual viewport size in CSS (logical) pixels, for the ClickContext's
/// `viewport_width`/`viewport_height` — consumed by flip/placement logic in
/// `Select`/`DropdownMenu`/`ContextMenu`. Mirrors the desktop backend, which
/// fills these on the click/mouse/contextmenu paths (but not the drag paths).
fn viewport_dims() -> (f32, f32) {
    let win = web_sys::window();
    let w = win
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    let h = win
        .as_ref()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;
    (w, h)
}

/// Compare two elements by DOM node identity.
fn same_element(a: &web_sys::Element, b: &web_sys::Element) -> bool {
    let a_node: &web_sys::Node = a.as_ref();
    let b_node: &web_sys::Node = b.as_ref();
    a_node.is_same_node(Some(b_node))
}

/// Walk up from `el` to the nearest ancestor carrying `attr`, set the
/// [`events::ClickContext`] (cursor + element bounds + button + modifiers from
/// `event`), and dispatch the registered handler. Mirrors the `data-rid`
/// pattern; used for `data-onmousedown`/`data-onmouseup`/`data-onmousemove`.
fn dispatch_mouse_attr(el: &web_sys::Element, attr: &str, event: &web_sys::MouseEvent) {
    let selector = format!("[{attr}]");
    if let Ok(Some(target_el)) = el.closest(&selector)
        && let Some(id_str) = target_el.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        let rect = target_el.get_bounding_client_rect();
        let (vp_w, vp_h) = viewport_dims();
        events::set_click_context(events::ClickContext {
            mouse_x: event.client_x() as f32,
            mouse_y: event.client_y() as f32,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit: Default::default(),
            viewport_width: vp_w,
            viewport_height: vp_h,
            button: mouse_button_from_event(event),
            modifiers: modifiers_from_event(event),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

// ── Element-to-element drag-and-drop (the data-ondrag* attribute suite) ───────
//
// Synthesized from Pointer Events to mirror the desktop backend's DOM drag system
// (NOT the browser's native HTML5 drag events). A `draggable="true"` element
// under pointerdown becomes a *pending* source; once it *activates* the drag
// fires data-ondragstart. While active, each move tracks the [data-ondrop] target
// under the cursor (data-ondragenter/leave), fires data-ondragover on it and
// data-ondragmove on the source; pointerup fires data-ondrop + data-ondragend
// (Escape / pointercancel cancels). Typed payloads flow through the
// backend-agnostic `DragContext<T>` in the app's handlers — nothing here.
//
// Unlike desktop there is no drag ghost (the browser owns paint); apps render
// their own visual feedback by positioning an element from data-ondragmove.
//
// Activation policy (differs by pointer kind, so a mouse stays snappy while a
// touch does not hijack scrolling):
//   * **Mouse:** activates immediately once the pointer moves past
//     `WEB_DRAG_THRESHOLD` (5 CSS px) — unchanged from the old mouse-only code.
//   * **Touch / pen:** activates on a short *long-press* hold
//     (`TOUCH_LONG_PRESS_MS`) while the contact stays within `TOUCH_MOVE_SLOP`.
//     If the contact moves past the slop *before* the hold completes, the gesture
//     is a scroll/pan — the pending drag is abandoned and the browser scrolls
//     normally. This is why touch does NOT use the bare movement threshold: doing
//     so would turn every swipe over a list of draggable rows into a drag.
// Once a touch/pen drag activates it grabs pointer capture and suppresses
// scrolling (preventDefault + `touch-action: none` on the source) so tracking is
// reliable even if the finger leaves the source element.

/// The pure activation state machine, kept free of `web_sys` so it can be
/// unit-tested on the host target (`cargo test --workspace`).
mod drag_machine {
    /// Movement threshold (CSS px) before a *mouse* drag activates — matches desktop.
    pub const WEB_DRAG_THRESHOLD: f32 = 5.0;
    /// Hold duration (ms) before a *touch/pen* drag activates via long-press.
    pub const TOUCH_LONG_PRESS_MS: i32 = 350;
    /// Movement (CSS px) a touch/pen contact may drift during the long-press hold
    /// before the gesture is reclassified as a scroll and the drag abandoned.
    pub const TOUCH_MOVE_SLOP: f32 = 10.0;

    /// Which physical input started the drag. Drives the activation policy.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum PointerKind {
        Mouse,
        Pen,
        Touch,
    }

    impl PointerKind {
        /// Map a `PointerEvent.pointerType` string onto the enum.
        pub fn from_type(pointer_type: &str) -> Self {
            match pointer_type {
                "touch" => Self::Touch,
                "pen" => Self::Pen,
                // "mouse" and anything unrecognized behave like a mouse.
                _ => Self::Mouse,
            }
        }

        /// Touch and pen arm via a long-press hold; a mouse uses the movement
        /// threshold and activates immediately past it.
        pub fn uses_long_press(self) -> bool {
            matches!(self, Self::Touch | Self::Pen)
        }
    }

    /// Decision for a pointermove that arrives while the drag is still *pending*
    /// (activation not yet reached).
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum PendingMove {
        /// Stay pending; do nothing.
        Ignore,
        /// Cross the movement threshold → activate now (mouse only).
        Activate,
        /// A touch/pen contact moved during the hold — treat as a scroll/pan and
        /// abandon the pending drag so the browser scrolls.
        AbortScroll,
    }

    /// Classify a pending-state pointer move given the travel from the start point.
    pub fn pending_move_decision(kind: PointerKind, dx: f32, dy: f32) -> PendingMove {
        let dist_sq = dx * dx + dy * dy;
        if kind.uses_long_press() {
            // Long-press pointers only ever activate from the timer. Any real
            // movement before it fires means the user is scrolling.
            if dist_sq > TOUCH_MOVE_SLOP * TOUCH_MOVE_SLOP {
                PendingMove::AbortScroll
            } else {
                PendingMove::Ignore
            }
        } else if dist_sq >= WEB_DRAG_THRESHOLD * WEB_DRAG_THRESHOLD {
            PendingMove::Activate
        } else {
            PendingMove::Ignore
        }
    }

    /// True when the primary button/contact is no longer down (`buttons & 1 == 0`),
    /// i.e. a pointerup/pointercancel was missed and the drag should self-heal.
    pub fn primary_released(buttons: u16) -> bool {
        (buttons & 1) == 0
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn pointer_kind_from_type() {
            assert_eq!(PointerKind::from_type("touch"), PointerKind::Touch);
            assert_eq!(PointerKind::from_type("pen"), PointerKind::Pen);
            assert_eq!(PointerKind::from_type("mouse"), PointerKind::Mouse);
            // Unknown/empty pointerType falls back to mouse semantics.
            assert_eq!(PointerKind::from_type(""), PointerKind::Mouse);
            assert_eq!(PointerKind::from_type("stylus?"), PointerKind::Mouse);
        }

        #[test]
        fn only_touch_and_pen_use_long_press() {
            assert!(!PointerKind::Mouse.uses_long_press());
            assert!(PointerKind::Touch.uses_long_press());
            assert!(PointerKind::Pen.uses_long_press());
        }

        #[test]
        fn mouse_activates_past_threshold_only() {
            // Below threshold in both axes → ignore.
            assert_eq!(
                pending_move_decision(PointerKind::Mouse, 3.0, 3.0),
                PendingMove::Ignore
            );
            // Exactly at threshold on one axis → activate (>=).
            assert_eq!(
                pending_move_decision(PointerKind::Mouse, WEB_DRAG_THRESHOLD, 0.0),
                PendingMove::Activate
            );
            // Well past → activate; sign of travel is irrelevant.
            assert_eq!(
                pending_move_decision(PointerKind::Mouse, -20.0, 0.0),
                PendingMove::Activate
            );
            // A mouse never aborts to scroll.
            assert_ne!(
                pending_move_decision(PointerKind::Mouse, 100.0, 100.0),
                PendingMove::AbortScroll
            );
        }

        #[test]
        fn touch_ignores_small_drift_but_aborts_a_swipe() {
            // Small drift within slop while holding → keep waiting for the timer.
            assert_eq!(
                pending_move_decision(PointerKind::Touch, 4.0, 4.0),
                PendingMove::Ignore
            );
            // Past the slop → this is a scroll, not a drag.
            assert_eq!(
                pending_move_decision(PointerKind::Touch, TOUCH_MOVE_SLOP + 1.0, 0.0),
                PendingMove::AbortScroll
            );
            // A touch move NEVER activates directly (only the long-press timer does).
            assert_ne!(
                pending_move_decision(PointerKind::Touch, 3.0, 0.0),
                PendingMove::Activate
            );
            assert_ne!(
                pending_move_decision(PointerKind::Touch, 500.0, 500.0),
                PendingMove::Activate
            );
        }

        #[test]
        fn pen_uses_the_same_long_press_policy_as_touch() {
            assert_eq!(
                pending_move_decision(PointerKind::Pen, 2.0, 2.0),
                PendingMove::Ignore
            );
            assert_eq!(
                pending_move_decision(PointerKind::Pen, 40.0, 0.0),
                PendingMove::AbortScroll
            );
        }

        #[test]
        fn primary_released_reads_the_low_bit() {
            // Primary (left / first contact) held.
            assert!(!primary_released(0b0001));
            assert!(!primary_released(0b0011));
            // Primary not held (e.g. only the right button, or nothing).
            assert!(primary_released(0b0000));
            assert!(primary_released(0b0010));
        }
    }
}

use drag_machine::{PendingMove, PointerKind, pending_move_decision};

struct WebDragState {
    /// The `draggable="true"` source element.
    source: web_sys::Element,
    /// The `PointerEvent.pointerId` that owns this drag. Only events carrying this
    /// id drive it (the multi-touch guard — secondary contacts are ignored).
    pointer_id: i32,
    /// Which physical input started the drag (activation policy + capture).
    kind: PointerKind,
    /// Pointer position at pointerdown (for the activation threshold / slop).
    start_x: f32,
    start_y: f32,
    /// Last known pointer position (used by Escape-cancel and long-press activate).
    cursor: (f32, f32),
    /// Whether the drag has activated (past threshold / long-press).
    active: bool,
    /// The current [data-ondrop] target under the pointer.
    over_target: Option<web_sys::Element>,
    /// Whether we hold pointer capture on the source (touch/pen only).
    captured: bool,
    /// `setTimeout` handle for the long-press timer (touch/pen), while pending.
    long_press_timer: Option<i32>,
    /// Keeps the long-press timer callback alive until it fires or is cleared.
    _long_press_closure: Option<Closure<dyn FnMut()>>,
    /// The source's inline `touch-action` before we forced it to `none`, restored
    /// on drag end/cancel (touch/pen only).
    saved_touch_action: Option<String>,
}

thread_local! {
    static WEB_DRAG: std::cell::RefCell<Option<WebDragState>> =
        const { std::cell::RefCell::new(None) };

    /// Pointer capture taken for an active pointer-capture `Drag` (slider, panel,
    /// resize handle). Held as `(element, pointer_id)` so touch/pen moves keep
    /// tracking on the element (and the pointermove handler can `preventDefault`
    /// the scroll) instead of the browser panning the page. Released on
    /// pointerup/pointercancel. Independent of `WEB_DRAG` (the element-DnD suite).
    static DRAG_POINTER_CAPTURE: std::cell::RefCell<Option<(web_sys::Element, i32)>> =
        const { std::cell::RefCell::new(None) };
}

/// Release any pointer capture taken for an active `Drag` (slider/panel/resize).
/// Idempotent; safe to call when none was taken.
fn release_drag_pointer_capture() {
    DRAG_POINTER_CAPTURE.with(|c| {
        if let Some((el, pointer_id)) = c.borrow_mut().take() {
            let _ = el.release_pointer_capture(pointer_id);
        }
    });
}

/// Clear a pending long-press timer (if any) on the given state, dropping the
/// closure so it never fires. Safe to call when no timer is armed.
fn clear_long_press_timer(state: &mut WebDragState) {
    if let Some(handle) = state.long_press_timer.take()
        && let Some(win) = web_sys::window()
    {
        win.clear_timeout_with_handle(handle);
    }
    state._long_press_closure = None;
}

/// Release pointer capture and restore the source's `touch-action`. Undoes the
/// scroll-suppression applied at activation. Idempotent.
fn release_capture_and_scroll(state: &mut WebDragState) {
    if state.captured {
        let _ = state.source.release_pointer_capture(state.pointer_id);
        state.captured = false;
    }
    if let Some(prev) = state.saved_touch_action.take()
        && let Some(html) = state.source.dyn_ref::<web_sys::HtmlElement>()
    {
        let style = html.style();
        if prev.is_empty() {
            let _ = style.remove_property("touch-action");
        } else {
            let _ = style.set_property("touch-action", &prev);
        }
    }
}

/// Walk up from `el` for the nearest ancestor with `attr` and dispatch its
/// handler with no ClickContext (data-ondragstart/enter/leave/drop).
fn dispatch_plain_attr(el: &web_sys::Element, attr: &str) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Dispatch `attr` with a ClickContext set to the cursor position and the
/// resolved element's bounds (data-ondragover).
fn dispatch_drag_with_bounds(el: &web_sys::Element, attr: &str, x: f32, y: f32) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        let rect = t.get_bounding_client_rect();
        events::set_click_context(events::ClickContext {
            mouse_x: x,
            mouse_y: y,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit: Default::default(),
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: events::MouseButton::Left,
            modifiers: events::ModifierState::default(),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Dispatch `attr` with a ClickContext set to the cursor position and zero
/// element bounds (data-ondragmove / data-ondragend), matching desktop.
fn dispatch_drag_cursor_only(el: &web_sys::Element, attr: &str, x: f32, y: f32) {
    let selector = format!("[{attr}]");
    if let Ok(Some(t)) = el.closest(&selector)
        && let Some(id_str) = t.get_attribute(attr)
        && let Ok(id) = id_str.parse::<usize>()
    {
        events::set_click_context(events::ClickContext {
            mouse_x: x,
            mouse_y: y,
            element_x: 0.0,
            element_y: 0.0,
            element_width: 0.0,
            element_height: 0.0,
            text_hit: Default::default(),
            viewport_width: 0.0,
            viewport_height: 0.0,
            button: events::MouseButton::Left,
            modifiers: events::ModifierState::default(),
        });
        events::dispatch_event(events::EventHandlerId(id));
    }
}

/// Resolve the topmost element under `(x, y)` for an *active* drag, then track
/// the [data-ondrop] target: fire data-ondragenter/leave when it changes,
/// data-ondragover on the target, and data-ondragmove on the source. `hit` is the
/// topmost element under the cursor — for a captured (touch/pen) drag this must
/// come from `elementFromPoint` (pointer capture retargets the raw event to the
/// source), for a mouse drag it is the raw `event.target()`. All WEB_DRAG borrows
/// are released before dispatching handlers.
fn process_drag_target(source: &web_sys::Element, hit: Option<web_sys::Element>, x: f32, y: f32) {
    let old_target = WEB_DRAG.with(|d| d.borrow().as_ref().and_then(|s| s.over_target.clone()));

    let new_target = hit.and_then(|el| el.closest("[data-ondrop]").ok().flatten());

    let changed = match (&old_target, &new_target) {
        (Some(o), Some(n)) => !same_element(o, n),
        (None, None) => false,
        _ => true,
    };
    if changed {
        if let Some(o) = &old_target {
            dispatch_plain_attr(o, "data-ondragleave");
        }
        if let Some(n) = &new_target {
            dispatch_plain_attr(n, "data-ondragenter");
        }
        WEB_DRAG.with(|d| {
            if let Some(s) = d.borrow_mut().as_mut() {
                s.over_target = new_target.clone();
            }
        });
    }

    if let Some(t) = &new_target {
        dispatch_drag_with_bounds(t, "data-ondragover", x, y);
    }
    dispatch_drag_cursor_only(source, "data-ondragmove", x, y);
}

/// Topmost element under `(x, y)` via `document.elementFromPoint`. Used to resolve
/// the drop target for captured (touch/pen) drags, where the raw pointer event is
/// retargeted to the capturing source element.
fn element_from_point(x: f32, y: f32) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.element_from_point(x, y)
}

/// The element under the pointer for `data-onmouse*` dispatch. While a captured
/// (touch/pen) element drag owns this pointer, the raw `event.target()` is the
/// capturing source, so resolve the real element under the finger via
/// `elementFromPoint`; otherwise use the raw target (mouse — unchanged).
fn pointer_hit_element(event: &web_sys::PointerEvent) -> Option<web_sys::Element> {
    let captured = WEB_DRAG.with(|d| {
        d.borrow()
            .as_ref()
            .is_some_and(|s| s.captured && s.pointer_id == event.pointer_id())
    });
    if captured {
        element_from_point(event.client_x() as f32, event.client_y() as f32)
    } else {
        event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    }
}

/// Transition the current *pending* drag to *active*: fire data-ondragstart, and
/// for touch/pen grab pointer capture + suppress scrolling on the source, then
/// process the drop target under the current cursor. No-op if there is no pending
/// drag or it is already active. Called from the movement-threshold path (mouse)
/// and from the long-press timer (touch/pen).
fn activate_web_drag() {
    let Some((source, kind, pointer_id, cursor)) = WEB_DRAG.with(|d| {
        let mut b = d.borrow_mut();
        let s = b.as_mut()?;
        if s.active {
            return None;
        }
        s.active = true;
        clear_long_press_timer(s);
        Some((s.source.clone(), s.kind, s.pointer_id, s.cursor))
    }) else {
        return;
    };

    // Touch/pen: capture the pointer and stop the browser from scrolling/panning
    // for the rest of the gesture. Scoped to the source element, so nothing else
    // on the page loses its scrollability.
    if kind.uses_long_press() {
        let captured = source.set_pointer_capture(pointer_id).is_ok();
        let saved_touch_action = source.dyn_ref::<web_sys::HtmlElement>().map(|html| {
            let style = html.style();
            let prev = style.get_property_value("touch-action").unwrap_or_default();
            let _ = style.set_property("touch-action", "none");
            prev
        });
        WEB_DRAG.with(|d| {
            if let Some(s) = d.borrow_mut().as_mut() {
                s.captured = captured;
                s.saved_touch_action = saved_touch_action;
            }
        });
    }

    dispatch_plain_attr(&source, "data-ondragstart");
    // Process the target at the current cursor on this same frame so a coarse
    // drag (few moves) still fires enter/over and can drop. (Desktop splits
    // dragstart and the first target frame to render a drag ghost first; web has
    // no ghost, so doing both here is fine.)
    let hit = element_from_point(cursor.0, cursor.1);
    process_drag_target(&source, hit, cursor.0, cursor.1);
}

/// Advance an in-progress element drag on pointer move. Only the pointer that
/// owns the drag drives it (multi-touch guard). Handles the self-heal for a
/// missed release, the pending→active transition (mouse threshold; touch/pen
/// abandons to scroll on real movement, activation is timer-driven), and target
/// tracking while active. Returns whether the browser default should be
/// suppressed (true while a captured touch/pen drag is active, to block scroll).
fn handle_web_pointer_move(event: &web_sys::PointerEvent) -> bool {
    let x = event.client_x() as f32;
    let y = event.client_y() as f32;

    // Snapshot; ignore events from a non-owning pointer (secondary touch). The
    // source Element is fetched lazily in the active branch below, so a jittery
    // long-press hold (many Ignore moves) and secondary-finger moves don't clone
    // it each frame.
    let Some((kind, was_active, captured, start, owns)) = WEB_DRAG.with(|d| {
        d.borrow().as_ref().map(|s| {
            (
                s.kind,
                s.active,
                s.captured,
                (s.start_x, s.start_y),
                s.pointer_id == event.pointer_id(),
            )
        })
    }) else {
        return false;
    };
    if !owns {
        return false;
    }

    // Self-heal: primary contact no longer down means a release was missed (e.g.
    // outside the window) — cancel rather than leave the drag stuck.
    if drag_machine::primary_released(event.buttons()) {
        cancel_web_drag();
        return false;
    }

    // Record the latest cursor (used by Escape-cancel and long-press activate).
    WEB_DRAG.with(|d| {
        if let Some(s) = d.borrow_mut().as_mut() {
            s.cursor = (x, y);
        }
    });

    if !was_active {
        match pending_move_decision(kind, x - start.0, y - start.1) {
            PendingMove::Ignore => return false,
            PendingMove::AbortScroll => {
                // Touch/pen swipe before the long-press fired → it's a scroll.
                cancel_web_drag();
                return false;
            }
            PendingMove::Activate => {
                // Mouse crossed the threshold; activate and process this frame.
                activate_web_drag();
                return false;
            }
        }
    }

    // Active: resolve the target. A captured (touch/pen) drag retargets the raw
    // event to the source, so use elementFromPoint; a mouse drag uses the raw hit.
    let hit = if captured {
        element_from_point(x, y)
    } else {
        event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    };
    let Some(source) = WEB_DRAG.with(|d| d.borrow().as_ref().map(|s| s.source.clone())) else {
        return false;
    };
    process_drag_target(&source, hit, x, y);

    // While a captured touch/pen drag is live, block the browser from scrolling.
    captured
}

/// Cancel an in-progress element drag (Escape, pointercancel, or a release missed
/// outside the window). An *active* drag fires data-ondragleave on the target and
/// data-ondragend on the source (using the last known cursor); a merely *pending*
/// drag is just cleared. Always releases pointer capture, clears the long-press
/// timer, and restores `touch-action`. Returns true if any drag — pending or
/// active — was in progress, so the caller can consume the key, matching the
/// desktop backend (which swallows Escape for both pending and active drags).
fn cancel_web_drag() -> bool {
    let Some(mut state) = WEB_DRAG.with(|d| d.borrow_mut().take()) else {
        return false;
    };
    clear_long_press_timer(&mut state);
    release_capture_and_scroll(&mut state);
    if state.active {
        let (x, y) = state.cursor;
        finish_active_drag(&state, x, y, false);
    }
    true
}

/// Fire the terminal events for an *active* drag at `(x, y)`: `data-ondrop` (only
/// when `do_drop`) then `data-ondragleave` on the current target, then
/// `data-ondragend` on the source. Shared by the pointerup (drop) and
/// cancel/Escape (no drop) paths so the teardown stays in one place.
fn finish_active_drag(state: &WebDragState, x: f32, y: f32, do_drop: bool) {
    if let Some(t) = &state.over_target {
        if do_drop {
            dispatch_plain_attr(t, "data-ondrop");
        }
        dispatch_plain_attr(t, "data-ondragleave");
    }
    dispatch_drag_cursor_only(&state.source, "data-ondragend", x, y);
}

/// Convert a UTF-16 code unit offset within a string to a UTF-8 byte offset.
fn utf16_offset_to_utf8_bytes(text: &str, utf16_offset: u32) -> usize {
    let mut utf16_count = 0u32;
    for (byte_idx, ch) in text.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        utf16_count += ch.len_utf16() as u32;
    }
    text.len() // offset is at or past the end
}

/// Use `document.caretRangeFromPoint` to resolve a click position to a text hit.
/// Returns `Some(TextHitInfo)` if the click resolved to a text position inside a block.
fn resolve_text_hit(
    browser_doc: &web_sys::Document,
    client_x: f32,
    client_y: f32,
) -> Option<events::TextHitInfo> {
    // caretRangeFromPoint is non-standard but available in Chrome/Safari/Edge
    let func = js_sys::Reflect::get(browser_doc, &"caretRangeFromPoint".into()).ok()?;
    let func: js_sys::Function = func.dyn_into().ok()?;
    let range_val = func
        .call2(
            browser_doc,
            &JsValue::from(client_x),
            &JsValue::from(client_y),
        )
        .ok()?;
    if range_val.is_null() || range_val.is_undefined() {
        return None;
    }
    let range: web_sys::Range = range_val.dyn_into().ok()?;
    let start_container = range.start_container().ok()?;
    let start_offset = range.start_offset().ok()?;

    // Walk up from start_container to find nearest ancestor with data-block-index
    let mut current: Option<web_sys::Node> = Some(start_container.clone());
    let mut block_el: Option<web_sys::Element> = None;
    while let Some(node) = current {
        if let Ok(el) = node.clone().dyn_into::<web_sys::Element>()
            && el.has_attribute("data-block-index")
        {
            block_el = Some(el);
            break;
        }
        current = node.parent_node();
    }
    let block_el = block_el?;
    let block_index: usize = block_el.get_attribute("data-block-index")?.parse().ok()?;

    // Compute byte_offset: walk all text nodes in the block before start_container,
    // summing their UTF-8 byte lengths. For start_container, convert start_offset
    // (UTF-16) to UTF-8 bytes.
    let byte_offset = compute_byte_offset_in_block(&block_el, &start_container, start_offset);

    Some(events::TextHitInfo {
        block_index,
        byte_offset,
        inline_root_node_id: 0,
        valid: true,
    })
}

/// Compute the UTF-8 byte offset within a block element by walking text nodes.
pub(crate) fn compute_byte_offset_in_block(
    block_el: &web_sys::Element,
    target_text_node: &web_sys::Node,
    utf16_offset_in_target: u32,
) -> usize {
    let mut byte_offset = 0usize;
    walk_text_nodes_for_offset(
        &block_el.clone().into(),
        target_text_node,
        utf16_offset_in_target,
        &mut byte_offset,
    );
    byte_offset
}

/// Depth-first walk of text nodes. Accumulates UTF-8 byte lengths.
/// When we reach `target_text_node`, adds the UTF-8 equivalent of `utf16_offset_in_target`
/// and returns true (found).
fn walk_text_nodes_for_offset(
    node: &web_sys::Node,
    target: &web_sys::Node,
    utf16_offset: u32,
    byte_offset: &mut usize,
) -> bool {
    if node.node_type() == web_sys::Node::TEXT_NODE {
        if node == target {
            let text = node.text_content().unwrap_or_default();
            *byte_offset += utf16_offset_to_utf8_bytes(&text, utf16_offset);
            return true;
        }
        let text = node.text_content().unwrap_or_default();
        *byte_offset += text.len();
        return false;
    }
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && walk_text_nodes_for_offset(&child, target, utf16_offset, byte_offset)
        {
            return true;
        }
    }
    false
}

/// Dispatch the click (`data-rid`) handler nearest to `el`, with a full
/// [`events::ClickContext`] (cursor, element bounds, text-hit, button,
/// modifiers) and `prevent_default`. Used for the mousedown click and for the
/// deferred click when a draggable's drag never activates, so both paths get
/// identical click semantics.
fn dispatch_click_at(
    el: &web_sys::Element,
    browser_doc: &web_sys::Document,
    event: &web_sys::MouseEvent,
) {
    if let Ok(Some(rid_el)) = el.closest("[data-rid]")
        && let Some(rid_str) = rid_el.get_attribute("data-rid")
        && let Ok(rid) = rid_str.parse::<usize>()
    {
        let rect = rid_el.get_bounding_client_rect();
        let (vp_w, vp_h) = viewport_dims();
        let text_hit = resolve_text_hit(
            browser_doc,
            event.client_x() as f32,
            event.client_y() as f32,
        )
        .unwrap_or_default();
        events::set_click_context(events::ClickContext {
            mouse_x: event.client_x() as f32,
            mouse_y: event.client_y() as f32,
            element_x: rect.x() as f32,
            element_y: rect.y() as f32,
            element_width: rect.width() as f32,
            element_height: rect.height() as f32,
            text_hit,
            viewport_width: vp_w,
            viewport_height: vp_h,
            button: mouse_button_from_event(event),
            modifiers: modifiers_from_event(event),
        });

        // Ancestor chain (immediate parent → root, absolute viewport bounds) so
        // handlers can use find_click_ancestor()/click_ancestors() — matches the
        // desktop backend. getBoundingClientRect already returns absolute logical
        // px, so no manual offset accumulation is needed.
        let mut ancestors = Vec::new();
        let mut parent = rid_el.parent_element();
        while let Some(p) = parent {
            let p_rect = p.get_bounding_client_rect();
            ancestors.push(events::AncestorBounds {
                tag: p.tag_name().to_lowercase(),
                id: p.id(),
                class: p.class_name(),
                x: p_rect.x() as f32,
                y: p_rect.y() as f32,
                width: p_rect.width() as f32,
                height: p_rect.height() as f32,
            });
            parent = p.parent_element();
        }
        events::set_click_ancestors(ancestors);

        // Prevent browser default behavior (e.g. text selection during slider
        // drag, <label> synthesizing extra events).
        event.prevent_default();
        events::dispatch_event(events::EventHandlerId(rid));
    }
}

/// Set up global event listeners that delegate to rinch's event handler registry.
///
/// Wires `pointerdown`/`pointermove`/`pointerup`/`pointercancel` (click dispatch,
/// `data-onmousedown`/`up`/`move`, slider drag tracking, text-hit resolution,
/// hover `data-onenter`/`data-onleave`, and the element drag-and-drop
/// `data-ondrag*` suite — which now works on touch/pen as well as mouse),
/// `keydown` (render-surface routing, keyboard interceptor, `data-onsubmit` on
/// Enter, Escape drag-cancel), `input` (`data-oninput`), `contextmenu`
/// (`data-oncontextmenu`), and capture-phase `scroll` (`data-onscroll`).
///
/// Pointer Events are used (rather than raw mouse events) so a single code path
/// covers mouse, touch, and pen. `web_sys::PointerEvent` derefs to `MouseEvent`,
/// so `client_x/y`, `button`, `buttons`, and the modifier getters keep working;
/// the mouse listeners are *replaced* (not supplemented) to avoid the double
/// dispatch that would occur from the browser's compatibility mouse events.
///
/// Listeners are leaked via `Closure::forget` and live for the lifetime of the
/// page.
pub fn setup_event_delegation(doc: &WebDocument) {
    let browser_doc = doc.browser_document().clone();
    let browser_doc_for_click = browser_doc.clone();

    // Pointerdown delegation: find nearest [data-rid] ancestor and dispatch.
    // We use pointerdown (not click) so drag operations (sliders, element DnD)
    // can begin tracking movement immediately, on mouse and touch alike.
    let pointerdown_closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
        // A drag is owned by a single pointer. A *primary* pointerdown means no
        // primary contact is currently down, so any drag still recorded is stale
        // — its pointerup/pointercancel was missed (e.g. a dropped mobile
        // system-gesture interruption, or the source removed mid-drag). Clear it
        // and proceed, so a stuck drag can't wedge all pointer input (the guarded
        // pointerdown + owns-gated move/up would otherwise leave no recovery path
        // on a touch device). A *secondary* (non-primary) pointerdown is a
        // concurrent finger during a live drag: ignore it so it neither starts a
        // second DnD nor disturbs the owning one.
        if WEB_DRAG.with(|d| d.borrow().is_some()) {
            if event.is_primary() {
                cancel_web_drag();
            } else {
                return;
            }
        }

        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
        {
            // Clear render surface focus if click is outside any surface
            if rinch::render_surface::focused_surface_id().is_some()
                && el.closest("[data-render-surface]").ok().flatten().is_none()
            {
                rinch::render_surface::set_focused_surface(None);
            }

            // Additive: fire data-onmousedown before the click dispatch below,
            // matching DOM order (down precedes click) and the desktop arm.
            dispatch_mouse_attr(&el, "data-onmousedown", &event);

            // Element drag-and-drop: a pointerdown on a `draggable="true"` element
            // starts a pending drag (primary pointer only). Its click is deferred
            // to pointerup (and fires only if the drag never activates), matching
            // the desktop backend. A mouse arms on the movement threshold; a
            // touch/pen arms on a long-press hold (see the drag_machine module).
            let drag_source = el.closest("[draggable=\"true\"]").ok().flatten();
            if let Some(source) = &drag_source
                && event.is_primary()
            {
                let kind = PointerKind::from_type(&event.pointer_type());
                WEB_DRAG.with(|d| {
                    *d.borrow_mut() = Some(WebDragState {
                        source: source.clone(),
                        pointer_id: event.pointer_id(),
                        kind,
                        start_x: event.client_x() as f32,
                        start_y: event.client_y() as f32,
                        cursor: (event.client_x() as f32, event.client_y() as f32),
                        active: false,
                        over_target: None,
                        captured: false,
                        long_press_timer: None,
                        _long_press_closure: None,
                        saved_touch_action: None,
                    });
                });

                // Touch/pen: arm the long-press timer. A stationary hold past
                // TOUCH_LONG_PRESS_MS activates the drag; movement before then
                // abandons it to scroll (handled in the move path). The timer is a
                // no-op if the drag was already cancelled or activated by the time
                // it fires (activate_web_drag guards on the current state).
                if kind.uses_long_press()
                    && let Some(win) = web_sys::window()
                {
                    let timer_cb = Closure::wrap(Box::new(activate_web_drag) as Box<dyn FnMut()>);
                    if let Ok(handle) = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        timer_cb.as_ref().unchecked_ref(),
                        drag_machine::TOUCH_LONG_PRESS_MS,
                    ) {
                        WEB_DRAG.with(|d| {
                            if let Some(s) = d.borrow_mut().as_mut() {
                                s.long_press_timer = Some(handle);
                                s._long_press_closure = Some(timer_cb);
                            }
                        });
                    }
                }
            }

            // Click dispatch (data-rid fires on pointerdown). The primary (left)
            // button always clicks; a non-primary button clicks only when there
            // is no contextmenu handler in the ancestry — mirroring the desktop
            // right-click gate where oncontextmenu suppresses the click. Draggable
            // sources defer their click to pointerup (above).
            let click_allowed = drag_source.is_none()
                && (event.button() == 0
                    || el.closest("[data-oncontextmenu]").ok().flatten().is_none());

            // Dispatch the click for the nearest [data-rid] (draggables defer to
            // pointerup above).
            if click_allowed {
                dispatch_click_at(&el, &browser_doc_for_click, &event);

                // If that click armed a pointer-capture `Drag` (slider, panel,
                // resize handle — started from the handler via `Drag::start()`),
                // capture the pointer on the element so touch/pen moves keep
                // tracking on it and the pointermove handler's `preventDefault`
                // can stop the page from scrolling out from under the drag.
                // Released on pointerup/pointercancel.
                if rinch_core::Drag::is_active() {
                    let _ = el.set_pointer_capture(event.pointer_id());
                    DRAG_POINTER_CAPTURE
                        .with(|c| *c.borrow_mut() = Some((el.clone(), event.pointer_id())));
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback(
            "pointerdown",
            pointerdown_closure.as_ref().unchecked_ref(),
        )
        .unwrap();
    pointerdown_closure.forget();

    // Pointermove delegation: feed active drag operations.
    let pointermove_closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
        let (drag_active, _) =
            rinch_core::update_drag(event.client_x() as f32, event.client_y() as f32);
        if drag_active {
            event.prevent_default();
        }

        if let Some(el) = pointer_hit_element(&event) {
            dispatch_mouse_attr(&el, "data-onmousemove", &event);

            // Element drag-and-drop: if a drag is pending/active, advance it and
            // skip hover — an active drag owns the move (mirrors desktop). A live
            // captured touch/pen drag asks us to block the browser's scroll.
            if WEB_DRAG.with(|d| d.borrow().is_some()) {
                if handle_web_pointer_move(&event) {
                    event.prevent_default();
                }
                return;
            }

            // Hover: fire data-onleave on the old element and data-onenter on the
            // new one whenever the resolved hover element changes.
            let new_hover = el.closest("[data-onenter],[data-onleave]").ok().flatten();
            LAST_HOVER.with(|lh| {
                let changed = {
                    let cur = lh.borrow();
                    match (cur.as_ref(), new_hover.as_ref()) {
                        (Some(old), Some(new)) => !same_element(old, new),
                        (None, None) => false,
                        _ => true,
                    }
                };
                if !changed {
                    return;
                }
                // Drop all borrows before dispatching (handlers may re-enter).
                let old = lh.borrow().clone();
                if let Some(old_el) = old
                    && let Some(id_str) = old_el.get_attribute("data-onleave")
                    && let Ok(id) = id_str.parse::<usize>()
                {
                    events::dispatch_event(events::EventHandlerId(id));
                }
                if let Some(new_el) = new_hover.as_ref()
                    && let Some(id_str) = new_el.get_attribute("data-onenter")
                    && let Ok(id) = id_str.parse::<usize>()
                {
                    events::dispatch_event(events::EventHandlerId(id));
                }
                *lh.borrow_mut() = new_hover;
            });
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback(
            "pointermove",
            pointermove_closure.as_ref().unchecked_ref(),
        )
        .unwrap();
    pointermove_closure.forget();

    // Pointerup delegation: stop active drag operations and fire data-onmouseup.
    let browser_doc_for_up = browser_doc.clone();
    let pointerup_closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
        // Finish (not cancel) any pointer-capture Drag so its `on_end` fires with
        // the release position — matching the desktop backend
        // (`app::event_dispatch` calls `finish_drag` on mouseup). A no-op when no
        // drag is active. pointercancel still uses `Drag::cancel()` (no commit).
        rinch_core::finish_drag(event.client_x() as f32, event.client_y() as f32);
        release_drag_pointer_capture();
        if let Some(el) = pointer_hit_element(&event) {
            dispatch_mouse_attr(&el, "data-onmouseup", &event);
        }

        // Element drag-and-drop: only the owning pointer finishes the drag (a
        // secondary finger lifting must not end it). Finish drop/dragend, or fire
        // the click deferred when this draggable's pointerdown started a pending
        // drag (a tap that never activated).
        let owns = WEB_DRAG.with(|d| {
            d.borrow()
                .as_ref()
                .is_some_and(|s| s.pointer_id == event.pointer_id())
        });
        if owns && let Some(mut state) = WEB_DRAG.with(|d| d.borrow_mut().take()) {
            clear_long_press_timer(&mut state);
            release_capture_and_scroll(&mut state);
            if state.active {
                let x = event.client_x() as f32;
                let y = event.client_y() as f32;
                finish_active_drag(&state, x, y, true);
            } else {
                // Never activated (a quick tap / sub-threshold press) — treat as a
                // click on the draggable, with the same right-click/contextmenu
                // gate as the pointerdown path (a right-press under a
                // data-oncontextmenu ancestor is suppressed, not doubled with the
                // contextmenu).
                let click_allowed = event.button() == 0
                    || state
                        .source
                        .closest("[data-oncontextmenu]")
                        .ok()
                        .flatten()
                        .is_none();
                if click_allowed {
                    dispatch_click_at(&state.source, &browser_doc_for_up, &event);
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("pointerup", pointerup_closure.as_ref().unchecked_ref())
        .unwrap();
    pointerup_closure.forget();

    // Pointercancel delegation: the browser took the pointer away (e.g. a system
    // gesture, or scrolling won out). Cancel the owning drag like Escape — an
    // active drag fires dragend, a pending one is just cleared — and release any
    // slider drag too.
    let pointercancel_closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
        let owns = WEB_DRAG.with(|d| {
            d.borrow()
                .as_ref()
                .is_some_and(|s| s.pointer_id == event.pointer_id())
        });
        if owns {
            cancel_web_drag();
        }
        rinch_core::Drag::cancel();
        release_drag_pointer_capture();
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback(
            "pointercancel",
            pointercancel_closure.as_ref().unchecked_ref(),
        )
        .unwrap();
    pointercancel_closure.forget();

    // Suppress the browser's native HTML5 drag for rinch draggables: the element
    // drag-and-drop above is synthesized from pointer events, and a native drag
    // would otherwise hijack the gesture (suppressing pointermove/pointerup).
    // Preventing the default on `dragstart` cancels the native drag so the pointer
    // events keep flowing. (This is the browser's own `dragstart` event for
    // images/text/links — unrelated to our `data-ondragstart` handler attribute.)
    let dragstart_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
            && el.closest("[draggable=\"true\"]").ok().flatten().is_some()
        {
            event.prevent_default();
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("dragstart", dragstart_closure.as_ref().unchecked_ref())
        .unwrap();
    dragstart_closure.forget();

    // Keyboard delegation: route to focused render surface or keyboard interceptor.
    let keydown_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        // Escape cancels an in-progress element drag (consumed only if one was
        // actually active, so Escape otherwise reaches the app normally).
        if event.key() == "Escape" && cancel_web_drag() {
            event.prevent_default();
            return;
        }

        // If a render surface is focused, route keyboard events to it
        if let Some(surface_id) = rinch::render_surface::focused_surface_id() {
            let key_data = rinch::render_surface::SurfaceKeyData {
                key: event.key(),
                code: event.code(),
                ctrl: event.ctrl_key() || event.meta_key(),
                shift: event.shift_key(),
                alt: event.alt_key(),
                meta: event.meta_key(),
            };
            rinch::render_surface::dispatch_surface_event(
                surface_id,
                rinch::render_surface::SurfaceEvent::KeyDown(key_data),
            );
            // Also dispatch TextInput for printable characters
            let key = event.key();
            if key.len() == 1 && !event.ctrl_key() && !event.meta_key() && !event.alt_key() {
                rinch::render_surface::dispatch_surface_event(
                    surface_id,
                    rinch::render_surface::SurfaceEvent::TextInput(key),
                );
            }
            event.prevent_default();
            event.stop_propagation();
            return;
        }

        let key_data = events::KeyEventData {
            key: event.key(),
            code: event.code(),
            ctrl: event.ctrl_key() || event.meta_key(),
            shift: event.shift_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        };
        if events::dispatch_keyboard_event(&key_data) {
            event.prevent_default();
            event.stop_propagation();
        } else if event.key() == "Enter" && !event.shift_key() {
            // Enter (without Shift) fires the nearest ancestor's data-onsubmit,
            // matching a native form submit. Shift+Enter is left alone so
            // multiline inputs (a textarea) can insert a newline.
            if let Some(target) = event.target()
                && let Ok(el) = target.dyn_into::<web_sys::Element>()
            {
                let mut current: Option<web_sys::Element> = Some(el);
                while let Some(el) = current {
                    if let Some(handler_str) = el.get_attribute("data-onsubmit")
                        && let Ok(handler_id) = handler_str.parse::<usize>()
                    {
                        // Prevent the browser from also inserting a newline
                        // (in a textarea) or running a native form submit.
                        event.prevent_default();
                        events::dispatch_event(events::EventHandlerId(handler_id));
                        break;
                    }
                    current = el.parent_element();
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("keydown", keydown_closure.as_ref().unchecked_ref())
        .unwrap();
    keydown_closure.forget();

    // keyup: route to a focused render surface (games need key-release). No app
    // keyup delegation path exists, so this only acts when a surface is focused.
    let keyup_closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        if let Some(surface_id) = rinch::render_surface::focused_surface_id() {
            let key_data = rinch::render_surface::SurfaceKeyData {
                key: event.key(),
                code: event.code(),
                ctrl: event.ctrl_key() || event.meta_key(),
                shift: event.shift_key(),
                alt: event.alt_key(),
                meta: event.meta_key(),
            };
            rinch::render_surface::dispatch_surface_event(
                surface_id,
                rinch::render_surface::SurfaceEvent::KeyUp(key_data),
            );
            event.prevent_default();
            event.stop_propagation();
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback("keyup", keyup_closure.as_ref().unchecked_ref())
        .unwrap();
    keyup_closure.forget();

    // Input delegation: find [data-oninput] on the target or ancestors.
    let browser_doc2 = browser_doc.clone();
    let input_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(target) = event.target() {
            // Try HtmlInputElement first
            let value = if let Ok(input) = target.clone().dyn_into::<web_sys::HtmlInputElement>() {
                Some(input.value())
            } else if let Ok(textarea) = target.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
                Some(textarea.value())
            } else {
                None
            };

            if let Some(value) = value {
                // Walk up from target to find [data-oninput]
                if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                    let mut current: Option<web_sys::Element> = Some(el);
                    while let Some(el) = current {
                        if let Some(handler_str) = el.get_attribute("data-oninput")
                            && let Ok(handler_id) = handler_str.parse::<usize>()
                        {
                            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
                            break;
                        }
                        current = el.parent_element();
                    }
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc2
        .add_event_listener_with_callback("input", input_closure.as_ref().unchecked_ref())
        .unwrap();
    input_closure.forget();

    // Contextmenu delegation: dispatch data-oncontextmenu and suppress the
    // native browser menu when a handler is found.
    let contextmenu_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
            && let Ok(Some(menu_el)) = el.closest("[data-oncontextmenu]")
            && let Some(id_str) = menu_el.get_attribute("data-oncontextmenu")
            && let Ok(id) = id_str.parse::<usize>()
        {
            let rect = menu_el.get_bounding_client_rect();
            let (vp_w, vp_h) = viewport_dims();
            events::set_click_context(events::ClickContext {
                mouse_x: event.client_x() as f32,
                mouse_y: event.client_y() as f32,
                element_x: rect.x() as f32,
                element_y: rect.y() as f32,
                element_width: rect.width() as f32,
                element_height: rect.height() as f32,
                text_hit: Default::default(),
                viewport_width: vp_w,
                viewport_height: vp_h,
                button: events::MouseButton::Right,
                modifiers: modifiers_from_event(&event),
            });
            event.prevent_default();
            events::dispatch_event(events::EventHandlerId(id));
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback(
            "contextmenu",
            contextmenu_closure.as_ref().unchecked_ref(),
        )
        .unwrap();
    contextmenu_closure.forget();

    // Scroll delegation: the native `scroll` event fires on the scrolled element
    // and does NOT bubble, so register in the capture phase to catch all
    // descendants with a single document-level listener.
    let scroll_closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        if let Some(target) = event.target()
            && let Ok(el) = target.dyn_into::<web_sys::Element>()
            && let Some(id_str) = el.get_attribute("data-onscroll")
            && let Ok(id) = id_str.parse::<usize>()
        {
            events::dispatch_scroll_event(events::EventHandlerId(id), el.scroll_top() as f64);
        }
    }) as Box<dyn FnMut(_)>);
    browser_doc
        .add_event_listener_with_callback_and_bool(
            "scroll",
            scroll_closure.as_ref().unchecked_ref(),
            true, // capture phase
        )
        .unwrap();
    scroll_closure.forget();
}
