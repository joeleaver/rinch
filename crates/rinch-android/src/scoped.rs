//! Callback registries that know which component filled them (issue #183).
//!
//! Every platform service in this crate hands the app a callback and files it in
//! a `thread_local!` registry: a sensor reading, a location fix, an activity
//! lifecycle transition, the result of a file picker. Those callbacks are the
//! natural thing to write inside a `#[component]`, so they capture that
//! component's `Signal`s — and since #141 PR4 a scope *owns* the signals created
//! while it was the ambient owner, and disposing it frees them. A registry with
//! no tie to that scope therefore hands the next frame an unmounted component's
//! state: a write to a freed signal is a lenient warn-once no-op, but a **read**
//! panics.
//!
//! This module holds the one copy of the fix, and it is deliberately **not**
//! behind `cfg(target_os = "android")`: the whole of the lifetime, dispatch and
//! re-entrancy logic compiles and is unit-tested on the host, so the modules that
//! consume it are left holding only their JNI call sites. Nothing in this
//! repository builds an APK in CI, so logic that lives behind the `cfg` is logic
//! nothing checks.
//!
//! # Which template this is, and why
//!
//! `rinch_core::reactive` offers two shapes for the same problem. The
//! *cleanup-tied* one (`install_scoped_slot`) registers an `on_cleanup` per
//! registration and reclaims the slot when the scope disposes. The
//! *dispatch-checked* one, established by `main_thread::park_main_callback`,
//! keeps the [`Owner`] beside the callback and tests
//! [`is_alive`](Owner::is_alive) when the event arrives.
//!
//! These registries take the second, for three reasons:
//!
//! 1. **They are written from live components, repeatedly.** `location::start`
//!    is called from an `onclick` in `examples/hello-android`, and a handler
//!    dispatch re-pushes its registration-time owner — so under the cleanup-tied
//!    shape every tap of "Start GPS" would append a boxed cleanup and a pinning
//!    `Weak` that live as long as the component does. `scoped.rs`'s own docs in
//!    `rinch-core` name this as the case that must use the other template.
//! 2. **A cleanup cannot attribute what the callback allocates.** Running the
//!    callback inside [`Owner::run`] means a `Signal` created by a sensor
//!    callback belongs to the component that registered it, rather than to
//!    whatever the event loop happened to be doing. That asymmetry is the second
//!    defect issue #183 records.
//! 3. **Every one of these registries is already drained once a frame**, so a
//!    liveness test at drain time is free and prompt — the same discipline as
//!    the poll and bounds registries in `rinch-core`, which derive their lifetime
//!    from what drives them.
//!
//! # The rules it encodes
//!
//! - **Ownerless registration keeps app lifetime.** [`current_owner`] returns
//!   `None` outside any render — from `android_main`, from a timer, from a
//!   detached callback — and such a callback must keep firing forever. That is
//!   the pre-#141 default and it must never regress.
//! - **An ownerless callback dispatches under [`unowned`], not bare.** The owner
//!   stack is not an ancestor chain: dispatching bare would let an app-lifetime
//!   callback allocate into whatever unrelated scope the drain happened to be
//!   nested inside, to be freed when *that* component unmounts.
//! - **Never hold the registry borrow across the callback.** Stopping a sensor
//!   once a reading crosses a threshold, or swapping a lifecycle handler, is
//!   ordinary use and re-enters the registry; under the borrow it is a
//!   `BorrowMutError`. Multi-shot callbacks are held in an [`Rc`] and cloned out
//!   first; one-shots are removed first.
//! - **Read and prune under one borrow.** A dispatch decides in a single
//!   `borrow_mut` whether to clone a live entry out or take a dead one out, so
//!   there is no window in which a registration that replaced the dead entry
//!   could be reclaimed by mistake. `rinch-core`'s cleanup-tied helper needs an
//!   [`Rc::ptr_eq`] "is this still mine" test precisely because its removal
//!   happens later, from a scope cleanup; here it happens in the same borrow as
//!   the read, so the test would be unreachable code. Testing liveness under the
//!   borrow is safe — it is a `Weak` upgrade and a flag read, never user code.
//! - **Drop the displaced callback after the borrow ends.** It is user code whose
//!   `Drop` may re-enter the registry; dropping it inside the `borrow_mut`
//!   panics. Every write here binds it to a `let` that outlives the borrow.
//! - **A release reports what it freed, and only what is now unclaimed.** Half
//!   of these callbacks are driving hardware — a sensor at `DELAY_UI`, the GPS
//!   radio — and once the entry is gone nothing else can turn it off:
//!   `sensors::stop` takes the `SensorType` the entry held, and the component
//!   that knew it is disposed. So a sweep hands its caller the keys it released,
//!   to power them down. It must not hand over a key something live holds: the
//!   released callback is user code whose `Drop` runs *inside* the sweep and may
//!   re-register at that very key, and disarming then would stop a live
//!   component's sensor. Both sweeps therefore re-read the registry after the
//!   drops and report only what is still empty.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use rinch_core::reactive::{Owner, current_owner, unowned};

