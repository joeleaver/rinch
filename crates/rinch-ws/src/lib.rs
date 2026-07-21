//! Cross-platform, callback-based WebSocket client for rinch.
//!
//! One [`connect`] function works on both targets and returns a [`WsHandle`] on
//! which the consumer registers callbacks and issues sends:
//!
//! - **Native** (`cfg(not(target_arch = "wasm32"))`): a synchronous
//!   [`tungstenite`] client runs its read/write loop on a spawned `std::thread`.
//! - **Web** (`cfg(target_arch = "wasm32")`): a browser [`web_sys::WebSocket`]
//!   runs on the single web thread.
//!
//! On **both** platforms every callback (`on_open`, `on_message`, `on_close`,
//! `on_error`) is invoked on the main (UI) thread, so consumers can update rinch
//! [`Signal`](rinch_core::Signal)s directly with `.set()` from inside them. On
//! native this is arranged by hopping each event back to the main thread via
//! [`rinch_core::run_on_main_thread`] — the same transport `rinch-http` uses for
//! its `fetch` result — where it is dispatched into a main-thread-local registry
//! of the (`!Send`) callbacks. Only the connection id and the `Send` event
//! payload cross the thread boundary; the callbacks never leave the main thread,
//! so they need not be `Send` and may capture `!Send` UI state (`Rc`, `Signal`,
//! an editor handle). On web the callbacks already run on the UI thread.
//!
//! Unlike `rinch-http`'s one-shot `fetch` (a `FnOnce` parked with
//! [`park_main_callback`](rinch_core::park_main_callback)), a WebSocket is a
//! long-lived connection that fires its callbacks repeatedly, so `rinch-ws` keeps
//! its own persistent registry of `FnMut` callbacks keyed by connection id rather
//! than parking a single continuation.
//!
//! # Threading
//!
//! [`connect`] must be called on the main (UI) thread — that is where the
//! callback registry lives and where events are delivered. On native, delivery
//! relies on the rinch runtime's cross-thread dispatcher (installed at startup);
//! outside a running rinch app you must register one yourself (see the crate's
//! integration test).
//!
//! # URLs
//!
//! The URL must be absolute and use the `ws://` or `wss://` scheme. There is no
//! `base_url` notion (unlike relative HTTP fetches on web): a browser
//! `WebSocket` already requires an absolute URL, and the native client has no
//! page origin to resolve against.
//!
//! # Example
//!
//! ```ignore
//! use rinch_ws::{connect, WsMessage};
//!
//! let ws = connect("ws://127.0.0.1:3000/feed")?;
//! ws.on_open(|| { /* connection is up */ })
//!   .on_message(|msg| match msg {
//!       WsMessage::Text(t) => { /* handle t */ }
//!       WsMessage::Binary(b) => { /* handle b */ }
//!   })
//!   .on_close(|c| { /* c.code, c.reason */ })
//!   .on_error(|e| { /* e: WsError */ });
//!
//! ws.send_text("hello");
//! // Dropping `ws` closes the connection.
//! ```

mod error;

pub use error::WsError;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as backend;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// A message received over a WebSocket, or ready to be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    /// A UTF-8 text frame.
    Text(String),
    /// A binary frame.
    Binary(Vec<u8>),
}

impl WsMessage {
    /// The text of a [`WsMessage::Text`], or `None` for a binary frame.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            WsMessage::Text(t) => Some(t),
            WsMessage::Binary(_) => None,
        }
    }

    /// The bytes of a [`WsMessage::Binary`], or `None` for a text frame.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            WsMessage::Binary(b) => Some(b),
            WsMessage::Text(_) => None,
        }
    }
}

/// Details of a WebSocket close, delivered to [`WsHandle::on_close`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsClose {
    /// The WebSocket close code (RFC 6455). `1006` denotes an abnormal closure
    /// where no close frame was received.
    pub code: u16,
    /// The close reason, if the peer supplied one (often empty).
    pub reason: String,
}

/// An event produced by the platform backend and delivered to the registry on
/// the main thread. `Send` (only plain data), so it can hop worker→main on
/// native.
pub(crate) enum WsEvent {
    Open,
    Message(WsMessage),
    Close(WsClose),
    Error(WsError),
}

