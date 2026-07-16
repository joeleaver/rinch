//! Web (wasm32) [`Store`] backed by IndexedDB.
//!
//! One [`IdbStore`] owns an open `IdbDatabase` with a single object store keyed
//! by string; values are stored as `Uint8Array`. IndexedDB is deliberately
//! chosen over OPFS: its `web-sys` bindings are **stable**, whereas OPFS write
//! handles (`FileSystemFileHandle` / `FileSystemWritableFileStream`) sit behind
//! the `web_sys_unstable_apis` cfg — and enabling that cfg globally fails to
//! compile rinch (`rinch/src/render_surface.rs`'s `put_image_data` takes `f64`
//! where the unstable signature wants `i32`). IndexedDB ships this crate today
//! with no change to rinch's render path.
//!
//! ## Deferred: OPFS via a Worker
//!
//! The higher-performance backend — OPFS with `createSyncAccessHandle` in a
//! dedicated Worker (synchronous, LSM-friendly reads/writes) — is a future
//! optimization, not this crate. It needs the `web_sys_unstable_apis` cfg to
//! compile (so the one-line `render_surface.rs` fix must land first) plus a wasm
//! worker/threading setup. IndexedDB is production-viable for the keyed-blob
//! workload here (per-doc snapshots + change logs), so OPFS is intentionally out
//! of scope.
//!
//! ## Durability
//!
//! `put` / `delete` resolve only when their IndexedDB **transaction completes**
//! (not merely when the request succeeds), which is the point at which the write
//! is durable. `get` / `list` resolve on request success.
//!
//! Everything runs on the single web thread; the returned futures are `!Send`
//! (they hold JS handles) and are driven by the consumer's `spawn_local`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Event, IdbDatabase, IdbObjectStore, IdbOpenDbRequest, IdbRequest, IdbTransaction,
    IdbTransactionMode,
};

use crate::{StorageError, StorageFuture, StorageResult, Store};

/// A [`Store`] backed by an IndexedDB object store.
///
/// Cheap to [`clone`](Clone) (the open database is behind an `Rc`); a clone talks
/// to the same object store.
#[derive(Clone)]
pub struct IdbStore {
    inner: Rc<Inner>,
}

struct Inner {
    db: IdbDatabase,
    store: String,
}

impl IdbStore {
    /// Open (creating if needed) database `db_name` with a single object store
    /// `store_name`. Awaitable: opening IndexedDB is asynchronous.
    pub async fn open(db_name: &str, store_name: &str) -> StorageResult<Self> {
        let factory = web_sys::window()
            .ok_or_else(|| StorageError::Unavailable("no global `window`".to_string()))?
            .indexed_db()
            .map_err(|e| StorageError::Backend(js_err(&e)))?
            .ok_or_else(|| StorageError::Unavailable("IndexedDB not available".to_string()))?;

        let open_req: IdbOpenDbRequest = factory
            .open(db_name)
            .map_err(|e| StorageError::Backend(js_err(&e)))?;

        // Create the object store on first open (version-change upgrade). Capture
        // the request itself so we don't need the EventTarget bindings.
        let req_for_upgrade = open_req.clone();
        let store_owned = store_name.to_string();
        let on_upgrade = Closure::once_into_js(move |_e: Event| {
            if let Ok(db_val) = req_for_upgrade.result() {
                let db: IdbDatabase = db_val.unchecked_into();
                if !db.object_store_names().contains(&store_owned) {
                    let _ = db.create_object_store(&store_owned);
                }
            }
        });
        open_req.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));

        let db_val = request_result(open_req.unchecked_into())
            .await
            .map_err(|e| StorageError::Backend(js_err(&e)))?;
        let db: IdbDatabase = db_val.unchecked_into();

        Ok(Self {
            inner: Rc::new(Inner {
                db,
                store: store_name.to_string(),
            }),
        })
    }

    fn object_store(&self, mode: IdbTransactionMode) -> StorageResult<(IdbTransaction, IdbObjectStore)> {
        let tx = self
            .inner
            .db
            .transaction_with_str_and_mode(&self.inner.store, mode)
            .map_err(|e| StorageError::Backend(js_err(&e)))?;
        let os = tx
            .object_store(&self.inner.store)
            .map_err(|e| StorageError::Backend(js_err(&e)))?;
        Ok((tx, os))
    }
}

impl Store for IdbStore {
    fn get(&self, key: &str) -> StorageFuture<Option<Vec<u8>>> {
        let this = self.clone();
        let key = key.to_string();
        Box::pin(async move {
            let (_tx, os) = this.object_store(IdbTransactionMode::Readonly)?;
            let req = os
                .get(&JsValue::from_str(&key))
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            let val = request_result(req)
                .await
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            if val.is_undefined() || val.is_null() {
                Ok(None)
            } else {
                Ok(Some(js_sys::Uint8Array::new(&val).to_vec()))
            }
        })
    }