/// The scope that was rendering when a callback was registered, if any.
///
/// `Owner` is a `Weak`, so this keeps nothing alive. `None` means the
/// registration happened outside any render and the callback has app lifetime.
#[derive(Clone)]
struct Registrant(Option<Owner>);

impl Registrant {
    /// Capture the ambient owner. Must be called where the *user's* closure was
    /// created — that is where it captured its signals.
    fn current() -> Self {
        Registrant(current_owner())
    }

    /// Whether the component that registered the callback is gone.
    ///
    /// `false` for an ownerless registration, which has app lifetime.
    fn is_dead(&self) -> bool {
        self.0.as_ref().is_some_and(|owner| !owner.is_alive())
    }

    /// Run `f` with the registering component as the ambient owner, so what it
    /// allocates belongs to that component — or with *no* owner when the
    /// registration was ownerless, so an app-lifetime callback cannot allocate
    /// into whatever scope the dispatch is nested inside.
    fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        match &self.0 {
            Some(owner) => owner.run(f),
            None => unowned(f),
        }
    }
}

/// One registered callback plus the scope that registered it.
#[derive(Clone)]
struct Entry<C> {
    registrant: Registrant,
    cb: C,
}

// ── Single-slot registries ──────────────────────────────────────────────────

/// A last-wins callback slot whose entry stops firing — and is released — once
/// the component that filled it is gone.
pub struct ScopedSlot<F: ?Sized> {
    slot: RefCell<Option<Entry<Rc<F>>>>,
}

impl<F: ?Sized> Default for ScopedSlot<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: ?Sized> ScopedSlot<F> {
    /// An empty slot. `const` so it can initialise a `thread_local!` directly.
    pub const fn new() -> Self {
        Self {
            slot: RefCell::new(None),
        }
    }

    /// Install `cb`, recording the scope that is currently rendering.
    pub fn install(&self, cb: Rc<F>) {
        // Built before the borrow: `Registrant::current` reads the reactive
        // runtime, and the displaced callback is user code whose `Drop` may
        // re-enter this slot. `_displaced` outlives the `RefMut`.
        let entry = Entry {
            registrant: Registrant::current(),
            cb,
        };
        let _displaced = self.slot.borrow_mut().replace(entry);
    }

    /// Empty the slot, dropping the callback **after** the borrow ends.
    pub fn clear(&self) {
        let _displaced = self.slot.borrow_mut().take();
    }

    /// Whether a callback is currently installed, live or not.
    #[cfg(test)]
    pub fn is_installed(&self) -> bool {
        self.slot.borrow().is_some()
    }

    /// Invoke the installed callback with no borrow held, so it may clear or
    /// replace the slot from inside its own dispatch.
    ///
    /// Returns whether it ran. A callback whose component is gone is **pruned**
    /// rather than called: leaving it would re-check it on every later event,
    /// and keep alive everything it captured.
    pub fn dispatch(&self, call: impl FnOnce(&F)) -> bool {
        // One borrow decides both: clone a live entry out so the callback runs
        // with no borrow held, or take a dead one out so it drops — after the
        // borrow ends, since its `Drop` is user code.
        let (live, _dead) = {
            let mut slot = self.slot.borrow_mut();
            match slot.as_ref() {
                Some(entry) if entry.registrant.is_dead() => (None, slot.take()),
                other => (other.cloned(), None),
            }
        };
        let Some(entry) = live else {
            return false;
        };
        entry.registrant.run(|| call(&entry.cb));
        true
    }

    /// Release the callback if the component that registered it is gone, whether
    /// or not an event ever arrives for it.
    ///
    /// [`dispatch`](Self::dispatch) prunes what it visits, but a sensor that has
    /// fallen silent or an activity result that never comes back would otherwise
    /// hold the callback — and everything it captured — for the life of the
    /// process. Cheap enough to run from a per-frame drain: a `Weak` upgrade and
    /// a flag read.
    ///
    /// Answers `true` only when the slot is **still empty** afterwards, because
    /// the caller's response to `true` is to power hardware down: dropping the
    /// callback runs user code, and a `Drop` that re-installs a live one has
    /// claimed the slot before the caller can act on the answer.
    pub fn release_if_dead(&self) -> bool {
        let released = {
            let mut slot = self.slot.borrow_mut();
            if slot.as_ref().is_some_and(|e| e.registrant.is_dead()) {
                slot.take()
            } else {
                None
            }
        };
        let was_released = released.is_some();
        // Explicitly, and outside the borrow above: this is user code, and it
        // must have finished re-entering the slot before the re-read below.
        drop(released);
        was_released && self.slot.borrow().is_none()
    }
}

