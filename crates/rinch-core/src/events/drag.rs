//! Pointer capture drag API.
//!
//! Use `Drag::absolute()` or `Drag::percent()` to start tracking mouse
//! movement from a click handler until mouseup.

use std::cell::RefCell;
use std::rc::Rc;

use crate::events::{ClickContext, get_click_context};

/// How coordinates are delivered to the `on_move` callback.
enum DragMode {
    /// Raw viewport pixel coordinates.
    Absolute,
    /// Normalized 0.0–1.0 relative to element bounds captured at start.
    Percent {
        element_x: f32,
        element_y: f32,
        element_width: f32,
        element_height: f32,
    },
}

impl DragMode {
    /// Map raw viewport coordinates into this mode's delivery space
    /// (raw pixels for `Absolute`, clamped 0.0–1.0 for `Percent`).
    fn map(&self, mouse_x: f32, mouse_y: f32) -> (f32, f32) {
        match self {
            DragMode::Absolute => (mouse_x, mouse_y),
            DragMode::Percent {
                element_x,
                element_y,
                element_width,
                element_height,
            } => {
                let px = if *element_width > 0.0 {
                    ((mouse_x - element_x) / element_width).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let py = if *element_height > 0.0 {
                    ((mouse_y - element_y) / element_height).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (px, py)
            }
        }
    }
}

/// Internal state for an active drag.
struct ActiveDrag {
    mode: DragMode,
    on_move: Rc<dyn Fn(f32, f32) + 'static>,
    on_end: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
    on_cancel: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
    /// The ClickContext captured at drag start.
    start_context: ClickContext,
    /// The last coordinates delivered to `on_move`, in the drag's mode-specific
    /// coordinate space. `None` until the first move; `cancel()` then falls
    /// back to the start position mapped through the same mode.
    last_pos: Option<(f32, f32)>,
    /// Whether to continue dispatching surface events during this drag.
    forward_surface_events: bool,
    /// The scope that was rendering when `start()` was called, if any.
    ///
    /// A drag lives in a process-lifetime global, so it can outlive the
    /// component that armed it: an `on_move` that writes a signal can flip a
    /// branch, switch a tab, or reconcile the very list item the drag belongs
    /// to. Disposing that scope frees the signals these callbacks close over
    /// (issue #141), and the fixpoint cannot reach the callbacks themselves —
    /// they sit in no scope's owned bucket and are reachable from no effect. So
    /// the drag carries its owner and checks it before every dispatch.
    ///
    /// `Option`, not the `Owner::none()` sentinel: a *dangling* `Weak` is what a
    /// dropped scope also looks like, and the two must not be confused. `None`
    /// means the drag was armed outside any render and has app lifetime; `Some`
    /// that died means stop.
    ///
    /// Identity, not a flag: `ACTIVE_DRAG` is a single global slot, so a bool
    /// could not say *whose* drag was abandoned. `Owner` is a `Weak`, so this
    /// does not keep the scope — or anything it will free — alive.
    owner: Option<crate::reactive::Owner>,
}

impl ActiveDrag {
    /// Whether the scope that armed this drag has since been disposed.
    fn is_abandoned(&self) -> bool {
        self.owner.as_ref().is_some_and(|o| !o.is_alive())
    }
}

/// Drop the active drag if the scope that armed it is gone.
///
/// Returns `true` if a drag was discarded. Deliberately silent: `on_cancel` is
/// not fired, because it belongs to the same dead component and would be the
/// next thing to read a freed signal. There is nothing left to restore.
fn discard_if_abandoned() -> bool {
    let abandoned = ACTIVE_DRAG.with(|drag| {
        let mut slot = drag.borrow_mut();
        match slot.as_ref() {
            Some(state) if state.is_abandoned() => slot.take(),
            _ => None,
        }
    });
    // Dropped out here: the callbacks it owns are arbitrary user closures.
    if abandoned.is_some() {
        tracing::debug!("dropping an in-flight drag whose owning scope was disposed");
        drop(abandoned);
        return true;
    }
    false
}

thread_local! {
    static ACTIVE_DRAG: RefCell<Option<ActiveDrag>> = const { RefCell::new(None) };
}

/// Builder for starting a pointer capture drag.
///
/// # Examples
///
/// ```ignore
/// // Drag a panel with absolute coordinates
/// let ctx = get_click_context();
/// let offset_x = ctx.mouse_x - panel_x.get();
/// Drag::absolute()
///     .on_move(move |x, y| panel_x.set(x - offset_x))
///     .on_end(move |x, y| save_position(x, y))
///     .on_cancel(move |_x, _y| restore_position())
///     .start();
///
/// // Slider with percentage coordinates
/// Drag::percent()
///     .on_move(move |px, _| slider.set(px * 100.0))
///     .start();
///
/// // Access element position at drag start
/// let start = Drag::start_context().unwrap();
/// println!("Drag started on element at ({}, {})", start.element_x, start.element_y);
/// ```
pub struct Drag {
    mode: DragMode,
    on_move: Option<Box<dyn Fn(f32, f32) + 'static>>,
    on_end: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
    on_cancel: Option<Box<dyn FnOnce(f32, f32) + 'static>>,
    forward_surface_events: bool,
}

impl Drag {
    /// Start building a drag that delivers raw viewport pixel coordinates.
    pub fn absolute() -> Self {
        Self {
            mode: DragMode::Absolute,
            on_move: None,
            on_end: None,
            on_cancel: None,
            forward_surface_events: false,
        }
    }

    /// Start building a drag that delivers 0.0–1.0 percentage coordinates.
    ///
    /// Reads element bounds from the current `ClickContext` automatically.
    pub fn percent() -> Self {
        let ctx = get_click_context();
        Self {
            mode: DragMode::Percent {
                element_x: ctx.element_x,
                element_y: ctx.element_y,
                element_width: ctx.element_width,
                element_height: ctx.element_height,
            },
            on_move: None,
            on_end: None,
            on_cancel: None,
            forward_surface_events: false,
        }
    }

    /// Set the callback invoked on each mouse move during the drag.
    pub fn on_move<F: Fn(f32, f32) + 'static>(mut self, f: F) -> Self {
        self.on_move = Some(Box::new(f));
        self
    }

    /// Set the callback invoked once when the drag ends (mouseup).
    pub fn on_end<F: FnOnce(f32, f32) + 'static>(mut self, f: F) -> Self {
        self.on_end = Some(Box::new(f));
        self
    }

