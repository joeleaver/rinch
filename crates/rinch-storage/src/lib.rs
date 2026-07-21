//! Cross-platform durable key/blob persistence for rinch.
//!
//! `rinch-storage` is the local-storage seam of the offline-first architecture:
//! one small [`Store`] trait — keyed byte-blob get/put/delete/list — with a
//! backend per target, selected at compile time exactly like [`rinch-http`]:
//!
//! - **Native** (`cfg(not(target_arch = "wasm32"))`): [`FsStore`], a directory
//!   of blob files. Writes are atomic (temp file + rename) so a crash mid-write
//!   never corrupts a value.
//! - **Web** (`cfg(target_arch = "wasm32")`): [`IdbStore`], one IndexedDB object
//!   store keyed by string, values as `Uint8Array`. IndexedDB is **stable** in
//!   `web-sys` (no `web_sys_unstable_apis` cfg), so this ships today without
//!   touching rinch's render path — see the module docs on `wasm` for why OPFS
//!   is deferred.
//!
//! The store trades in **opaque bytes**. It has no knowledge of any encoding,
//! schema, or serialization format — the consumer owns that and hands the store
//! the bytes to persist. Related values are kept apart by **key convention**
//! rather than by separate stores, made ergonomic by [`Namespace`] (a
//! prefix-scoped view that is itself a `Store`).
//!
//! # Async model
//!
//! Every operation returns a [`StorageFuture`] — a boxed, **non-`Send`** future
//! resolved by the consumer's executor (`spawn_local` on web; any local block-on
//! on native). Futures (rather than [`rinch-http`]'s callbacks) are the right
//! shape here because storage work is naturally *sequential and composable*
//! (`get` then `put`), and the web backend (IndexedDB) is inherently an async,
//! event-driven API. The futures are `!Send` on purpose: the web backend holds
//! JS handles (`IdbDatabase`, …) that are `!Send`, and the trait must not force a
//! bound the web target cannot honor. The native futures could be `Send` but are
//! not required to be, keeping one trait for both targets.
//!
//! Each returned future is `'static`: an operation clones the bytes/key and its
//! backend handle (a cheap `Arc`/`Rc`) into the future, so it borrows nothing
//! and can be spawned freely without lifetime plumbing.
//!
//! [`rinch-http`]: https://docs.rs/rinch-http
//!
//! # Example
//!
//! ```ignore
//! use rinch_storage::{Store, Namespace};
//! # async fn go(store: impl Store) {
//! // One physical store, carved into per-entity namespaces so unrelated values
//! // never collide.
//! let entity = Namespace::new(store, "entity:abc123/");
//! entity.put("state", &state_bytes).await.unwrap();
//! entity.put("log/00000001", &entry_bytes).await.unwrap();
//! let restored = entity.get("state").await.unwrap();      // Some(bytes)
//! let entries = entity.list("log/").await.unwrap();       // ["log/00000001"]
//! # }
//! ```

mod error;

pub use error::StorageError;

use std::future::Future;
use std::pin::Pin;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::FsStore;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::IdbStore;

/// The result of a [`Store`] operation.
pub type StorageResult<T> = Result<T, StorageError>;

/// The future returned by every [`Store`] operation.
///
/// Boxed and **not `Send`** (see the crate-level "Async model" docs), and
/// `'static` — it captures everything it needs by value, so it can be handed to
/// `spawn_local` / an executor without borrowing the store.
pub type StorageFuture<T> = Pin<Box<dyn Future<Output = StorageResult<T>>>>;

/// A durable key → byte-blob store.
///
/// The whole surface: four orthogonal operations over opaque `Vec<u8>` values,
/// keyed by opaque UTF-8 strings. Deliberately minimal — no transactions, no
/// value typing, no CRDT awareness. Namespacing is a **key convention**: build
/// hierarchical keys (`"entity:{id}/state"`), or wrap the store in a
/// [`Namespace`] for a prefix-scoped view. Enumeration is by [`list`](Store::list)
/// with a key prefix.
///
/// The trait is object-safe: `Box<dyn Store>` / `Rc<dyn Store>` work, so a
/// consumer can hold a backend behind a trait object without leaking a generic
/// through its state.
pub trait Store {
    /// Read the bytes stored at `key`, or `None` if the key is absent.
    fn get(&self, key: &str) -> StorageFuture<Option<Vec<u8>>>;

    /// Durably store `value` at `key`, replacing any existing value.
    fn put(&self, key: &str, value: &[u8]) -> StorageFuture<()>;

    /// Remove `key`. Absent key is not an error (idempotent).
    fn delete(&self, key: &str) -> StorageFuture<()>;

    /// List every key that starts with `prefix` (pass `""` for all keys).
    ///
    /// Order is unspecified. The returned keys are the **full** keys as stored
    /// (they include `prefix`), so a caller can act on them directly.
    fn list(&self, prefix: &str) -> StorageFuture<Vec<String>>;
}

/// A prefix-scoped view over another [`Store`].
///
/// Every key is transparently prefixed with `prefix` before hitting the inner
/// store, so a consumer can carve one physical store into per-entity (or
/// per-concern) partitions without those keys ever colliding. `Namespace` is
/// itself a [`Store`], so it composes: pass it anywhere a `Store` is wanted, or
/// nest it with [`namespace`](Namespace::namespace).
///
/// [`list`](Store::list) is prefix-*relative*: it strips the namespace prefix
/// back off the returned keys, so a scoped caller sees the keys it wrote, not the
/// physical ones.
///
/// The `prefix` is prepended verbatim — include your own separator, e.g.
/// `Namespace::new(store, "entity:abc/")`.
pub struct Namespace<S> {
    inner: S,
    prefix: String,
}

impl<S: Store> Namespace<S> {
    /// Scope `inner` under `prefix` (prepended verbatim to every key).
    pub fn new(inner: S, prefix: impl Into<String>) -> Self {
        Self {
            inner,
            prefix: prefix.into(),
        }
    }

    /// Nest a further scope inside this one: keys become `self.prefix + sub + key`.
    pub fn namespace(self, sub: impl Into<String>) -> Namespace<Namespace<S>> {
        let sub = sub.into();
        Namespace::new(self, sub)
    }

    fn join(&self, key: &str) -> String {
        let mut k = String::with_capacity(self.prefix.len() + key.len());
        k.push_str(&self.prefix);
        k.push_str(key);
        k
    }
}

impl<S: Store> Store for Namespace<S> {
    fn get(&self, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        self.inner.get(&self.join(key))
    }

    fn put(&self, key: &str, value: &[u8]) -> StorageFuture<()> {
        self.inner.put(&self.join(key), value)
    }

    fn delete(&self, key: &str) -> StorageFuture<()> {
        self.inner.delete(&self.join(key))
    }

    fn list(&self, prefix: &str) -> StorageFuture<Vec<String>> {
        let our_prefix = self.prefix.clone();
        let fut = self.inner.list(&self.join(prefix));
        Box::pin(async move {
            let keys = fut.await?;
            Ok(keys
                .into_iter()
                .map(|k| match k.strip_prefix(&our_prefix) {
                    Some(rest) => rest.to_string(),
                    None => k,
                })
                .collect())
        })
    }
}