// ── Additive registries ─────────────────────────────────────────────────────

/// A registry where **every** entry hears every event, each released once the
/// component that registered it is gone. Neither keyed nor last-wins.
///
/// # Why a third shape, rather than one of the two it sits between
///
/// [`ScopedSlot`] is last-wins, and a caller that wants several listeners is not
/// asking for a slot in a form it can afford: `intent::on_incoming_intent`'s
/// contract is that *"two independent parts of an app can each care about being
/// shared to without having to know about each other, and neither can silently
/// disable the other by registering second"*. A slot inverts that by
/// construction — the second registrant disables the first, silently, which is
/// exactly the thing the contract rules out.
///
/// [`ScopedMap`] is many, but its key is how a caller **addresses** an entry:
/// install at a key, remove at a key, dispatch *to* a key. Nothing here
/// addresses an entry — every registrant hears every event, and none is ever
/// removed by name. A key would be write-only, minted to satisfy the helper
/// rather than because the API has one, and
/// [`release_dead`](ScopedMap::release_dead)'s report — whose whole purpose in
/// the keyed twin is to tell the caller which sensor to power down — would hand
/// back identifiers that mean nothing to anybody.
///
/// So what this adds is the one axis the other two cannot express between them:
/// **many entries with no identity.** That it took three registries in three
/// pull requests to need it is the reason it is being written now rather than
/// guessed at earlier (issue #599).
///
/// # What it keeps from them, and the one thing it deliberately does not
///
/// The module's rules all apply unchanged: ownerless registration keeps app
/// lifetime, the callback runs inside its registrant's [`Owner`], no borrow is
/// held across user code, and a dispatch reads and prunes under one borrow.
///
/// **`release_dead` does not re-read after the drops, and the siblings do.**
/// Theirs must: their caller's response to the report is to power hardware down,
/// and a released callback whose `Drop` re-registers at that very key has
/// claimed it before the caller can act. Here the report is a *count*, with no
/// key attached and nothing to disarm — "this many entries were removed" stays
/// true whatever a `Drop` did afterwards, so the re-read would be ceremony
/// copied from a sibling rather than a rule.
pub struct ScopedList<F: ?Sized> {
    entries: RefCell<Vec<Entry<Rc<F>>>>,
}

impl<F: ?Sized> Default for ScopedList<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: ?Sized> ScopedList<F> {
    /// An empty registry. `const` so it can initialise a `thread_local!`
    /// directly.
    pub const fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Add `cb`, recording the scope that is currently rendering.
    ///
    /// Appends rather than replaces — that is the whole point of this shape —
    /// so **delivery order is registration order**, and it stays that way
    /// across a prune (see [`dispatch`](Self::dispatch)).
    pub fn install(&self, cb: Rc<F>) {
        // Built before the borrow: `Registrant::current` reads the reactive
        // runtime, which must not happen under this `RefCell`.
        let entry = Entry {
            registrant: Registrant::current(),
            cb,
        };
        self.entries.borrow_mut().push(entry);
    }

