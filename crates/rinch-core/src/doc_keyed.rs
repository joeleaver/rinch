//! Registries keyed by a document, and the one sweep that forgets a document
//! in all of them when it dies (issue #2797).
//!
//! Much of rinch keeps state per document in thread-locals: app-key
//! listeners, the dismiss stack, focus targets and claims, pending focus and
//! selection requests, the document-scoped interceptor slots. Most entries go
//! with the scope that made them, but an entry made where no scope will dispose
//! it (in an `on_cleanup` that runs while the document is being disposed, or
//! by code that names the document's key outright) used to outlive the
//! document for the life of the thread.
//!
//! A registry declared with [`doc_keyed!`](crate::doc_keyed!) joins this
//! thread's inventory the first time it is written on this thread, and
//! [`forget_doc`] takes a document's entries out of every registry in the
//! inventory. It takes and drops one registry at a time, over a copy of the
//! inventory taken when it starts. So an entry registered again for the same
//! document during the sweep (from a swept entry's drop) survives it when its
//! registry was already swept (that same registry, or one earlier in the
//! inventory) or joined the inventory during the sweep (it is not in the
//! copy); one made in a registry still to come is taken in the same sweep.
//! It runs once per document, when the document dies:
//!
//! - desktop: at the end of `RinchApp::dispose_document`, which every window's
//!   close runs (a second window, DevTools, the main window closed while the
//!   app runs on) and `rinch::testing`'s `TestApp` too;
//! - an app that is dropped undisposed (an embed `RinchContext`, Android's
//!   app, the desktop main window at the end of the event loop): `Drop for
//!   RinchApp` disposes it, and so forgets it, there;
//! - the web: `rinch_web::RootHandle::unmount`, after the root's scope is
//!   disposed and its tree removed. A root made with `rinch_web::mount` is
//!   never unmounted, so its document is never swept. The unmount's disposal
//!   runs as the document's code, the one place the web marks a document, so
//!   the sweep takes what its cleanups register. An ambient registration a
//!   root's ordinary code makes (an app key, an interceptor) is no document's
//!   on the web and goes with the scope that owns it, as before.
//!
//! # Cost
//!
//! A registry here is a `const`-initialised thread-local, as before. Reading
//! it ([`DocKeyed::with`]) is a `LocalKey::with` and a `RefCell` borrow, as
//! before: no check. Writing it ([`DocKeyed::with_mut`]) reads one
//! `Cell<bool>` to see whether this thread's inventory has it yet. The sweep is
//! one walk over the registries this thread has written, per dying document.
//!
//! # Threads
//!
//! The inventory is per thread. A document lives on one thread (its app, its
//! `Rc`s, its reactive runtime), so its entries exist only in that thread's
//! instances of these thread-locals, and its sweep runs there. A process-wide
//! inventory would walk registries other threads used, creating empty
//! instances of them on this thread for nothing.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::thread::LocalKey;

/// What a registry took out for a document. The sweep drops it after the
/// registry's borrow has ended, so the drop of a taken callback's captures,
/// which is app code, may use the registry again.
pub type Removed = Vec<Box<dyn Any>>;

/// A registry that keeps entries for documents and can forget one document's.
pub trait ForgetDoc: 'static {
    /// Take every entry of document `doc` out, handing them back to be
    /// dropped once the registry is no longer borrowed.
    fn take_doc(&mut self, doc: u64) -> Removed;

    /// How many entries document `doc` has here.
    fn entries_for_doc(&self, doc: u64) -> usize;
}

/// A map whose whole key is a document: the registry of anything a document
/// has at most one of.
#[derive(Debug)]
pub struct DocMap<V> {
    map: BTreeMap<u64, V>,
}

impl<V> DocMap<V> {
    /// An empty map, `const` for a thread-local initializer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Document `doc`'s value.
    #[must_use]
    pub fn get(&self, doc: u64) -> Option<&V> {
        self.map.get(&doc)
    }