    /// Set the callback invoked once if the drag is cancelled via
    /// [`Drag::cancel`] (e.g. the web backend's `pointercancel` — a
    /// touch-scroll takeover, system gesture, or pointer-capture loss).
    ///
    /// Receives the last coordinates delivered to `on_move`, in the same
    /// coordinate space (raw pixels for `absolute`, 0.0–1.0 for `percent`);
    /// if the pointer never moved, the start position mapped into that space
    /// is delivered instead. `on_end` does **not** fire on cancellation, so
    /// use `on_cancel` for teardown (timers, hover state, overlays) and keep
    /// commit logic in `on_end`.
    pub fn on_cancel<F: FnOnce(f32, f32) + 'static>(mut self, f: F) -> Self {
        self.on_cancel = Some(Box::new(f));
        self
    }

    /// Allow surface events to continue dispatching during this drag.
    ///
    /// By default, active drags suppress all other event handling (hover,
    /// surface events, etc.). Set this to `true` if a `RenderSurface` needs
    /// to receive `MouseMove` events alongside the drag callback.
    pub fn forward_surface_events(mut self, forward: bool) -> Self {
        self.forward_surface_events = forward;
        self
    }

    /// Activate the drag. Call from a mousedown/click handler.
    pub fn start(self) {
        let on_move: Rc<dyn Fn(f32, f32)> = match self.on_move {
            Some(f) => Rc::from(f),
            None => Rc::new(|_, _| {}),
        };
        let start_context = get_click_context();
        // Captured here rather than at dispatch: `start()` runs inside the
        // handler that armed the drag, so the registering scope is ambient
        // (`register_handler` re-enters it on dispatch). By the time `on_move`
        // fires, the owner stack is unrelated.
        let owner = crate::reactive::current_owner();
        let previous = ACTIVE_DRAG.with(|drag| {
            drag.borrow_mut().replace(ActiveDrag {
                mode: self.mode,
                on_move,
                on_end: self.on_end,
                on_cancel: self.on_cancel,
                start_context,
                last_pos: None,
                forward_surface_events: self.forward_surface_events,
                owner,
            })
        });
        // Dropped outside the borrow: a superseded drag's callbacks are
        // arbitrary user closures, and their `Drop` may query the drag state.
        drop(previous);
    }

    /// Cancel the active drag, firing `on_cancel` (but **not** `on_end` — a
    /// cancelled drag must not commit).
    ///
    /// `on_cancel` receives the last coordinates delivered to `on_move`, or
    /// the start position mapped into the drag's coordinate space if the
    /// pointer never moved. A no-op when no drag is active.
    pub fn cancel() {
        // Take the state and release the borrow before invoking the callback
        // (mirrors `finish_drag`) so an `on_cancel` that queries or starts a
        // drag doesn't hit a re-entrant borrow.
        let cancelled = ACTIVE_DRAG.with(|drag| drag.borrow_mut().take());
        if let Some(state) = cancelled
            && let Some(on_cancel) = state.on_cancel
        {
            let (x, y) = state.last_pos.unwrap_or_else(|| {
                state
                    .mode
                    .map(state.start_context.mouse_x, state.start_context.mouse_y)
            });
            on_cancel(x, y);
        }
    }

    /// Check if a drag is currently active.
    ///
    /// A drag whose owning scope has been disposed is discarded here rather
    /// than reported as active — the runtime gates hover, surface events and
    /// text selection on this, and a stale `true` would wedge all three.
    pub fn is_active() -> bool {
        discard_if_abandoned();
        ACTIVE_DRAG.with(|drag| drag.borrow().is_some())
    }

    /// Get the [`ClickContext`] captured when the active drag started.
    ///
    /// Returns `None` if no drag is active. The context contains the element
    /// bounds and mouse position at the moment `start()` was called, which is
    /// useful for computing offsets in `Drag::absolute()` mode.
    ///
    /// ```ignore
    /// // In an on_move callback or elsewhere while drag is active:
    /// if let Some(ctx) = Drag::start_context() {
    ///     let elem_x = ctx.element_x;
    ///     let elem_y = ctx.element_y;
    /// }
    /// ```
    pub fn start_context() -> Option<ClickContext> {
        ACTIVE_DRAG.with(|drag| drag.borrow().as_ref().map(|d| d.start_context))
    }
}

