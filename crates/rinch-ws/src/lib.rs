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
//! # Lifetime
//!
//! Callbacks registered from inside a render belong to the component that
//! registered them. A [`WsHandle`] kept past its component — parked in a store,
//! a global, or simply leaked — would otherwise keep firing callbacks that hold
//! that component's freed `Signal`s, and reading a freed signal panics (issue
//! #183). So the ambient owner is recorded at registration and checked at
//! dispatch: once it is gone the callback is dropped instead of called, and it
//! is not put back.
//!
//! The owner is recorded per **callback**, not per connection, so one handle may
//! be shared by more than one component — a message list registering
//! `on_message`, a connection-status badge registering `on_close` — and each
//! callback stops firing on its own component's unmount. A per-connection owner
//! would let the badge's live registration keep the unmounted list's
//! `on_message` armed, which is the very panic this guards against.
//!
//! The check happens at dispatch rather than through a cleanup registered on the
//! scope, because registration here is per-`on_message` call and a component may
//! re-register freely — one cleanup per call would grow without bound. This is
//! the same trade-off
//! [`park_main_callback`](rinch_core::park_main_callback) makes, and the
//! opposite of the one
//! [`install_scoped_slot`](rinch_core::reactive::install_scoped_slot) makes for
//! write-once interceptor slots.
//!
//! Registering with **no ambient owner** — from `main`, from startup code, from
//! a detached callback — records no owner and keeps app lifetime, unchanged.
//! Dropping the handle still deregisters everything, and remains the ordinary
//! way a connection ends.
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

use rinch_core::reactive::{Owner, current_owner, unowned};

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

/// One registered callback plus the scope that registered it.
///
/// The owner is recorded per **slot**, not per connection. The four callbacks of
/// a connection are usually registered together off one `connect`, but they need
/// not be — a message list and a connection-status badge may each hold the same
/// handle — and a shared per-connection owner would let the live registration
/// keep the unmounted one's callback armed, which is exactly the panic this
/// guards against (issue #183).
struct Slot<T> {
    /// The scope that was rendering when this callback was registered, if any.
    ///
    /// `None` means registration happened outside any render (`main`, a timer, a
    /// detached callback) and the callback has app lifetime. `Some` that has
    /// since been disposed means the component is gone and [`invoke`] must prune
    /// rather than call. `Owner` is a `Weak`, so this keeps nothing alive.
    owner: Option<Owner>,
    cb: Box<dyn FnMut(T)>,
}

impl<T> Slot<T> {
    /// Box a user callback together with the scope that is currently rendering.
    ///
    /// The owner is captured *here*, where the user's closure was created and
    /// where it captured its `Signal`s — not at [`connect`], which may well run
    /// from a different scope, or from none.
    fn new(cb: Box<dyn FnMut(T)>) -> Self {
        Self {
            owner: current_owner(),
            cb,
        }
    }

    /// Whether the component that registered this callback is gone.
    ///
    /// `false` for an ownerless registration, which has app lifetime.
    fn is_dead(&self) -> bool {
        self.owner.as_ref().is_some_and(|owner| !owner.is_alive())
    }
}

/// The set of callbacks registered for one connection. Stored main-thread-local
/// so the callbacks may be `!Send`. `on_open` is stored as `FnMut(())` purely so
/// every slot shares the generic [`invoke`] dispatch path.
#[derive(Default)]
struct Handlers {
    on_open: Option<Slot<()>>,
    on_message: Option<Slot<WsMessage>>,
    on_close: Option<Slot<WsClose>>,
    on_error: Option<Slot<WsError>>,
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
///
/// A callback whose registering component has since unmounted is **dropped here
/// and not put back** (issue #183). Running it would hand a disposed component's
/// state to the next frame, and a read of a freed
/// [`Signal`](rinch_core::Signal) panics; putting it back would leave the slot
/// armed and re-checked for every event the connection ever delivers. Dropping
/// on the first dead dispatch prunes the slot and releases what the callback
/// captured; [`sweep_dead`] then takes the connection's other dead slots with
/// it. The entry itself stays, because [`WsHandle::drop`](WsHandle) is still
/// what removes it.
///
/// A live callback runs with its registering component as the **ambient owner**,
/// so a `Signal` it creates belongs to that component rather than to whatever
/// the event loop happened to be doing. An *ownerless* callback runs under
/// [`unowned`] for the same reason in reverse: it has app lifetime, so what it
/// allocates must not be handed to whatever scope the dispatch happens to be
/// nested inside.
fn invoke<T: 'static>(id: u64, arg: T, select: impl Fn(&mut Handlers) -> &mut Option<Slot<T>>) {
    let taken = HANDLERS.with(|h| h.borrow_mut().get_mut(&id).and_then(|hs| select(hs).take()));
    let Some(Slot { owner, mut cb }) = taken else {
        return;
    };

