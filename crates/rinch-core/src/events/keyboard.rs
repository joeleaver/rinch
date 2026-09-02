//! Keyboard event interception.

use std::cell::RefCell;
use std::rc::Rc;

/// Which phase of a keystroke an event reports (issue #337).
///
/// A releases-aware consumer pairs a `Down` with its `Up` by comparing
/// [`KeyEventData::key`]. That pairing only works because press and release
/// are spelled by the same function from the same fields — see `hook_key_str`
/// in the desktop runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyEventKind {
    /// The key went down.
    ///
    /// **OS auto-repeat also arrives as `Down`**, and there is currently no
    /// flag distinguishing it from a fresh press: the browser supplies one
    /// (`KeyboardEvent.repeat`) but `PlatformEvent::KeyDown` does not carry
    /// winit's, so a `repeat` field here would be truthful on web and silently
    /// `false` on desktop — the exact divergence a document-level hook exists
    /// to avoid. It arrives with the plumbing, in the issue that retires the
    /// runtime's own hand-rolled activation latch.
    #[default]
    Down,
    /// The key came up.
    Up,
}

/// Keyboard event data for the global keyboard interceptor.
///
/// `#[non_exhaustive]`: build one with [`KeyEventData::new`] and the `with_*`
/// setters rather than a struct literal, so a field added in a minor release
/// is not a breaking change. Reading fields is unaffected — and reading is
/// what an interceptor does, since these are delivered, not constructed, by
/// app code.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct KeyEventData {
    /// The logical key value (e.g., "a", "Enter", "Backspace")
    pub key: String,
    /// The physical key code (e.g., "KeyA", "Enter")
    pub code: String,
    /// Whether Ctrl/Cmd is pressed
    pub ctrl: bool,
    /// Whether Shift is pressed
    pub shift: bool,
    /// Whether Alt is pressed
    pub alt: bool,
    /// Whether Meta/Super is pressed
    pub meta: bool,
    /// Press or release (issue #337). Before it, only presses were ever
    /// delivered, so a consumer had no way to see a key let go — and no way to
    /// ask, either, since every event was implicitly a press.
    pub kind: KeyEventKind,
}

impl KeyEventData {
    /// A press of `key` (spelled as `KeyboardEvent.key`) at physical `code`,
    /// with no modifiers held.
    ///
    /// Chain [`Self::with_modifiers`] and [`Self::with_kind`] for the rest.
    /// The constructor takes only the two fields that have no sensible
    /// default: every event has a key and a code, while "no modifiers, a
    /// press" is the common case and reads better as an absence than as four
    /// `false`s at every call site.
    pub fn new(key: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            kind: KeyEventKind::Down,
        }
    }

    /// Set all four modifiers at once, in the order they are declared:
    /// **ctrl, shift, alt, meta**.
    ///
    /// One call rather than four setters because they are always known
    /// together — they come off a single `Modifiers` — and splitting them
    /// invites a site that sets three and forgets the fourth, which is exactly
    /// how `meta` came to be hardcoded `false` on two of three paths (#336).
    pub fn with_modifiers(mut self, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Self {
        self.ctrl = ctrl;
        self.shift = shift;
        self.alt = alt;
        self.meta = meta;
        self
    }

    /// Mark this as a press or a release.
    pub fn with_kind(mut self, kind: KeyEventKind) -> Self {
        self.kind = kind;
        self
    }

    /// Whether this is a press — auto-repeat included.
    pub fn is_down(&self) -> bool {
        self.kind == KeyEventKind::Down
    }

    /// Whether this is a release.
    pub fn is_up(&self) -> bool {
        self.kind == KeyEventKind::Up
    }
}

/// Type alias for the keyboard interceptor callback.
/// Returns true if the event was handled (should not propagate to the runtime).
pub type KeyboardInterceptor = Rc<dyn Fn(&KeyEventData) -> bool>;

thread_local! {
    /// The one interceptor slot for the whole thread.
    ///
    /// Deliberately **not** keyed by document, unlike the pointer-capture drag
    /// (issue #139): unscoped and last-wins is a real hazard here — two
    /// documents on one thread (a desktop app and its DevTools window, two
    /// embedded `RinchContext`s) share this slot, so the second
    /// [`set_keyboard_interceptor`] silently displaces the first and every
    /// document's keys then reach whichever registered last. It stays a single
    /// slot only because there is no in-tree registrant today, so it is an API
    /// hazard rather than a live bug, and because the fix belongs with the
    /// `(doc_key, node_id)` focus registry in issue #147 — keyboard routing is
    /// one decision, and giving this its own parallel map would have to be
    /// unpicked to land it. Tracked as issue #340; the *lifetime* of whatever
    /// occupies the slot is issue #183 and is handled below.
    static KEYBOARD_INTERCEPTOR: RefCell<Option<KeyboardInterceptor>> = const { RefCell::new(None) };
}