    /// Whether any entry's component is still alive.
    ///
    /// Distinct from "is the list non-empty", and the distinction is
    /// load-bearing for any caller that treats "nobody is listening" as a reason
    /// to hold an event back: a list of nothing but dead entries would otherwise
    /// read as a listener, and the event would be delivered to no one and lost.
    pub fn has_live(&self) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|entry| !entry.registrant.is_dead())
    }

    /// How many entries are held, live or not.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    /// Empty the registry. Test-only: this shape has no removal API by design,
    /// and the fixtures share one `thread_local!`.
    #[cfg(test)]
    pub fn clear_for_test(&self) {
        let _displaced = std::mem::take(&mut *self.entries.borrow_mut());
    }

    /// Invoke every live entry, oldest registration first, with no borrow held
    /// — so a callback may register another from inside its own dispatch.
    ///
    /// Returns how many ran. Entries whose component is gone are **pruned**
    /// rather than called, exactly as [`ScopedSlot::dispatch`] prunes its one:
    /// leaving them would re-check them on every later event and keep alive
    /// everything they captured.
    ///
    /// A callback registered *during* this dispatch does not receive this
    /// event. The snapshot is taken before any user code runs, which is what
    /// makes the delivery well-defined; a caller that wants a new registrant to
    /// see the *next* event in the same batch gets that by calling this once per
    /// event rather than once per batch.
    pub fn dispatch(&self, call: impl Fn(&F)) -> usize {
        // One borrow decides both: live entries are cloned out so they run with
        // no borrow held, dead ones are taken out so they drop — after the
        // borrow ends, since their `Drop` is user code that may re-enter here.
        let (live, _dead) = {
            let mut entries = self.entries.borrow_mut();
            let mut live: Vec<Entry<Rc<F>>> = Vec::with_capacity(entries.len());
            let mut dead: Vec<Entry<Rc<F>>> = Vec::new();
            // Drained and rebuilt in order rather than `retain`ed, so the
            // survivors keep their registration order across a prune. A swap
            // removal would be cheaper and would silently reorder delivery.
            for entry in std::mem::take(&mut *entries) {
                if entry.registrant.is_dead() {
                    dead.push(entry);
                } else {
                    live.push(entry.clone());
                    entries.push(entry);
                }
            }
            (live, dead)
        };
        for entry in &live {
            entry.registrant.run(|| call(&entry.cb));
        }
        live.len()
    }

    /// Release every entry whose component is gone, whether or not an event
    /// ever arrives for it, and report how many went.
    ///
    /// [`dispatch`](Self::dispatch) prunes what it visits, but an event that
    /// never comes prunes nothing — and this registry has no removal API at all,
    /// so without a sweep a callback registered by a component that has since
    /// unmounted is held, with everything it captured, for the life of the
    /// process. Cheap enough to run from a per-frame drain: a `Weak` upgrade and
    /// a flag read per entry.
    pub fn release_dead(&self) -> usize {
        let released = {
            let mut entries = self.entries.borrow_mut();
            let mut dead: Vec<Entry<Rc<F>>> = Vec::new();
            for entry in std::mem::take(&mut *entries) {
                if entry.registrant.is_dead() {
                    dead.push(entry);
                } else {
                    entries.push(entry);
                }
            }
            dead
        };
        let released_count = released.len();
        // Explicitly, and outside the borrow above: this is user code whose
        // `Drop` may re-enter the registry.
        drop(released);
        released_count
    }
}

// ── Keyed registries ────────────────────────────────────────────────────────

/// A keyed registry of repeat-firing callbacks, each released once the component
/// that registered it is gone. The keyed twin of [`ScopedSlot`].
pub struct ScopedMap<K, F: ?Sized> {
    entries: RefCell<HashMap<K, Entry<Rc<F>>>>,
}

impl<K: Eq + Hash, F: ?Sized> Default for ScopedMap<K, F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash, F: ?Sized> ScopedMap<K, F> {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
        }
    }

    /// Install `cb` under `key`, recording the scope that is currently rendering.
    pub fn install(&self, key: K, cb: Rc<F>) {
        let entry = Entry {
            registrant: Registrant::current(),
            cb,
        };
        // `_displaced` outlives the borrow, as in `ScopedSlot::install`.
        let _displaced = self.entries.borrow_mut().insert(key, entry);
    }

    /// Remove `key`, dropping the callback **after** the borrow ends.
    pub fn remove(&self, key: &K) {
        let _displaced = self.entries.borrow_mut().remove(key);
    }

    /// Whether a callback is registered under `key`, live or not.
    #[cfg(test)]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.borrow().contains_key(key)
    }

    /// Invoke `key`'s callback with no borrow held, so it may remove or replace
    /// any entry from inside its own dispatch. See [`ScopedSlot::dispatch`].
    pub fn dispatch(&self, key: &K, call: impl FnOnce(&F)) -> bool {
        // One borrow, as in `ScopedSlot::dispatch`.
        let (live, _dead) = {
            let mut entries = self.entries.borrow_mut();
            match entries.get(key) {
                Some(entry) if entry.registrant.is_dead() => (None, entries.remove(key)),
                other => (other.cloned(), None),
            }
        };
        let Some(entry) = live else {
            return false;
        };
        entry.registrant.run(|| call(&entry.cb));
        true
    }

    /// Drop every entry whose registering component is gone, whether or not an
    /// event ever arrives for it, and return the keys that are now **unclaimed**
    /// — so the caller can power down whatever they were driving.
    ///
    /// A released key is withheld when a live entry occupies it again by the
    /// time the sweep finishes. That is not hypothetical: the released callbacks
    /// are user code, their `Drop` runs during this call, and re-registering at
    /// the same key is exactly what a restart looks like. See
    /// [`ScopedSlot::release_if_dead`], which withholds for the same reason.
    pub fn release_dead(&self) -> Vec<K> {
        let released = release_dead_entries(&self.entries);
        let (unclaimed, _reclaimed): (Vec<K>, Vec<K>) = {
            let entries = self.entries.borrow();
            released
                .into_iter()
                .partition(|key| !entries.contains_key(key))
        };
        // `partition` moves every key into one vector or the other, so none is
        // dropped under the borrow above; `_reclaimed` drops after it ends.
        unclaimed
    }
}