    if owner.as_ref().is_some_and(|owner| !owner.is_alive()) {
        // Dropped here, outside the borrow above, and deliberately NOT put back.
        drop(cb);
        sweep_dead(id);
        return;
    }

    match &owner {
        Some(owner) => owner.run(|| cb(arg)),
        None => unowned(|| cb(arg)),
    }

    HANDLERS.with(|h| {
        if let Some(hs) = h.borrow_mut().get_mut(&id) {
            let slot = select(hs);
            if slot.is_none() {
                *slot = Some(Slot { owner, cb });
            }
        }
    });
}

/// Drop every callback on connection `id` whose registering component is gone.
///
/// Each slot prunes itself when it is next dispatched, but a slot may never be
/// dispatched again — `on_open` has already fired, `on_error` never fires on a
/// healthy socket — so on a handle that outlives its component those callbacks
/// would hold what they captured for the life of the process. Called only from
/// the pruning branch of [`invoke`], so a healthy connection pays nothing.
fn sweep_dead(id: u64) {
    // Bound to a `let` that outlives the borrow: the callbacks being dropped are
    // user code whose `Drop` may re-enter the registry.
    let _dead = HANDLERS.with(|h| {
        h.borrow_mut().get_mut(&id).map(|hs| {
            (
                take_dead(&mut hs.on_open),
                take_dead(&mut hs.on_message),
                take_dead(&mut hs.on_close),
                take_dead(&mut hs.on_error),
            )
        })
    });
}

/// Take a slot out if the component that registered it is gone; leave it
/// otherwise. Returns the callback so the caller can drop it outside the borrow.
fn take_dead<T>(slot: &mut Option<Slot<T>>) -> Option<Slot<T>> {
    if slot.as_ref().is_some_and(Slot::is_dead) {
        slot.take()
    } else {
        None
    }
}

