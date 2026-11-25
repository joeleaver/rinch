//! Keyboard event interception.

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
    /// **OS auto-repeat also arrives as `Down`**, and this payload still does
    /// not distinguish it from a fresh press.
    ///
    /// The *platform* event does, since issue #463:
    /// `PlatformEvent::KeyDown` carries a three-state `KeyRepeat`
    /// (`Fresh`/`Repeat`/`Unknown`), which is what the runtime's own
    /// once-per-press activation now reads. Surfacing it here is a further
    /// step, not a formality: the honest field is that same three-state one —
    /// a plain `bool` would report a held key as fresh on any backend that
    /// cannot see, which is the divergence a document-level hook exists to
    /// avoid — and `rinch-core` depends on neither `rinch-platform` (where the
    /// enum lives) nor the browser, so it needs a home for the type first.
    /// Tracked as issue #797.
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
    /// The logical key value, spelled as `KeyboardEvent.key` (e.g. `"a"`,
    /// `"Enter"`, `"Backspace"`). The spacebar is `" "` on every backend — it
    /// was `"Space"` on desktop until issue #1161; match it with
    /// [`Self::is_space`] or `code == "Space"`.
    pub key: String,
    /// The physical key code, spelled as `KeyboardEvent.code` (e.g. `"KeyA"`,
    /// `"Enter"`, `"Space"`).
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
    /// The element the key was sent to — the focused element, `KeyboardEvent.
    /// target` on the web — when the backend knows it (#2091). An element
    /// handler (`onkeydown`) hears keys from its descendants too; this tells a
    /// key meant for the element itself from one typed in a field inside it.
    /// Set on the keys offered to element handlers; `None` elsewhere, and from
    /// a host that does not say.
    pub target: Option<crate::dom::NodeId>,
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
            target: None,
        }
    }

    /// Name the element the key was sent to ([`Self::target`]).
    pub fn with_target(mut self, target: Option<crate::dom::NodeId>) -> Self {
        self.target = target;
        self
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

    /// Whether this is the spacebar (issue #1161).
    ///
    /// `key` is `" "` for it on desktop and on the web alike, as a browser's
    /// `KeyboardEvent.key` is; `code` is `"Space"`. Either identifies it, and
    /// this asks both, so a payload built by hand with only one of them — an
    /// embedder's own event, a test — still answers.
    pub fn is_space(&self) -> bool {
        self.key == " " || self.code == "Space"
    }
}

/// Type alias for the keyboard interceptor callback.
/// Returns true if the event was handled (should not propagate to the runtime).
pub type KeyboardInterceptor = Rc<dyn Fn(&KeyEventData) -> bool>;

/// The per-document interceptor map (issue #340) — see
/// [`DocScopedSlotMap`](crate::reactive::DocScopedSlotMap).
type InterceptorSlots = crate::reactive::DocScopedSlotMap<dyn Fn(&KeyEventData) -> bool>;

crate::doc_keyed! {
    /// One interceptor slot **per document**, plus the ownerless `None` entry
    /// for registrations made outside any dispatch (issue #340).
    ///
    /// This used to be a single slot for the whole thread, last-wins — the
    /// #134/#136 hazard: a thread can pump several documents' event streams (a
    /// desktop app and its DevTools window, two embedded `RinchContext`s), so
    /// the second document's [`set_keyboard_interceptor`] silently disabled
    /// the first's and whichever remained drove keyboard capture for both.
    /// Now a registration is keyed by
    /// [`current_dispatching_doc`](crate::context::current_dispatching_doc)
    /// and dispatch prefers the dispatching document's interceptor, falling
    /// back to the ownerless entry — so a hook installed from `main` still
    /// intercepts every document's keys, as it always has. (A hook installed
    /// at a `RinchApp` mount, or from an effect a document owns, is that
    /// document's: both run under its marker since issue #295.) The
    /// *lifetime* of each entry is issue #183 and is handled below.
    static KEYBOARD_INTERCEPTOR: InterceptorSlots = InterceptorSlots::new();
}

/// Set the keyboard interceptor for the current document.
///
/// Only one interceptor can be active at a time **per document**: a second
/// call from the same document replaces the first, while another document's
/// registration is a separate slot (issue #340). "The current document" is the
/// one whose code is running right now — its event dispatch, its mount, an
/// effect it owns, or a timer / `run_on_main_thread` closure / socket callback
/// it armed (issues #295, #963); registering outside any document — from
/// `main`, or from a timer armed there — fills the thread-global fallback
/// slot, which intercepts for every document that has no interceptor
/// of its own.
///
/// **Ahead of every other key handler** (issue #3062): the interceptor is the
/// app's capture-phase hook, offered a press before the focused element's
/// `onkeydown`, the rich-text editor's `on_key` and keymap, the popups' key
/// entries, the dismiss stack, a focused render surface and the app-wide
/// listeners; returning `true` keeps it from all of them. Only the app's menu
/// accelerators, Escape ending a drag, and (desktop) an open text context menu
/// or `<select>` list are asked first, as Chrome lets its own shortcuts and
/// popups take a key before the page. On the web it is a `window` capture
/// listener and backs off a key the page already `preventDefault`ed. It hears
/// every release too, first. The table is `docs/internals/focus-and-keyboard.md`
/// "The key handler order".
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
    crate::reactive::install_doc_scoped_slot(&KEYBOARD_INTERCEPTOR, Rc::new(cb));
}