    /// Document `doc`'s value, to change.
    pub fn get_mut(&mut self, doc: u64) -> Option<&mut V> {
        self.map.get_mut(&doc)
    }

    /// Document `doc`'s value, made with `make` if it has none.
    pub fn get_or_insert_with(&mut self, doc: u64, make: impl FnOnce() -> V) -> &mut V {
        self.map.entry(doc).or_insert_with(make)
    }

    /// Set document `doc`'s value, answering the one it replaced.
    pub fn insert(&mut self, doc: u64, value: V) -> Option<V> {
        self.map.insert(doc, value)
    }

    /// Take document `doc`'s value.
    pub fn remove(&mut self, doc: u64) -> Option<V> {
        self.map.remove(&doc)
    }

    /// Whether document `doc` has a value.
    #[must_use]
    pub fn contains(&self, doc: u64) -> bool {
        self.map.contains_key(&doc)
    }

    /// Whether no document has a value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Every document's value, by document.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &V)> {
        self.map.iter().map(|(doc, value)| (*doc, value))
    }
}

impl<V> Default for DocMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: 'static> ForgetDoc for DocMap<V> {
    fn take_doc(&mut self, doc: u64) -> Removed {
        self.map
            .remove(&doc)
            .map(|value| Box::new(value) as Box<dyn Any>)
            .into_iter()
            .collect()
    }

    fn entries_for_doc(&self, doc: u64) -> usize {
        usize::from(self.map.contains_key(&doc))
    }
}

/// A list whose entries each belong to a document, or to none: the registry
/// of anything kept in an order across documents (dispatch order,
/// registration order, posting order), which a map by document would lose.
/// It derefs to its `Vec`.
pub struct DocList<E> {
    items: Vec<E>,
    doc_of: fn(&E) -> Option<u64>,
}

impl<E> DocList<E> {
    /// An empty list whose entries' documents `doc_of` reads; `const` for a
    /// thread-local initializer.
    #[must_use]
    pub const fn new(doc_of: fn(&E) -> Option<u64>) -> Self {
        Self {
            items: Vec::new(),
            doc_of,
        }
    }
}

impl<E> std::ops::Deref for DocList<E> {
    type Target = Vec<E>;
    fn deref(&self) -> &Vec<E> {
        &self.items
    }
}

impl<E> std::ops::DerefMut for DocList<E> {
    fn deref_mut(&mut self) -> &mut Vec<E> {
        &mut self.items
    }
}

impl<E: 'static> ForgetDoc for DocList<E> {
    fn take_doc(&mut self, doc: u64) -> Removed {
        let doc_of = self.doc_of;
        let (gone, kept): (Vec<E>, Vec<E>) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|e| doc_of(e) == Some(doc));
        self.items = kept;
        gone.into_iter()
            .map(|e| Box::new(e) as Box<dyn Any>)
            .collect()
    }

    fn entries_for_doc(&self, doc: u64) -> usize {
        self.items
            .iter()
            .filter(|e| (self.doc_of)(e) == Some(doc))
            .count()
    }
}

/// A [`doc_keyed!`](crate::doc_keyed!) registry's thread-local storage. Not
/// used directly.
#[doc(hidden)]
pub struct DocKeyedCell<T> {
    /// Whether this thread's inventory has the registry.
    registered: Cell<bool>,
    value: RefCell<T>,
}

impl<T> DocKeyedCell<T> {
    #[doc(hidden)]
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self {
            registered: Cell::new(false),
            value: RefCell::new(value),
        }
    }
}

/// A registry keyed by document, declared with
/// [`doc_keyed!`](crate::doc_keyed!): a thread-local that [`forget_doc`]
/// sweeps.
pub struct DocKeyed<T: 'static> {
    key: &'static LocalKey<DocKeyedCell<T>>,
    name: &'static str,
}

impl<T: ForgetDoc> DocKeyed<T> {
    /// The registry over `key`, named `name` (its static's path) in
    /// [`doc_entries`].
    #[doc(hidden)]
    #[must_use]
    pub const fn new(key: &'static LocalKey<DocKeyedCell<T>>, name: &'static str) -> Self {
        Self { key, name }
    }