/// Drop every entry whose registering component is gone, returning the keys
/// that went. Shared by both keyed registries — the repeat-firing one and the
/// one-shot one differ only in what they hold as the callback.
///
/// One pass, and the callbacks are dropped **after** the borrow ends: they are
/// user code whose `Drop` may re-enter the registry, which inside the
/// `borrow_mut` would panic. They are also dropped before this returns, so a
/// caller that re-reads the registry sees whatever those `Drop`s registered.
fn release_dead_entries<K: Eq + Hash, C>(entries: &RefCell<HashMap<K, Entry<C>>>) -> Vec<K> {
    let released = {
        let mut entries = entries.borrow_mut();
        // `collect` exhausts the iterator, so `ExtractIf`'s own `Drop` has
        // nothing left to remove while the borrow is still held.
        entries
            .extract_if(|_, e| e.registrant.is_dead())
            .collect::<Vec<_>>()
    };
    // Each callback drops as `map` moves past it — outside the borrow above.
    released.into_iter().map(|(key, _cb)| key).collect()
}

/// What became of a one-shot callback when its result arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The callback ran.
    Ran,
    /// Nothing was registered under that key.
    Unregistered,
    /// The component that registered it is gone, so it was dropped undelivered.
    Dropped,
}

/// A keyed registry of **one-shot** callbacks, removed on delivery.
///
/// Removal on delivery bounds the leak; it does not scope the lifetime. An
/// Android activity result comes back whatever the user does — a cancelled
/// picker still delivers `RESULT_CANCELED` — so a callback registered by a
/// component that has since unmounted is delivered exactly once, into freed
/// state.
pub struct ScopedOnceMap<K, T> {
    entries: RefCell<HashMap<K, OnceEntry<T>>>,
}

/// One registered `FnOnce` plus the scope that registered it.
type OnceEntry<T> = Entry<Box<dyn FnOnce(T)>>;