/// Clear the keyboard interceptor a dispatch would reach right now: the
/// current document's own if it has one, else the thread-global fallback.
pub fn clear_keyboard_interceptor() {
    crate::reactive::clear_doc_scoped_slot(&KEYBOARD_INTERCEPTOR);
}

/// Dispatch a keyboard event to the interceptor and then the open popups: the
/// two halves below, [`dispatch_key_interceptor`] and then
/// [`dispatch_popup_key`], in one transaction. Returns true if either consumed
/// it.
///
/// The backends no longer call this for a press (issue #3062): they call the
/// halves at their own places in the key order, with the focused element's
/// `onkeydown` between them, as a browser runs a `window` capture listener,
/// then the target's listeners, then `document`'s. It stays for app and test
/// code that wants "what would the document-level hooks do with this key". See
/// `docs/internals/focus-and-keyboard.md` "The key handler order".
///
/// A **release** never dismisses and reaches no key entry: only the
/// interceptor hears it.
pub fn dispatch_keyboard_event(data: &KeyEventData) -> bool {
    // One transaction for the interceptor and the dismiss scan together, like
    // every other handler dispatch (`crate::reactive::batch`).
    crate::reactive::batch(|| {
        dispatch_key_interceptor_unbatched(data) || dispatch_popup_key_unbatched(data)
    })
}

/// Offer a key to the dispatching document's interceptor (or the thread-global
/// fallback) — [`set_keyboard_interceptor`]. Returns whether it consumed the
/// key.
///
/// **The interceptor is the app's capture-phase hook** (issue #3062): both
/// backends offer it a press after the app's menu accelerators and rinch's
/// three keyboard owners (Escape ending a drag or a pointer lock, an open
/// `<select>` list or text context menu), and before everything else — the
/// editor's and a node view's `on_key`, the focused element's `onkeydown`, the
/// open popups' key entries and the dismiss stack, `on_app_key` and every
/// rinch default. On rinch-web it runs from a `window` capture listener. A
/// release reaches it before the focused element's `onkeyup`, and its answer
/// is ignored there.
///
/// The `Rc` is cloned out before the call so the handler may re-enter (install a
/// different interceptor, for instance) without a double borrow. The handler
/// runs untracked (#931), in one `batch`.
pub fn dispatch_key_interceptor(data: &KeyEventData) -> bool {
    crate::reactive::batch(|| dispatch_key_interceptor_unbatched(data))
}

/// Offer a key **press** to the open popups: their
/// [key entries](super::push_key_handler), topmost first, then — for Escape
/// only — the [dismiss stack](super::dispatch_dismiss). Returns whether one
/// consumed it; a release always answers `false`.
///
/// Both backends call it after the focused element's `onkeydown` chain and
/// before `on_app_key` and rinch's defaults (the editor's keymap included), the
/// place a `document` listener has in a browser (issue #3062). That is what
/// lets a `[[` picker inside a `Modal` take Escape (#1678): the picker's
/// element handler runs first, and the modal's dismiss entry only sees an
/// Escape the picker left. Putting the dismiss scan here rather than at each
/// call site is what lets overlays close on Escape on every backend, and keeps
/// the question "what beats Escape?" answered in one place.
pub fn dispatch_popup_key(data: &KeyEventData) -> bool {
    crate::reactive::batch(|| dispatch_popup_key_unbatched(data))
}

fn dispatch_key_interceptor_unbatched(data: &KeyEventData) -> bool {
    match crate::reactive::read_doc_scoped_slot(&KEYBOARD_INTERCEPTOR) {
        // Untracked (#931): these functions are public, so they can be called
        // from inside an effect.
        Some(cb) => crate::reactive::untracked_handler(|| cb(data)),
        None => false,
    }
}

fn dispatch_popup_key_unbatched(data: &KeyEventData) -> bool {
    if !data.is_down() {
        return false;
    }
    // An open popup that owns the keyboard (#434) — see `push_key_handler`.
    if super::dismiss::dispatch_key_handlers(data) {
        return true;
    }
    if data.key == "Escape" {
        return super::dispatch_dismiss();
    }
    false
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