    /// Read this thread's registry.
    ///
    /// # Panics
    ///
    /// As `LocalKey::with` and `RefCell::borrow`: during the thread's
    /// destruction, or while `with_mut` holds it.
    pub fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.key.with(|cell| f(&cell.value.borrow()))
    }

    /// Change this thread's registry. The first time on a thread, the
    /// registry joins that thread's inventory, which [`forget_doc`] walks.
    /// While the thread is ending and its inventory is gone (a first write
    /// from another thread-local's `Drop`), the write is made and the
    /// registry joins nothing: no document of the thread will be swept again.
    ///
    /// # Panics
    ///
    /// As `LocalKey::with` and `RefCell::borrow_mut`: during the thread's
    /// destruction, or while the registry is borrowed.
    pub fn with_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.key.with(|cell| {
            if !cell.registered.get() {
                let joined = INVENTORY
                    .try_with(|inventory| inventory.borrow_mut().push(self))
                    .is_ok();
                cell.registered.set(joined);
            }
            f(&mut cell.value.borrow_mut())
        })
    }

    /// [`Self::with`], or `None` when the registry cannot be reached (the
    /// thread is ending) or is borrowed mutably: for a release path, such as
    /// a handle's `Drop`, that must not panic.
    pub fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.key
            .try_with(|cell| cell.value.try_borrow().ok().map(|value| f(&value)))
            .ok()
            .flatten()
    }

    /// Whether this thread's registry is borrowed right now (`false` once the
    /// thread is ending and it cannot be reached): what tells a failed
    /// [`Self::try_with_mut`] in a release path that it is a bug rather than
    /// the thread's end.
    pub fn is_borrowed(&'static self) -> bool {
        self.key
            .try_with(|cell| cell.value.try_borrow_mut().is_err())
            .unwrap_or(false)
    }

    /// [`Self::with_mut`], or `None` when the registry cannot be reached (the
    /// thread is ending) or is borrowed: for a release path, such as a
    /// handle's `Drop`, that must not panic. A release only takes entries out,
    /// so it does not join the inventory: a registry this thread never wrote
    /// has nothing to release.
    pub fn try_with_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.key
            .try_with(|cell| {
                cell.value
                    .try_borrow_mut()
                    .ok()
                    .map(|mut value| f(&mut value))
            })
            .ok()
            .flatten()
    }
}

/// What a sweep of one registry came to.
enum Taken {
    /// The document's entries, to drop now the registry is let go.
    Entries(Removed),
    /// The thread is ending and the registry is gone: nothing left to take.
    Gone,
    /// The registry is borrowed: its entries for the document stay. A bug in
    /// the caller (a sweep from inside a registry's `with`), reported.
    Borrowed,
}