/// Set the global keyboard interceptor.
///
/// Only one interceptor can be active at a time, **per thread, not per
/// document**: a second call from another document on the same thread replaces
/// the first (issue #139; the per-document routing is issue #340).
///
/// **Released on unmount.** Registering from inside a render ties the
/// interceptor to the ambient scope, so disposing that scope clears it — a
/// callback that captured a `Signal` cannot outlive the signal and read freed
/// state (issue #183). The cleanup only clears the slot if this interceptor is
/// *still* the one installed, so a later `set_keyboard_interceptor` is never
/// clobbered by an earlier component unmounting. Registering outside any
/// render — from `main`, a timer, a detached callback — has no owner and so
/// lives for the life of the app, as before.
pub fn set_keyboard_interceptor<F>(cb: F)
where
    F: Fn(&KeyEventData) -> bool + 'static,
{
    crate::reactive::install_scoped_slot(&KEYBOARD_INTERCEPTOR, Rc::new(cb));
}

/// Clear the global keyboard interceptor.
pub fn clear_keyboard_interceptor() {
    crate::reactive::clear_scoped_slot(&KEYBOARD_INTERCEPTOR);
}

/// Dispatch a keyboard event to the interceptor.
/// Returns true if the event was handled.
///
/// The `Rc` is cloned out before the call so the handler may re-enter (install a
/// different interceptor, for instance) without a double borrow.
pub fn dispatch_keyboard_event(data: &KeyEventData) -> bool {
    match crate::reactive::read_scoped_slot(&KEYBOARD_INTERCEPTOR) {
        Some(cb) => cb(data),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::reactive::{Scope, Signal};

    fn key(name: &str) -> KeyEventData {
        KeyEventData::new(name.to_string(), name.to_string())
    }

    /// `with_modifiers` takes four positional bools — the shape whose failure
    /// mode is a silent transposition, which no symmetric-modifier test can
    /// see. Two asymmetric patterns pin every position to its field.
    #[test]
    fn with_modifiers_assigns_each_position_to_its_field() {
        let a = KeyEventData::new("a", "KeyA").with_modifiers(true, false, true, false);
        assert!(a.ctrl && !a.shift && a.alt && !a.meta, "{a:?}");
        let b = KeyEventData::new("a", "KeyA").with_modifiers(false, true, false, true);
        assert!(!b.ctrl && b.shift && !b.alt && b.meta, "{b:?}");
    }

    /// #183: a registry that outlives the component that filled it hands a
    /// disposed scope's state to the next event. Registering inside a render
    /// ties the interceptor to that scope.
    #[test]
    fn an_interceptor_registered_in_a_scope_is_released_when_the_scope_disposes() {
        clear_keyboard_interceptor();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(move || {
            set_keyboard_interceptor(move |_| {
                flag.set(true);
                true
            })
        });
        assert!(
            dispatch_keyboard_event(&key("a")),
            "the interceptor is live while its scope is"
        );
        assert!(ran.get());
        ran.set(false);

        scope.dispose();
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "disposing the owning scope must release the interceptor"
        );
        assert!(!ran.get(), "a released interceptor must not run");
    }

    /// An earlier component unmounting must not clear a *later* component's
    /// interceptor — the cleanup only takes the slot back if it still holds
    /// the one it installed.
    #[test]
    fn an_earlier_scopes_cleanup_does_not_clobber_a_later_interceptor() {
        clear_keyboard_interceptor();
        let first = Scope::new();
        first.run(|| set_keyboard_interceptor(|_| false));

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let second = Scope::new();
        second.run(move || {
            set_keyboard_interceptor(move |_| {
                flag.set(true);
                true
            })
        });

        first.dispose();
        assert!(
            dispatch_keyboard_event(&key("a")),
            "the second interceptor must survive the first scope's disposal"
        );
        assert!(ran.get(), "the surviving interceptor is the second one");

        second.dispose();
        assert!(!dispatch_keyboard_event(&key("a")));
    }

    /// Registering outside any render has no owner, so nothing releases it —
    /// the pre-existing app-lifetime behaviour.
    #[test]
    fn an_interceptor_registered_with_no_ambient_owner_lives_on() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| true);
        Scope::new().dispose();
        assert!(
            dispatch_keyboard_event(&key("a")),
            "an ownerless interceptor keeps app lifetime"
        );
        clear_keyboard_interceptor();
    }

    /// The actual #183 shape: the interceptor captured a signal its component
    /// owns. Once the component is gone the signal's storage is freed, and a
    /// read of a freed signal panics — so the interceptor must never run again.
    #[test]
    fn a_released_interceptor_does_not_read_its_components_freed_signal() {
        clear_keyboard_interceptor();
        let scope = Scope::new();
        let count = scope.run(|| {
            let count = Signal::new(7);
            set_keyboard_interceptor(move |_| count.get() > 0);
            count
        });

        assert!(dispatch_keyboard_event(&key("a")));

        scope.dispose();
        assert!(!count.is_alive(), "disposal freed the component's signal");
        // Before the fix this dispatches into the stale interceptor, which
        // reads the freed signal and panics.
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "a released interceptor must not read its component's freed state"
        );
    }

    /// The dispatch must not hold the slot's borrow across user code: an
    /// interceptor is allowed to install its replacement from inside its own
    /// dispatch.
    #[test]
    fn an_interceptor_may_replace_itself_from_inside_dispatch() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| {
            set_keyboard_interceptor(|_| false);
            true
        });

        assert!(dispatch_keyboard_event(&key("a")), "the first ran");
        assert!(
            !dispatch_keyboard_event(&key("a")),
            "and installed its replacement"
        );
        clear_keyboard_interceptor();
    }
}