/// Install `cb` into the slot `select` picks out for connection `id`, recording
/// the scope that is currently rendering so [`invoke`] can tell whether it is
/// still alive.
///
/// A no-op if `id` is not registered — the handle was dropped, or the connection
/// failed before the callbacks were attached.
fn install<T: 'static>(
    id: u64,
    select: impl Fn(&mut Handlers) -> &mut Option<Slot<T>>,
    cb: Box<dyn FnMut(T)>,
) {
    // Built before the borrow, for two reasons. `Slot::new` captures the ambient
    // owner, which must be read where the user's closure was created; and the
    // callback being displaced is user code whose `Drop` may re-enter the
    // registry (dropping a `WsHandle`, say), which inside the `borrow_mut` would
    // be a `BorrowMutError`. `_displaced` outlives the borrow.
    let slot = Slot::new(cb);
    let _displaced = HANDLERS.with(|h| {
        h.borrow_mut()
            .get_mut(&id)
            .and_then(|hs| select(hs).replace(slot))
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
/// A callback registered from inside a render also stops firing when that
/// component unmounts, even if the handle itself lives on — see the [module
/// docs](crate#lifetime).
///
/// Not `Send`: it lives on the main (UI) thread alongside its callbacks.
pub struct WsHandle {
    id: u64,
    backend: backend::Backend,
}

impl WsHandle {
    /// Register the connection-opened callback (replaces any previous one).
    pub fn on_open(&self, mut cb: impl FnMut() + 'static) -> &Self {
        install(self.id, |h| &mut h.on_open, Box::new(move |()| cb()));
        self
    }

    /// Register the message-received callback (replaces any previous one).
    pub fn on_message(&self, cb: impl FnMut(WsMessage) + 'static) -> &Self {
        install(self.id, |h| &mut h.on_message, Box::new(cb));
        self
    }

    /// Register the connection-closed callback (replaces any previous one).
    ///
    /// Fires once, whether the close was initiated locally, by the peer, or by an
    /// abnormal drop (reported with code `1006`).
    pub fn on_close(&self, cb: impl FnMut(WsClose) + 'static) -> &Self {
        install(self.id, |h| &mut h.on_close, Box::new(cb));
        self
    }

    /// Register the error callback (replaces any previous one).
    pub fn on_error(&self, cb: impl FnMut(WsError) + 'static) -> &Self {
        install(self.id, |h| &mut h.on_error, Box::new(cb));
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
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;
    use std::rc::Rc;

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

        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let s = seen.clone();
        install(
            id,
            |h| &mut h.on_message,
            Box::new(move |m| {
                if let WsMessage::Text(t) = m {
                    s.borrow_mut().push(t);
                }
                // Re-enter the registry from inside the callback: must not
                // double-borrow.
                let present = HANDLERS.with(|h| h.borrow().contains_key(&id));
                assert!(present);
            }),
        );

        dispatch(id, WsEvent::Message(WsMessage::Text("a".to_string())));
        dispatch(id, WsEvent::Message(WsMessage::Text("b".to_string())));
        assert_eq!(*seen.borrow(), vec!["a".to_string(), "b".to_string()]);

        // Unknown ids are a silent no-op.
        dispatch(999_999, WsEvent::Open);

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// A callback registered while a component was rendering must not run once
    /// that component is gone: it captured the component's `Signal`s, disposal
    /// freed them, and a read of a freed signal panics (issue #183, #141 PR4).
    #[test]
    fn a_callback_registered_in_a_scope_is_not_invoked_after_the_scope_disposes() {
        let id = 424_243;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| {
            install(id, |h| &mut h.on_message, Box::new(move |_| flag.set(true)));
        });

        scope.dispose();
        dispatch(
            id,
            WsEvent::Message(WsMessage::Text("after-unmount".to_string())),
        );

        assert!(
            !ran.get(),
            "a callback registered by a since-disposed scope must not be invoked"
        );

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// Pruning, not re-arming: the dead callback is dropped on the first dispatch
    /// that finds it, so its captures are released and no later event re-checks
    /// it. Putting it back would leave the entry armed forever.
    #[test]
    fn a_dead_callback_is_dropped_rather_than_restored() {
        struct DropSpy {
            ran: Rc<Cell<bool>>,
            dropped: Rc<Cell<bool>>,
        }
        impl DropSpy {
            fn note_run(&self) {
                self.ran.set(true);
            }
        }
        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.dropped.set(true);
            }
        }

        let id = 424_244;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let ran = Rc::new(Cell::new(false));
        let dropped = Rc::new(Cell::new(false));
        let spy = DropSpy {
            ran: ran.clone(),
            dropped: dropped.clone(),
        };
        let scope = Scope::new();
        scope.run(|| {
            install(id, |h| &mut h.on_message, Box::new(move |_| spy.note_run()));
        });

        scope.dispose();
        dispatch(id, WsEvent::Message(WsMessage::Text("x".to_string())));

        assert!(!ran.get(), "the dead callback must not run");
        let still_armed = HANDLERS.with(|h| h.borrow().get(&id).map(|hs| hs.on_message.is_some()));
        assert_eq!(
            still_armed,
            Some(false),
            "the dead callback must be pruned rather than put back, or every \
             later event re-checks it"
        );
        assert!(
            dropped.get(),
            "pruning must actually drop the callback, releasing what it captured"
        );

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// Registration from `main`, from startup code or from a detached callback
    /// has no ambient owner and therefore app lifetime — the pre-#141 default,
    /// which the liveness check must not disturb.
    #[test]
    fn a_callback_registered_with_no_ambient_owner_still_runs() {
        let id = 424_245;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let s = seen.clone();
        // Deliberately not inside a `Scope::run`.
        install(
            id,
            |h| &mut h.on_message,
            Box::new(move |m| {
                if let WsMessage::Text(t) = m {
                    s.borrow_mut().push(t);
                }
            }),
        );

        dispatch(id, WsEvent::Message(WsMessage::Text("a".to_string())));
        dispatch(id, WsEvent::Message(WsMessage::Text("b".to_string())));
        assert_eq!(*seen.borrow(), vec!["a".to_string(), "b".to_string()]);

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// The callback runs with its registering component as the ambient owner, so
    /// whatever it allocates belongs to that component rather than to whatever
    /// the event loop happened to be doing.
    #[test]
    fn a_live_callback_runs_with_its_component_as_ambient_owner() {
        let id = 424_246;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let scope = Scope::new();
        scope.run(|| {
            install(
                id,
                |h| &mut h.on_message,
                Box::new(|_| {
                    let _owned_by_the_component = Signal::new(0u32);
                }),
            );
        });

        let before = scope.owned_counts().signals;
        dispatch(id, WsEvent::Message(WsMessage::Text("x".to_string())));
        let after = scope.owned_counts().signals;
        assert_eq!(
            after,
            before + 1,
            "a signal created inside the callback must be attributed to the \
             scope that registered it"
        );

        scope.dispose();
        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// The owner is per **callback**, not per connection. Two components may
    /// share one handle — a message list and a connection-status badge — and the
    /// badge staying mounted must not keep the list's dead `on_message` armed.
    #[test]
    fn a_live_sibling_registration_does_not_keep_a_dead_callback_armed() {
        let id = 424_247;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let list_ran = Rc::new(Cell::new(false));
        let badge_ran = Rc::new(Cell::new(false));

        let list_flag = list_ran.clone();
        let list = Scope::new();
        list.run(|| {
            install(
                id,
                |h| &mut h.on_message,
                Box::new(move |_| list_flag.set(true)),
            );
        });

        // Registered *after* the list, from a different, longer-lived component.
        let badge_flag = badge_ran.clone();
        let badge = Scope::new();
        badge.run(|| {
            install(
                id,
                |h| &mut h.on_close,
                Box::new(move |_| badge_flag.set(true)),
            );
        });

        list.dispose();
        dispatch(id, WsEvent::Message(WsMessage::Text("x".to_string())));
        assert!(
            !list_ran.get(),
            "the unmounted component's callback must not run just because a \
             sibling component registered later on the same handle"
        );

        dispatch(
            id,
            WsEvent::Close(WsClose {
                code: 1000,
                reason: String::new(),
            }),
        );
        assert!(
            badge_ran.get(),
            "the still-mounted component's callback must keep working"
        );

        badge.dispose();
        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }

    /// A slot that will never be dispatched again (`on_open` after the
    /// handshake, `on_error` on a healthy socket) would otherwise hold its
    /// component's captures for the life of the process, so the first dead
    /// dispatch sweeps the connection's other dead slots too.
    #[test]
    fn pruning_one_slot_sweeps_the_connections_other_dead_slots() {
        struct DropFlag(Rc<Cell<bool>>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let id = 424_248;
        HANDLERS.with(|h| h.borrow_mut().insert(id, Handlers::default()));

        let open_dropped = Rc::new(Cell::new(false));
        let open_guard = DropFlag(open_dropped.clone());
        let scope = Scope::new();
        scope.run(|| {
            install(
                id,
                |h| &mut h.on_open,
                Box::new(move |()| {
                    // Keeps the guard (and so the component's captures) alive.
                    let _ = &open_guard;
                }),
            );
            install(id, |h| &mut h.on_message, Box::new(|_| {}));
        });

        scope.dispose();
        // `on_open` has already fired for this connection and never will again;
        // only the message dispatch is left to notice.
        dispatch(id, WsEvent::Message(WsMessage::Text("x".to_string())));

        let armed = HANDLERS.with(|h| {
            h.borrow()
                .get(&id)
                .map(|hs| (hs.on_open.is_some(), hs.on_message.is_some()))
        });
        assert_eq!(
            armed,
            Some((false, false)),
            "a dead dispatch must sweep the connection's other dead slots"
        );
        assert!(
            open_dropped.get(),
            "sweeping must actually drop the swept callback, releasing what it \
             captured"
        );

        HANDLERS.with(|h| {
            h.borrow_mut().remove(&id);
        });
    }
}