/// Update the drag position. Called by the runtime on mouse move.
/// Returns `(handled, forward_surface_events)`:
/// - `handled`: true if a drag callback was invoked
/// - `forward_surface_events`: true if surface events should still be dispatched
pub fn update_drag(mouse_x: f32, mouse_y: f32) -> (bool, bool) {
    // A drag whose component was unmounted mid-gesture is dropped rather than
    // driven: its callbacks close over signals the dispose fixpoint has freed
    // (issue #141), so `on_move` would panic on the first read.
    discard_if_abandoned();

    // Map coords + record last_pos under a short borrow, then release it
    // before invoking `on_move` (which may itself query or cancel the drag).
    let pending = ACTIVE_DRAG.with(|drag| {
        drag.borrow_mut().as_mut().map(|state| {
            let (x, y) = state.mode.map(mouse_x, mouse_y);
            state.last_pos = Some((x, y));
            (state.on_move.clone(), x, y, state.forward_surface_events)
        })
    });
    if let Some((on_move, x, y, forward)) = pending {
        on_move(x, y);
        (true, forward)
    } else {
        (false, false)
    }
}

/// Finish the drag, firing `on_end` if set. Called by the runtime on mouseup.
///
/// A drag whose owning scope was disposed mid-gesture does not commit: `on_end`
/// is the *commit* callback, and there is nothing left to commit to.
pub fn finish_drag(mouse_x: f32, mouse_y: f32) {
    if discard_if_abandoned() {
        return;
    }
    let on_end = ACTIVE_DRAG.with(|drag| drag.borrow_mut().take().and_then(|s| s.on_end));
    if let Some(cb) = on_end {
        cb(mouse_x, mouse_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::set_click_context;
    use std::cell::Cell;

    #[test]
    fn test_cancel_fires_on_cancel_not_on_end() {
        let end_fired = Rc::new(Cell::new(false));
        let cancel_pos: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));

        let end = end_fired.clone();
        let cancel = cancel_pos.clone();
        Drag::absolute()
            .on_end(move |_, _| end.set(true))
            .on_cancel(move |x, y| cancel.set(Some((x, y))))
            .start();
        update_drag(42.0, 17.0);
        Drag::cancel();

        assert_eq!(cancel_pos.get(), Some((42.0, 17.0)));
        assert!(!end_fired.get());
        assert!(!Drag::is_active());
    }

    #[test]
    fn test_cancel_before_any_move_delivers_start_position() {
        let cancel_pos: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));

        set_click_context(ClickContext {
            mouse_x: 10.0,
            mouse_y: 20.0,
            ..Default::default()
        });
        let cancel = cancel_pos.clone();
        Drag::absolute()
            .on_cancel(move |x, y| cancel.set(Some((x, y))))
            .start();
        Drag::cancel();

        assert_eq!(cancel_pos.get(), Some((10.0, 20.0)));
        set_click_context(ClickContext::default());
    }

    #[test]
    fn test_cancel_percent_mode_delivers_percent_coords() {
        let cancel_pos: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));

        set_click_context(ClickContext {
            element_x: 0.0,
            element_y: 0.0,
            element_width: 100.0,
            element_height: 200.0,
            ..Default::default()
        });
        let cancel = cancel_pos.clone();
        Drag::percent()
            .on_cancel(move |x, y| cancel.set(Some((x, y))))
            .start();
        update_drag(25.0, 100.0);
        Drag::cancel();

        assert_eq!(cancel_pos.get(), Some((0.25, 0.5)));
        set_click_context(ClickContext::default());
    }

    #[test]
    fn test_finish_fires_on_end_not_on_cancel() {
        let end_pos: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));
        let cancel_fired = Rc::new(Cell::new(false));

        let end = end_pos.clone();
        let cancel = cancel_fired.clone();
        Drag::absolute()
            .on_end(move |x, y| end.set(Some((x, y))))
            .on_cancel(move |_, _| cancel.set(true))
            .start();
        finish_drag(5.0, 6.0);

        assert_eq!(end_pos.get(), Some((5.0, 6.0)));
        assert!(!cancel_fired.get());
        assert!(!Drag::is_active());
    }

    #[test]
    fn test_cancel_with_no_active_drag_is_noop() {
        assert!(!Drag::is_active());
        Drag::cancel(); // must not panic
        assert!(!Drag::is_active());
    }

    #[test]
    fn test_on_cancel_reentrancy_does_not_panic() {
        // The state is taken and the borrow released before on_cancel runs,
        // so the callback may query or re-cancel without panicking.
        let saw_inactive = Rc::new(Cell::new(false));

        let saw = saw_inactive.clone();
        Drag::absolute()
            .on_cancel(move |_, _| {
                saw.set(!Drag::is_active());
                Drag::cancel();
            })
            .start();
        Drag::cancel();

        assert!(saw_inactive.get());
    }

    /// A drag armed inside a component that is then unmounted mid-gesture is
    /// dropped rather than driven (issue #141).
    ///
    /// The callbacks close over the component's signals, which disposal has
    /// freed, so `on_move` would panic on the first read — and `ACTIVE_DRAG` is
    /// a process-lifetime global the dispose fixpoint cannot reach into.
    #[test]
    fn a_drag_whose_scope_was_disposed_stops_being_driven() {
        use crate::reactive::{Scope, Signal};

        let moves = Rc::new(Cell::new(0));
        let ends = Rc::new(Cell::new(0));

        let scope = Scope::new();
        let m = moves.clone();
        let e = ends.clone();
        scope.run(|| {
            let owned = Signal::new(0);
            Drag::absolute()
                .on_move(move |x, _| {
                    // Reads freed state if this is allowed to run post-dispose.
                    let _ = owned.get();
                    m.set(m.get() + 1);
                    owned.set(x as i32);
                })
                .on_end(move |_, _| e.set(e.get() + 1))
                .start();
        });

        assert!(Drag::is_active(), "the drag is armed");
        update_drag(10.0, 10.0);
        assert_eq!(moves.get(), 1, "and driven normally while mounted");

        scope.dispose();

        assert!(
            !Drag::is_active(),
            "an abandoned drag is not reported active"
        );
        update_drag(20.0, 20.0);
        finish_drag(20.0, 20.0);

        assert_eq!(moves.get(), 1, "no further on_move after the scope died");
        assert_eq!(ends.get(), 0, "and an abandoned drag must not commit");
    }

    /// The other half: a drag armed outside any render keeps app lifetime, so
    /// an unrelated scope being disposed must not disturb it.
    #[test]
    fn a_drag_armed_outside_a_render_is_never_abandoned() {
        use crate::reactive::Scope;

        let moves = Rc::new(Cell::new(0));
        let m = moves.clone();
        Drag::absolute()
            .on_move(move |_, _| m.set(m.get() + 1))
            .start();

        // An unrelated component unmounts.
        Scope::new().dispose();

        assert!(Drag::is_active(), "an ownerless drag survives");
        update_drag(5.0, 5.0);
        assert_eq!(moves.get(), 1, "and is still driven");
        Drag::cancel();
    }
}