impl<K: Eq + Hash, T> Default for ScopedOnceMap<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash, T> ScopedOnceMap<K, T> {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
        }
    }

    /// Install `cb` under `key`, recording the scope that is currently rendering.
    pub fn install(&self, key: K, cb: Box<dyn FnOnce(T)>) {
        let entry = Entry {
            registrant: Registrant::current(),
            cb,
        };
        let _displaced = self.entries.borrow_mut().insert(key, entry);
    }

    /// Take `key`'s callback out of the registry and run it with `arg`.
    ///
    /// Removed *before* the call, so the callback may register the next step of a
    /// chain — `take_photo` asks for the CAMERA permission and registers the
    /// activity callback from inside the permission result — without a
    /// double borrow.
    pub fn deliver(&self, key: &K, arg: T) -> Delivery {
        let taken = self.entries.borrow_mut().remove(key);
        let Some(Entry { registrant, cb }) = taken else {
            return Delivery::Unregistered;
        };
        if registrant.is_dead() {
            // Dropped here, outside the borrow above.
            drop(cb);
            return Delivery::Dropped;
        }
        registrant.run(move || cb(arg));
        Delivery::Ran
    }

    /// Drop every callback whose registering component is gone, returning how
    /// many went. A result that never arrives would otherwise pin what its
    /// callback captured forever.
    ///
    /// A count, not the keys its repeat-firing twin reports: a one-shot arms no
    /// hardware. The request it belongs to — a picker intent, a permission
    /// dialog — is Android's to finish, and it delivers a result whatever the
    /// app does, so there is nothing here to power down.
    pub fn release_dead(&self) -> usize {
        release_dead_entries(&self.entries).len()
    }

    /// Whether a callback is registered under `key`, live or not.
    #[cfg(test)]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.borrow().contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;

    thread_local! {
        static SLOT: ScopedSlot<dyn Fn()> = const { ScopedSlot::new() };
        static LIST: ScopedList<dyn Fn()> = const { ScopedList::new() };
        static MAP: ScopedMap<u8, dyn Fn()> = ScopedMap::new();
        static ONCE: ScopedOnceMap<u8, u32> = ScopedOnceMap::new();
    }

    fn allocate_a_signal() -> Rc<dyn Fn()> {
        Rc::new(|| {
            let _owned = Signal::new(0u32);
        })
    }

    /// An ownerless callback has app lifetime, so what it allocates must not be
    /// handed to whatever scope the drain happened to be nested inside — the
    /// owner stack is not an ancestor chain, and that scope may unmount first.
    #[test]
    fn an_ownerless_callback_does_not_allocate_into_the_dispatching_scope() {
        // Deliberately not inside a `Scope::run`.
        SLOT.with(|slot| slot.install(allocate_a_signal()));

        let unrelated = Scope::new();
        let before = unrelated.owned_counts().signals;
        unrelated.run(|| {
            SLOT.with(|slot| slot.dispatch(|cb| cb()));
        });
        let after = unrelated.owned_counts().signals;

        assert_eq!(
            after, before,
            "an app-lifetime callback must not allocate into the scope that \
             happened to be ambient at dispatch"
        );
        unrelated.dispose();
    }

    /// And the mirror image: a callback with an owner is attributed to *that*
    /// scope, not to the one the dispatch is nested inside.
    #[test]
    fn a_callback_is_attributed_to_its_registering_scope_not_the_dispatching_one() {
        let registrar = Scope::new();
        registrar.run(|| MAP.with(|map| map.install(1, allocate_a_signal())));

        let dispatcher = Scope::new();
        let registrar_before = registrar.owned_counts().signals;
        let dispatcher_before = dispatcher.owned_counts().signals;
        dispatcher.run(|| {
            MAP.with(|map| map.dispatch(&1, |cb| cb()));
        });

        assert_eq!(
            registrar.owned_counts().signals,
            registrar_before + 1,
            "the registering scope must own what the callback allocates"
        );
        assert_eq!(
            dispatcher.owned_counts().signals,
            dispatcher_before,
            "the dispatching scope must own nothing the callback allocates"
        );
        registrar.dispose();
        dispatcher.dispose();
        // Leave no dead entry behind: `releasing_dead_entries_leaves_live_siblings_alone`
        // asserts an exact release count, and would see this one too on any
        // runner that shares a thread between tests.
        MAP.with(|map| map.remove(&1));
    }

    struct Reinstall;

    impl Drop for Reinstall {
        fn drop(&mut self) {
            SLOT.with(|slot| slot.install(Rc::new(|| {})));
        }
    }

    /// The callback being displaced is user code, and its `Drop` may re-enter the
    /// registry — an `Rc<WsHandle>`-alike, or anything that deregisters itself.
    /// Dropping it inside the `borrow_mut` is a `BorrowMutError`.
    #[test]
    fn a_displaced_callback_may_reenter_the_slot_from_its_own_drop() {
        let guard = Reinstall;
        SLOT.with(|slot| {
            slot.install(Rc::new(move || {
                let _keep = &guard;
            }))
        });

        // Replacing it drops the first callback, whose `Drop` re-enters `install`.
        SLOT.with(|slot| slot.install(Rc::new(|| {})));

        assert!(
            SLOT.with(|slot| slot.is_installed()),
            "the slot must survive a displaced callback that re-enters it"
        );
        SLOT.with(|slot| slot.clear());
    }

    /// Whichever registration holds the slot now is the one that dispatches: an
    /// earlier component unmounting must not reclaim a later one's callback.
    #[test]
    fn a_later_registration_survives_the_earlier_scopes_disposal() {
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();

        let early = Scope::new();
        early.run(|| SLOT.with(|slot| slot.install(Rc::new(|| unreachable!()))));

        let late = Scope::new();
        late.run(|| {
            SLOT.with(|slot| {
                slot.install(Rc::new(move || flag.set(true)));
            })
        });

        early.dispose();
        SLOT.with(|slot| slot.dispatch(|cb| cb()));

        assert!(ran.get(), "the later registration must still dispatch");
        late.dispose();
        SLOT.with(|slot| slot.clear());
    }

    /// The sweep releases only what is dead, and reports how much.
    #[test]
    fn releasing_dead_entries_leaves_live_siblings_alone() {
        let live = Scope::new();
        live.run(|| MAP.with(|map| map.install(10, Rc::new(|| {}))));

        let dead = Scope::new();
        dead.run(|| MAP.with(|map| map.install(11, Rc::new(|| {}))));
        // Ownerless: app lifetime, must never be swept.
        MAP.with(|map| map.install(12, Rc::new(|| {})));

        dead.dispose();
        let released = MAP.with(|map| map.release_dead());

        assert_eq!(released, vec![11], "exactly the one dead entry");
        assert!(MAP.with(|map| map.contains(&10)), "the live entry stays");
        assert!(!MAP.with(|map| map.contains(&11)), "the dead entry goes");
        assert!(
            MAP.with(|map| map.contains(&12)),
            "an ownerless entry has app lifetime and must never be swept"
        );

        live.dispose();
        for key in [10, 11, 12] {
            MAP.with(|map| map.remove(&key));
        }
    }

    /// The released callbacks are user code, and their `Drop` runs *inside* the
    /// sweep — re-registering at the key being released is exactly what a
    /// restart looks like. The caller's response to a reported key is to power
    /// that hardware down, so a key a live registration reclaimed must be
    /// withheld. The keyed half of
    /// [`a_displaced_callback_may_reenter_the_slot_from_its_own_drop`].
    #[test]
    fn a_key_a_live_registration_reclaimed_during_the_sweep_is_not_reported() {
        struct Reregister;

        impl Drop for Reregister {
            fn drop(&mut self) {
                // Ownerless, so app lifetime: this one must survive the sweep.
                MAP.with(|map| map.install(13, Rc::new(|| {})));
            }
        }

        let scope = Scope::new();
        scope.run(|| {
            let guard = Reregister;
            MAP.with(|map| {
                map.install(
                    13,
                    Rc::new(move || {
                        let _keep = &guard;
                    }),
                )
            });
        });
        scope.dispose();

        assert!(
            MAP.with(|map| map.release_dead()).is_empty(),
            "a key a live registration reclaimed during the sweep must not be \
             reported as released"
        );
        assert!(
            MAP.with(|map| map.contains(&13)),
            "and that live registration must still be there"
        );
        MAP.with(|map| map.remove(&13));
    }

    /// A one-shot is removed *before* it is called, so it may register the next
    /// step of a chain — even at its own key — without a double borrow.
    #[test]
    fn a_one_shot_may_register_at_its_own_key_from_inside_its_delivery() {
        let seen = Rc::new(Cell::new(0u32));
        let first = seen.clone();
        ONCE.with(|map| {
            map.install(
                20,
                Box::new(move |v| {
                    first.set(v);
                    let second = first.clone();
                    ONCE.with(|map| map.install(20, Box::new(move |v| second.set(v))));
                }),
            )
        });

        assert_eq!(ONCE.with(|map| map.deliver(&20, 1)), Delivery::Ran);
        assert_eq!(ONCE.with(|map| map.deliver(&20, 2)), Delivery::Ran);
        assert_eq!(
            seen.get(),
            2,
            "the replacement must be installed and delivered"
        );
        assert_eq!(
            ONCE.with(|map| map.deliver(&20, 3)),
            Delivery::Unregistered,
            "a one-shot is removed on delivery"
        );
    }

    /// A one-shot whose component is gone is dropped undelivered, and says so —
    /// the caller must not log it as "nothing was registered".
    #[test]
    fn a_dead_one_shot_reports_dropped_rather_than_unregistered() {
        let scope = Scope::new();
        scope.run(|| ONCE.with(|map| map.install(21, Box::new(|_| unreachable!()))));
        scope.dispose();

        assert_eq!(ONCE.with(|map| map.deliver(&21, 1)), Delivery::Dropped);
        assert_eq!(ONCE.with(|map| map.deliver(&21, 1)), Delivery::Unregistered);
    }

    // ── ScopedList (#599) ────────────────────────────────────────────────
    //
    // **These test `dispatch` directly, and they have to.** `intent.rs` sweeps
    // with `release_dead` at the top of every drain, so by the time its
    // `dispatch` runs there is nothing dead left to prune — measured: three
    // separate mutants of `dispatch` (keep-and-call the dead, `has_live` as
    // `is_empty`, swap-removal) all survived the intent-level suite, because
    // the sweep had already removed the entry each of them needed to see.
    // A consumer's convenience is not a reason for the template's own
    // behaviour to go unpinned.

    fn record(log: &Rc<RefCell<Vec<&'static str>>>, name: &'static str) -> Rc<dyn Fn()> {
        let sink = log.clone();
        Rc::new(move || sink.borrow_mut().push(name))
    }

    /// An entry stops firing once the component that registered it is gone,
    /// and the others carry on.
    #[test]
    fn a_list_entry_stops_firing_once_its_scope_is_disposed() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        LIST.with(|list| list.install(record(&log, "live")));

        let scope = Scope::new();
        scope.run(|| LIST.with(|list| list.install(record(&log, "doomed"))));

        // Fail-first: prove the doomed one fires while its scope is alive.
        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(log.borrow().as_slice(), ["live", "doomed"]);

        log.borrow_mut().clear();
        scope.dispose();
        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(log.borrow().as_slice(), ["live"]);
        LIST.with(|list| list.clear_for_test());
    }

    /// A dispatch **removes** what it skips rather than merely stepping over
    /// it. Leaving it would re-check it on every later event and keep alive
    /// everything it captured — and nothing else in this registry would ever
    /// let go of it.
    #[test]
    fn a_dispatch_prunes_the_entry_it_skips() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::new();
        scope.run(|| LIST.with(|list| list.install(record(&log, "doomed"))));
        LIST.with(|list| list.install(record(&log, "live")));
        scope.dispose();

        assert_eq!(LIST.with(|list| list.len()), 2, "precondition: both held");
        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(
            LIST.with(|list| list.len()),
            1,
            "the dead entry must be gone after a dispatch, not merely skipped"
        );
        LIST.with(|list| list.clear_for_test());
    }

    /// `has_live` is about liveness, not emptiness — the distinction a caller
    /// that holds events back while nobody is listening depends on entirely.
    #[test]
    fn has_live_is_about_liveness_and_not_emptiness() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::new();
        scope.run(|| LIST.with(|list| list.install(record(&log, "doomed"))));
        assert!(LIST.with(|list| list.has_live()), "precondition");

        scope.dispose();
        assert_eq!(
            LIST.with(|list| list.len()),
            1,
            "precondition: the dead entry is still held, so `is_empty` is false"
        );
        assert!(
            !LIST.with(|list| list.has_live()),
            "a list of nothing but dead entries is not a listener"
        );
        LIST.with(|list| list.clear_for_test());
    }

    /// Pruning must not reorder the survivors.
    ///
    /// **Four entries, with the dead one second — and three is the fixed
    /// point.** The cheap prune is a swap-removal, which moves the *last*
    /// element into the hole; with `[live, dead, live]` the last element is
    /// already the one that belongs in the hole, so the swap and an
    /// order-preserving removal agree and the mutant survives. Measured: the
    /// three-entry version of this fixture passed against a `swap_remove`
    /// prune. With a fourth, the swap answers `first, fourth, third`.
    #[test]
    fn a_prune_during_dispatch_preserves_registration_order() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        LIST.with(|list| list.install(record(&log, "first")));

        let scope = Scope::new();
        scope.run(|| LIST.with(|list| list.install(record(&log, "second"))));
        LIST.with(|list| list.install(record(&log, "third")));
        LIST.with(|list| list.install(record(&log, "fourth")));
        scope.dispose();

        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(
            log.borrow().as_slice(),
            ["first", "third", "fourth"],
            "removing the second entry must not bring the last one forward"
        );
        LIST.with(|list| list.clear_for_test());
    }

    /// What a list callback allocates belongs to the component that registered
    /// it, not to whatever scope the dispatch is nested inside — the asymmetry
    /// issue #183 records, and the reason `dispatch` runs each entry inside its
    /// own `Registrant`.
    #[test]
    fn a_list_callback_is_attributed_to_its_registering_scope() {
        let registrar = Scope::new();
        registrar.run(|| LIST.with(|list| list.install(allocate_a_signal())));

        let dispatcher = Scope::new();
        let registrar_before = registrar.owned_counts().signals;
        let dispatcher_before = dispatcher.owned_counts().signals;
        dispatcher.run(|| {
            LIST.with(|list| list.dispatch(|cb| cb()));
        });

        assert_eq!(
            registrar.owned_counts().signals,
            registrar_before + 1,
            "the signal belongs to the component that registered the callback"
        );
        assert_eq!(
            dispatcher.owned_counts().signals,
            dispatcher_before,
            "and not to the one that happened to be ambient at dispatch"
        );
        registrar.dispose();
        dispatcher.dispose();
        LIST.with(|list| list.clear_for_test());
    }

    /// An ownerless registration keeps app lifetime — the rule that stops the
    /// prune overshooting into every handler registered from `android_main`.
    #[test]
    fn an_ownerless_list_entry_keeps_app_lifetime() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        // Deliberately not inside a `Scope::run`.
        LIST.with(|list| list.install(record(&log, "app lifetime")));

        let unrelated = Scope::new();
        unrelated.run(|| {});
        unrelated.dispose();

        assert_eq!(LIST.with(|list| list.release_dead()), 0);
        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(log.borrow().as_slice(), ["app lifetime"]);
        LIST.with(|list| list.clear_for_test());
    }

    /// The sweep frees the dead and leaves the living, for the case where no
    /// event ever arrives to prune anything.
    #[test]
    fn releasing_dead_list_entries_leaves_live_siblings_alone() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        LIST.with(|list| list.install(record(&log, "live")));
        let scope = Scope::new();
        scope.run(|| LIST.with(|list| list.install(record(&log, "doomed"))));
        scope.dispose();

        assert_eq!(LIST.with(|list| list.release_dead()), 1, "one freed");
        assert_eq!(LIST.with(|list| list.len()), 1, "and one kept");
        LIST.with(|list| list.dispatch(|cb| cb()));
        assert_eq!(log.borrow().as_slice(), ["live"]);
        LIST.with(|list| list.clear_for_test());
    }
}