/// What the inventory needs of a registry, whatever its type.
trait Swept {
    /// Take document `doc`'s entries.
    fn take(&'static self, doc: u64) -> Taken;
    /// Document `doc`'s entries here.
    fn count(&'static self, doc: u64) -> usize;
    /// The registry's name.
    fn name(&self) -> &'static str;
}

impl<T: ForgetDoc> Swept for DocKeyed<T> {
    fn take(&'static self, doc: u64) -> Taken {
        self.key
            .try_with(|cell| match cell.value.try_borrow_mut() {
                Ok(mut value) => Taken::Entries(value.take_doc(doc)),
                Err(_) => Taken::Borrowed,
            })
            .unwrap_or(Taken::Gone)
    }

    fn count(&'static self, doc: u64) -> usize {
        self.key
            .try_with(|cell| {
                cell.value
                    .try_borrow()
                    .map_or(0, |value| value.entries_for_doc(doc))
            })
            .unwrap_or(0)
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

thread_local! {
    /// The registries written on this thread, each once, in the order first
    /// written.
    static INVENTORY: RefCell<Vec<&'static dyn Swept>> = const { RefCell::new(Vec::new()) };
}

/// The registries in this thread's inventory, copied out, so a registry
/// written while the copy is walked (it joins the inventory) cannot disturb
/// the walk.
fn inventory() -> Vec<&'static dyn Swept> {
    INVENTORY
        .try_with(|inventory| inventory.borrow().clone())
        .unwrap_or_default()
}

/// Forget document `doc` in every registry this thread has written: take its
/// entries out of each, then drop them, each once its registry is no longer
/// borrowed. Idempotent: a second call finds nothing.
///
/// Called when a document dies (see the [module documentation](self) for
/// where, on each target). A document key of `0` is no document and is
/// ignored.
///
/// # Panics
///
/// In a debug build, after sweeping every other registry, when one was
/// borrowed and so could not be swept: a sweep from inside a registry's
/// `with` is a bug. A release build logs it (`tracing::warn!`, naming the
/// registry and the document) and goes on.
pub fn forget_doc(doc: u64) {
    if doc == 0 {
        return;
    }
    let mut borrowed = Vec::new();
    for registry in inventory() {
        match registry.take(doc) {
            // Dropped here, after `take` has let go of the registry.
            Taken::Entries(removed) => drop(removed),
            // The thread is ending: the registry and its entries are gone.
            Taken::Gone => {}
            // The rest are still swept; this one is reported below.
            Taken::Borrowed => borrowed.push(registry.name()),
        }
    }
    // The sweep runs where a document dies (its teardown), never from inside
    // a registry's `with`: a registry borrowed here is a bug in the caller,
    // and its entries for the document would stay for the thread's life, the
    // leak this sweep exists to end. Said out loud, not skipped silently.
    for name in borrowed {
        tracing::warn!("{name} was borrowed while document {doc} was swept; its entries leaked");
        debug_assert!(
            false,
            "{name} was borrowed while document {doc} was swept; its entries leaked"
        );
    }
}

/// Every registry in this thread's inventory with document `doc`'s entries
/// there, by name (the path of its static, such as
/// `rinch_core::events::app_key::LISTENERS`): what a test asserts is all zero once the document is gone.
#[doc(hidden)]
#[must_use]
pub fn doc_entries(doc: u64) -> Vec<(&'static str, usize)> {
    inventory()
        .into_iter()
        .map(|registry| (registry.name(), registry.count(doc)))
        .collect()
}

/// Declare thread-local registries keyed by document, which [`forget_doc`]
/// sweeps. Each is a [`DocKeyed`] over a `const`-initialised thread-local; the
/// type must implement [`ForgetDoc`] ([`DocMap`], [`DocList`], or a
/// registry's own type) and the initializer must be `const`.
///
/// ```
/// rinch_core::doc_keyed! {
///     /// Each document's zoom.
///     static ZOOM: rinch_core::DocMap<f32> = rinch_core::DocMap::new();
/// }
/// ZOOM.with_mut(|zoom| zoom.insert(7, 1.5));
/// assert_eq!(ZOOM.with(|zoom| zoom.get(7).copied()), Some(1.5));
/// rinch_core::forget_doc(7);
/// assert_eq!(ZOOM.with(|zoom| zoom.get(7).copied()), None);
/// ```
#[macro_export]
macro_rules! doc_keyed {
    ($($(#[$attr:meta])* $vis:vis static $name:ident: $ty:ty = $init:expr;)+) => {
        $(
            $(#[$attr])*
            $vis static $name: $crate::doc_keyed::DocKeyed<$ty> = {
                ::std::thread_local! {
                    static CELL: $crate::doc_keyed::DocKeyedCell<$ty> =
                        const { $crate::doc_keyed::DocKeyedCell::new($init) };
                }
                $crate::doc_keyed::DocKeyed::new(
                    &CELL,
                    ::std::concat!(::std::module_path!(), "::", ::std::stringify!($name)),
                )
            };
        )+
    };
}

#[cfg(test)]
#[path = "doc_keyed_tests.rs"]
mod tests;