/// The set of callbacks registered for one connection. Stored main-thread-local
/// so the callbacks may be `!Send`. `on_open` is stored as `FnMut(())` purely so
/// every slot shares the generic [`invoke`] dispatch path.
#[derive(Default)]
struct Handlers {
    on_open: Option<Box<dyn FnMut(())>>,
    on_message: Option<Box<dyn FnMut(WsMessage)>>,
    on_close: Option<Box<dyn FnMut(WsClose)>>,
    on_error: Option<Box<dyn FnMut(WsError)>>,
}

thread_local! {
    /// Callback sets keyed by connection id, on the main (UI) thread. Type-erased
    /// only by connection; kept thread-local (not global) so the callbacks may be
    /// `!Send`. Mirrors `rinch_core`'s parked-callback registry, but persistent:
    /// a WebSocket fires many events over its lifetime.
    static HANDLERS: RefCell<HashMap<u64, Handlers>> = RefCell::new(HashMap::new());
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Deliver `event` to connection `id`'s callbacks. Called on the main thread —
/// directly on web, and via [`rinch_core::run_on_main_thread`] on native.
pub(crate) fn dispatch(id: u64, event: WsEvent) {
    match event {
        WsEvent::Open => invoke(id, (), |h| &mut h.on_open),
        WsEvent::Message(m) => invoke(id, m, |h| &mut h.on_message),
        WsEvent::Close(c) => invoke(id, c, |h| &mut h.on_close),
        WsEvent::Error(e) => invoke(id, e, |h| &mut h.on_error),
    }
}

/// Invoke one callback slot without holding the registry borrow across the call.
///
/// The callback is *taken out* of the registry (releasing the borrow), invoked,
/// then put back only if the entry still exists and the slot is still empty. This
/// makes re-entrancy safe: a callback may drop the [`WsHandle`] (which re-enters
/// the registry to deregister) or register a replacement callback without a
/// double-borrow panic.
fn invoke<T: 'static>(
    id: u64,
    arg: T,
    select: impl Fn(&mut Handlers) -> &mut Option<Box<dyn FnMut(T)>>,
) {
    let taken = HANDLERS.with(|h| h.borrow_mut().get_mut(&id).and_then(|hs| select(hs).take()));
    let Some(mut cb) = taken else { return };
    cb(arg);
    HANDLERS.with(|h| {
        if let Some(hs) = h.borrow_mut().get_mut(&id) {
            let slot = select(hs);
            if slot.is_none() {
                *slot = Some(cb);
            }
        }
    });
}

/// Open a WebSocket connection to `url` (an absolute `ws://` or `wss://` URL).
///
/// Returns a [`WsHandle`] immediately; the connection is established
/// asynchronously and its outcome is reported through the callbacks (`on_open`
/// on success, `on_error` on failure). Register callbacks on the returned handle
/// right away — events are only delivered once control returns to the event loop,
/// so no event can be missed between `connect` and callback registration.
///
/// A synchronous `Err` is returned only when the request is rejected outright
/// (a non-`ws`/`wss` URL, or a browser that refuses to construct the socket).
///
/// Must be called on the main (UI) thread; see the [module docs](crate).
pub fn connect(url: impl Into<String>) -> Result<WsHandle, WsError> {
    let url = url.into();
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err(WsError::InvalidUrl(format!(
            "URL must be absolute and start with ws:// or wss:// (got {url:?})"
        )));
    }

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

    match backend::connect(&url, id) {
        Ok(backend) => Ok(WsHandle { id, backend }),
        Err(e) => {
            HANDLERS.with(|h| {
                h.borrow_mut().remove(&id);
            });
            Err(e)
        }
    }
}

/// A handle to an open (or opening) WebSocket connection.
///
/// Register callbacks with [`on_open`](Self::on_open) /
/// [`on_message`](Self::on_message) / [`on_close`](Self::on_close) /
/// [`on_error`](Self::on_error) (chainable), send frames with
/// [`send_text`](Self::send_text) / [`send_bytes`](Self::send_bytes), and end the
/// connection with [`close`](Self::close). Dropping the handle closes the
/// connection and deregisters its callbacks.
///
/// Not `Send`: it lives on the main (UI) thread alongside its callbacks.
pub struct WsHandle {
    id: u64,
    backend: backend::Backend,
}