    fn put(&self, key: &str, value: &[u8]) -> StorageFuture<()> {
        let this = self.clone();
        let key = key.to_string();
        let bytes = js_sys::Uint8Array::from(value);
        Box::pin(async move {
            let (tx, os) = this.object_store(IdbTransactionMode::Readwrite)?;
            os.put_with_key(&bytes.into(), &JsValue::from_str(&key))
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            // Resolve on transaction completion — that is when the write is durable.
            transaction_complete(tx)
                .await
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            Ok(())
        })
    }

    fn delete(&self, key: &str) -> StorageFuture<()> {
        let this = self.clone();
        let key = key.to_string();
        Box::pin(async move {
            let (tx, os) = this.object_store(IdbTransactionMode::Readwrite)?;
            os.delete(&JsValue::from_str(&key))
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            transaction_complete(tx)
                .await
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            Ok(())
        })
    }

    fn list(&self, prefix: &str) -> StorageFuture<Vec<String>> {
        let this = self.clone();
        let prefix = prefix.to_string();
        Box::pin(async move {
            let (_tx, os) = this.object_store(IdbTransactionMode::Readonly)?;
            let req = os
                .get_all_keys()
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            let val = request_result(req)
                .await
                .map_err(|e| StorageError::Backend(js_err(&e)))?;
            let arr = js_sys::Array::from(&val);
            let mut keys = Vec::new();
            for k in arr.iter() {
                if let Some(s) = k.as_string()
                    && s.starts_with(&prefix)
                {
                    keys.push(s);
                }
            }
            Ok(keys)
        })
    }
}

/// Shared state for [`Settle`]: the resolved value (once), plus the waker to
/// notify when it lands.
struct SettleState {
    done: Option<Result<JsValue, JsValue>>,
    waker: Option<Waker>,
}

/// A future that resolves when a JS event fires — the bridge from IndexedDB's
/// event-driven `IdbRequest` / `IdbTransaction` to Rust's `async`. It owns the
/// event-handler closures so they stay alive until the event lands.
struct Settle {
    state: Rc<RefCell<SettleState>>,
    _keep: Vec<Closure<dyn FnMut(Event)>>,
}

impl Future for Settle {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut st = self.state.borrow_mut();
        match st.done.take() {
            Some(res) => Poll::Ready(res),
            None => {
                st.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

fn settle(state: &Rc<RefCell<SettleState>>, res: Result<JsValue, JsValue>) {
    let mut st = state.borrow_mut();
    if st.done.is_none() {
        st.done = Some(res);
    }
    if let Some(w) = st.waker.take() {
        w.wake();
    }
}

/// Resolve when `req` succeeds (with its `result`) or errors.
fn request_result(req: IdbRequest) -> Settle {
    let state = Rc::new(RefCell::new(SettleState {
        done: None,
        waker: None,
    }));
    let mut keep = Vec::new();

    let s = state.clone();
    let r = req.clone();
    let on_success = Closure::wrap(Box::new(move |_e: Event| {
        let value = r.result().unwrap_or(JsValue::UNDEFINED);
        settle(&s, Ok(value));
    }) as Box<dyn FnMut(Event)>);
    req.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
    keep.push(on_success);

    let s = state.clone();
    let r = req.clone();
    let on_error = Closure::wrap(Box::new(move |_e: Event| {
        settle(&s, Err(request_error(&r)));
    }) as Box<dyn FnMut(Event)>);
    req.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    keep.push(on_error);

    Settle { state, _keep: keep }
}

/// Resolve when transaction `tx` completes (durable) or errors/aborts.
fn transaction_complete(tx: IdbTransaction) -> Settle {
    let state = Rc::new(RefCell::new(SettleState {
        done: None,
        waker: None,
    }));
    let mut keep = Vec::new();

    let s = state.clone();
    let on_complete = Closure::wrap(Box::new(move |_e: Event| {
        settle(&s, Ok(JsValue::UNDEFINED));
    }) as Box<dyn FnMut(Event)>);
    tx.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
    keep.push(on_complete);

    let s = state.clone();
    let on_error = Closure::wrap(Box::new(move |_e: Event| {
        settle(&s, Err(JsValue::from_str("indexeddb transaction failed")));
    }) as Box<dyn FnMut(Event)>);
    tx.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    keep.push(on_error);

    let s = state.clone();
    let on_abort = Closure::wrap(Box::new(move |_e: Event| {
        settle(&s, Err(JsValue::from_str("indexeddb transaction aborted")));
    }) as Box<dyn FnMut(Event)>);
    tx.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
    keep.push(on_abort);

    Settle { state, _keep: keep }
}

/// Best-effort extraction of an `IdbRequest`'s error into a `JsValue`.
fn request_error(req: &IdbRequest) -> JsValue {
    match req.error() {
        Ok(Some(dom_exception)) => dom_exception.into(),
        _ => JsValue::from_str("indexeddb request failed"),
    }
}

/// Render a `JsValue` error into a debug string (matches rinch-http's style).
fn js_err(e: &JsValue) -> String {
    format!("{e:?}")
}