impl WsHandle {
    /// Register the connection-opened callback (replaces any previous one).
    pub fn on_open(&self, mut cb: impl FnMut() + 'static) -> &Self {
        self.set(|h| h.on_open = Some(Box::new(move |()| cb())));
        self
    }

    /// Register the message-received callback (replaces any previous one).
    pub fn on_message(&self, cb: impl FnMut(WsMessage) + 'static) -> &Self {
        self.set(|h| h.on_message = Some(Box::new(cb)));
        self
    }

    /// Register the connection-closed callback (replaces any previous one).
    ///
    /// Fires once, whether the close was initiated locally, by the peer, or by an
    /// abnormal drop (reported with code `1006`).
    pub fn on_close(&self, cb: impl FnMut(WsClose) + 'static) -> &Self {
        self.set(|h| h.on_close = Some(Box::new(cb)));
        self
    }

    /// Register the error callback (replaces any previous one).
    pub fn on_error(&self, cb: impl FnMut(WsError) + 'static) -> &Self {
        self.set(|h| h.on_error = Some(Box::new(cb)));
        self
    }

    /// Queue a text frame to be sent. A no-op if the connection is already gone.
    pub fn send_text(&self, text: impl Into<String>) {
        self.backend.send_text(text.into());
    }

    /// Queue a binary frame to be sent. A no-op if the connection is already gone.
    pub fn send_bytes(&self, bytes: Vec<u8>) {
        self.backend.send_bytes(bytes);
    }

    /// Begin a graceful close. The [`on_close`](Self::on_close) callback fires
    /// once the close completes. Dropping the handle does the same.
    pub fn close(&self) {
        self.backend.close();
    }

    fn set(&self, f: impl FnOnce(&mut Handlers)) {
        HANDLERS.with(|h| {
            if let Some(hs) = h.borrow_mut().get_mut(&self.id) {
                f(hs);
            }
        });
    }
}

impl Drop for WsHandle {
    fn drop(&mut self) {
        self.backend.close();
        HANDLERS.with(|h| {
            h.borrow_mut().remove(&self.id);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_rejects_non_ws_scheme() {
        for url in [
            "http://example.com",
            "https://example.com/x",
            "example.com",
            "",
        ] {
            match connect(url) {
                Err(WsError::InvalidUrl(_)) => {}
                Err(e) => panic!("expected InvalidUrl for {url:?}, got {e:?}"),
                Ok(_) => panic!("expected InvalidUrl for {url:?}, got Ok"),
            }
        }
    }

    #[test]
    fn message_accessors() {
        let t = WsMessage::Text("hi".to_string());
        assert_eq!(t.as_text(), Some("hi"));
        assert_eq!(t.as_bytes(), None);

        let b = WsMessage::Binary(vec![1, 2, 3]);
        assert_eq!(b.as_bytes(), Some(&[1u8, 2, 3][..]));
        assert_eq!(b.as_text(), None);
    }

    #[test]
    fn invoke_dispatches_and_is_reentrant() {
        // Register a connection by hand (no real socket) and drive the dispatch
        // path directly, including a callback that re-enters the registry.
        let id = 424242;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let seen = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let s = seen.clone();
        HANDLERS.with(|h| {
            h.borrow_mut().get_mut(&id).unwrap().on_message = Some(Box::new(move |m| {
                if let WsMessage::Text(t) = m {
                    s.borrow_mut().push(t);
                }
                // Re-enter the registry from inside the callback: must not
                // double-borrow.
                let present = HANDLERS.with(|h| h.borrow().contains_key(&id));
                assert!(present);
            }));
        });

        dispatch(id, WsEvent::Message(WsMessage::Text("a".to_string())));
        dispatch(id, WsEvent::Message(WsMessage::Text("b".to_string())));
        assert_eq!(*seen.borrow(), vec!["a".to_string(), "b".to_string()]);

        // Unknown ids are a silent no-op.
        dispatch(999_999, WsEvent::Open);

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }
}
