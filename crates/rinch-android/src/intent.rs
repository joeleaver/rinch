//! Intents arriving *at* the app: a share, a view, a deep link.
//!
//! `share.rs` is the outbound half of this — `ACTION_SEND` fired at the chooser
//! so the user can hand something to another app. This is the half that was
//! missing. An app whose manifest declares an `<intent-filter>` for
//! `ACTION_SEND` is already a share target as far as Android is concerned: it
//! appears in every chooser, the user picks it, the system launches it with the
//! payload attached — and until now the payload went nowhere, because nothing
//! in this crate ever looked at the `Intent`. Being launched and being told why
//! were two different things and only the first one worked.
//!
//! # Shape
//!
//! The same shape as `callback.rs`, for the same reason. JNI pushes an
//! [`IncomingIntent`] into a `Mutex<Vec<_>>` from whichever thread Android felt
//! like calling from; [`drain_incoming_intents`] runs on the main thread once a
//! frame and hands each one to the registered handlers. Nothing crosses the
//! thread boundary except an owned struct of `String`s, so the handler — which
//! will want to write `Signal`s, and `Signal::set` panics off the main thread —
//! only ever runs where it is allowed to.
//!
//! # Ordering, and why the queue is not drained empty
//!
//! Cold start is the interesting case and it runs in this order:
//!
//! 1. `android_main` calls `init`, which registers the natives and then asks
//!    Java to flush whatever `onCreate` stashed. The launch intent — the share
//!    the user just made — lands in the queue here.
//! 2. The app builds its UI and calls [`on_incoming_intent`] somewhere in that
//!    process.
//! 3. The frame loop starts, and calls [`drain_incoming_intents`].
//!
//! Steps 1 and 2 are in that order and cannot be swapped: Java may not call a
//! native before Rust has registered it (see `flushPendingIntents` in
//! `RinchActivity.java`), so the launch intent necessarily arrives before any
//! app code has run. A drain that discarded intents nobody was listening for
//! would therefore discard *exactly* the one that matters, every time, and the
//! bug would look like "sharing to the app opens it empty". So a drain with no
//! handler registered leaves the queue alone and tries again next frame. The
//! cost is that an app which never registers a handler accumulates them, which
//! `MAX_PENDING` bounds.

use std::rc::Rc;
use std::sync::Mutex;

use crate::scoped::ScopedList;

/// `Intent.ACTION_MAIN` — what the launcher sends, and the one thing worth
/// filtering out. Spelled here rather than imported from anywhere because the
/// Java side sends the action across as a plain string.
pub const ACTION_MAIN: &str = "android.intent.action.MAIN";

/// `Intent.ACTION_SEND`.
pub const ACTION_SEND: &str = "android.intent.action.SEND";

/// `Intent.ACTION_SEND_MULTIPLE`.
pub const ACTION_SEND_MULTIPLE: &str = "android.intent.action.SEND_MULTIPLE";

/// `Intent.ACTION_VIEW`.
pub const ACTION_VIEW: &str = "android.intent.action.VIEW";

/// How many intents may sit undelivered before the oldest is dropped.
///
/// The queue is held rather than discarded while no handler is registered (see
/// the module docs), and "no handler is ever registered" is a perfectly
/// ordinary state for an app that declares no `<intent-filter>` and never asks.
/// Such an app should not grow a `Vec` for the lifetime of the process because
/// somebody deep-linked into it. Thirty-two is far more than the number of
/// shares a user can make between two frames and far less than a leak.
const MAX_PENDING: usize = 32;

/// One intent as it arrived, flattened to owned data.
///
/// Deliberately not a handle to the Java `Intent`: by the time a handler runs,
/// the frame loop is somewhere else entirely and the JNI local reference that
/// carried it is long gone. Everything a share can contain that rinch knows how
/// to describe is copied out on the JNI thread and travels as `String`s.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncomingIntent {
    /// `intent.getAction()`, e.g. `"android.intent.action.SEND"`. Empty when
    /// the intent carried no action at all, which is legal and rare.
    pub action: String,
    /// `intent.getType()` — the MIME type the sender declared, not one derived
    /// from the payload. `"text/plain"`, `"image/*"`, and so on.
    pub mime_type: Option<String>,
    /// `EXTRA_TEXT`. A share from a browser puts the URL here; a share from a
    /// notes app puts the note here.
    pub text: Option<String>,
    /// `EXTRA_STREAM`, as `content://` URIs. One entry for `ACTION_SEND`, many
    /// for `ACTION_SEND_MULTIPLE`, none for a text-only share. Read them with
    /// `file_picker::read_content_uri` — they are content-provider
    /// URIs belonging to the sending app, not paths.
    pub stream_uris: Vec<String>,
    /// `intent.getData()`. This is where `ACTION_VIEW` puts the thing to view,
    /// and where a deep link arrives.
    pub data_uri: Option<String>,
}

impl IncomingIntent {
    /// Whether the intent carries anything at all beyond its action.
    pub fn has_payload(&self) -> bool {
        self.mime_type.is_some()
            || self.text.is_some()
            || !self.stream_uris.is_empty()
            || self.data_uri.is_some()
    }

    /// Whether this is the intent the launcher sends when the user taps the
    /// icon: `ACTION_MAIN` and nothing else.
    ///
    /// Every cold start produces one of these, so delivering it would mean
    /// every app with a handler gets woken on every launch to be told nothing.
    /// The payload check is the reason this is not simply
    /// `action == ACTION_MAIN`: an app shortcut, and a launcher that supports
    /// them, sends `ACTION_MAIN` *with* a `data` URI naming which shortcut was
    /// tapped, and that is a real message with a real answer. Filtering on the
    /// action alone would swallow it.
    ///
    /// An intent with no action and no payload gets the same treatment, for
    /// want of any way to tell it apart from the above or anything to say
    /// about it.
    pub fn is_bare_launch(&self) -> bool {
        !self.has_payload() && (self.action.is_empty() || self.action == ACTION_MAIN)
    }
}

// ── The queue (pushed from JNI, any thread) ─────────────────────────────────

static INCOMING: Mutex<Vec<IncomingIntent>> = Mutex::new(Vec::new());

// ── The handler registry (main thread only) ─────────────────────────────────

// `Rc` rather than `callback.rs`'s `Box`, because a handler is app code and app
// code is entitled to call `on_incoming_intent` from inside a delivery — a
// screen that opens in response to a share and wants the next one too.
// Iterating the registry in place while that happens is a `RefCell`
// double-borrow panic, on a path nobody exercises until a user finds it, so the
// live entries are cloned out from under the borrow before any of them runs,
// and `Rc` is what makes that clone cheap. That clone-out now happens inside
// `ScopedList::dispatch` rather than here — this comment described the loop in
// this file until the registry grew a lifetime (#599).
thread_local! {
    /// The registered handlers, each tied to the component that registered it.
    ///
    /// A [`ScopedList`] rather than a bare `Vec`, and the reason is #183 rather
    /// than tidiness: a handler written inside a `#[component]` captures that
    /// component's `Signal`s, disposing the component frees them, and a read of
    /// a freed signal **panics**. An incoming intent arrives long after mount by
    /// nature — a share, a deep link, a shortcut — so "the component that
    /// registered has unmounted since" is the ordinary case here.
    ///
    /// **Why the list and not one of this crate's two older templates** is
    /// argued where the template lives ([`ScopedList`]), because it is a fact
    /// about the shapes rather than about this module: a [`ScopedSlot`] would
    /// make the second registrant silently disable the first, which is the
    /// opposite of what `on_incoming_intent` promises, and a
    /// [`ScopedMap`](crate::scoped::ScopedMap) needs a key that nothing here
    /// would ever use to address an entry. This registry is the third in three
    /// pull requests to ask the question and the first for which neither answer
    /// fitted (issue #599).
    static HANDLERS: ScopedList<dyn Fn(IncomingIntent)> = const { ScopedList::new() };
}

/// Register a handler for intents that arrive after launch *and* at launch.
///
/// Main thread only — the handler is stored thread-locally and will be run by
/// [`drain_incoming_intents`] on the thread that registered it, which is the
/// property that makes it safe for the handler to touch the reactive system.
///
/// Calling this more than once adds a handler rather than replacing the one
/// before it, and every one of them sees every intent. Two independent parts of
/// an app can each care about being shared to without having to know about each
/// other, and neither can silently disable the other by registering second.
///
/// The launch intent is *not* lost by registering late. Intents queued before
/// the first handler exists stay queued; see the module docs.
pub fn on_incoming_intent(cb: impl Fn(IncomingIntent) + 'static) {
    HANDLERS.with(|handlers| handlers.install(Rc::new(cb)));
}

/// Queue an intent for delivery on the next drain. Returns whether it was kept.
///
/// Separate from the JNI entry point below so that the filtering and the
/// bounding — the parts with a decision in them — can be exercised on a laptop.
#[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
fn enqueue(intent: IncomingIntent) -> bool {
    // The Java side already skipped this one; this is the second of the two
    // gates and the authoritative one. Java filters early so that a launcher
    // tap does not grow the pending list of an app that has not finished
    // starting yet, but the filter lives here, where it can be tested, and
    // Java's copy is an optimisation that is allowed to be wrong in the
    // conservative direction.
    if intent.is_bare_launch() {
        return false;
    }

    let mut queue = INCOMING.lock().unwrap_or_else(|e| e.into_inner());
    if queue.len() >= MAX_PENDING {
        // Oldest first: a share nobody has collected in thirty-two shares'
        // time is not the one the user is waiting on.
        queue.remove(0);
        log::warn!(
            "incoming intent queue is full ({MAX_PENDING}); dropping the oldest. \
             Is anything calling rinch_android::intent::on_incoming_intent()?"
        );
    }
    queue.push(intent);
    true
}

/// Deliver queued intents to the registered handlers. Called once a frame by
/// the shell's Android run loop, on the main thread.
///
/// A no-op — and specifically a *non-destructive* no-op — while no handler is
/// registered.
pub fn drain_incoming_intents() {
    // Sweep first, every frame. `dispatch` prunes the entries it visits, but an
    // intent that never arrives prunes nothing — and this registry has no
    // removal API — so a handler whose component unmounted before any share
    // came would otherwise be held, with everything it captured, for the life
    // of the process. A `Weak` upgrade and a flag read per handler, on a list
    // that has one or two entries.
    HANDLERS.with(|handlers| handlers.release_dead());

    // **Live**, not merely present. A handler whose component is gone is not a
    // listener, and counting it as one would take the queue below and deliver
    // it to nobody — turning this deliberate, non-destructive no-op into a
    // silent drop of the very share that launched the app.
    if !HANDLERS.with(|handlers| handlers.has_live()) {
        return;
    }

    let intents: Vec<IncomingIntent> =
        std::mem::take(&mut *INCOMING.lock().unwrap_or_else(|e| e.into_inner()));
    for intent in intents {
        // Dispatched per intent, not once for the loop: a handler that registers
        // another handler means the next intent in this same drain has one more
        // listener than the last did, which is the behaviour a reader expects
        // from a registry that is read at the point of use. `ScopedList` takes
        // its snapshot inside each call, so that property is this loop's to
        // keep.
        HANDLERS.with(|handlers| handlers.dispatch(|cb| cb(intent.clone())));
    }
}

// ── Android-only: the JNI entry point and the flush handshake ───────────────

/// Tell Java that the natives are registered and it may deliver what it has.
///
/// Called at the end of `init`, and that placement is the whole design of the
/// Java side. `RinchActivity` never calls a native from a lifecycle override,
/// because an override runs on the UI thread at a moment when the native thread
/// may not have got to `RegisterNatives` yet, and the result is an
/// `UnsatisfiedLinkError` that force-finishes the activity on cold start — the
/// comment beside `onCreate`'s neighbours in that file says so. So `onCreate`
/// only ever *stores* the launch intent, and this is Rust reaching over the
/// handshake to say that the other side is ready now.
#[cfg(target_os = "android")]
pub(crate) fn flush_pending_java_intents() {
    crate::bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "flushPendingIntents", "()V", &[]) {
            log::warn!("flushPendingIntents JNI call failed: {e}");
        }
    });
}

/// Read a possibly-null `JString` into an `Option<String>`.
#[cfg(target_os = "android")]
fn opt_string(env: &mut jni::JNIEnv, s: &jni::objects::JString, what: &str) -> Option<String> {
    if s.is_null() {
        return None;
    }
    match env.get_string(s) {
        Ok(s) => Some(String::from(s)),
        Err(e) => {
            log::warn!("failed to read {what} from the incoming intent: {e}");
            None
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnIncomingIntent(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    action: jni::objects::JString,
    mime_type: jni::objects::JString,
    text: jni::objects::JString,
    stream_uris: jni::objects::JObjectArray,
    data_uri: jni::objects::JString,
) {
    let action = opt_string(&mut env, &action, "action").unwrap_or_default();
    let mime_type = opt_string(&mut env, &mime_type, "type");
    let text = opt_string(&mut env, &text, "EXTRA_TEXT");
    let data_uri = opt_string(&mut env, &data_uri, "data URI");

    let mut streams = Vec::new();
    if !stream_uris.is_null() {
        let len = env.get_array_length(&stream_uris).unwrap_or(0);
        for i in 0..len {
            // `get_object_array_element` hands back a *local* reference, and
            // the JNI local-reference table on a thread this crate attached
            // itself has a modest guaranteed capacity. A SEND_MULTIPLE of a
            // hundred photos is an entirely ordinary thing for a user to do,
            // so each one is scoped and released rather than left to the frame
            // that never ends.
            let Ok(element) = env.get_object_array_element(&stream_uris, i) else {
                continue;
            };
            let uri = jni::objects::JString::from(element);
            if let Some(s) = opt_string(&mut env, &uri, "EXTRA_STREAM entry") {
                streams.push(s);
            }
            let _ = env.delete_local_ref(uri);
        }
    }

    let kept = enqueue(IncomingIntent {
        action,
        mime_type,
        text,
        stream_uris: streams,
        data_uri,
    });

    if kept {
        // Same reasoning as the activity-result callback next door: since K37
        // the loop only has frames when something is happening, and an intent
        // arriving while the app sits idle in the background *is* the
        // something. Without this the share would be delivered whenever the
        // user next touched the screen. See `wake.rs`.
        crate::wake::wake_main();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex as StdMutex;

    /// The queue is a process-wide `static` and the handler registry is
    /// thread-local, so two tests running at once on two threads would share
    /// the first and not the second — one test's intents delivered to the other
    /// test's handlers, or to nobody. Every test takes this first.
    static SERIAL: StdMutex<()> = StdMutex::new(());

    struct Guard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    /// Serialise, and start from a queue and a registry that are known empty —
    /// the registry because each test gets a fresh thread, the queue because
    /// nothing else does.
    fn setup() -> Guard {
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        INCOMING.lock().unwrap_or_else(|e| e.into_inner()).clear();
        Guard(guard)
    }

    fn send(text: &str) -> IncomingIntent {
        IncomingIntent {
            action: ACTION_SEND.into(),
            mime_type: Some("text/plain".into()),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    /// Register a handler that records what it was given, and hand back the log.
    fn recording_handler() -> Rc<RefCell<Vec<String>>> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        on_incoming_intent(move |intent| {
            sink.borrow_mut()
                .push(intent.text.unwrap_or_else(|| intent.action.clone()));
        });
        seen
    }

    #[test]
    fn a_bare_launcher_intent_is_not_an_intent_worth_waking_for() {
        let main = IncomingIntent {
            action: ACTION_MAIN.into(),
            ..Default::default()
        };
        assert!(main.is_bare_launch());
        assert!(!main.has_payload());

        // No action at all and nothing attached is the same non-message.
        assert!(IncomingIntent::default().is_bare_launch());
    }

    #[test]
    fn action_main_carrying_a_shortcut_uri_is_a_real_message() {
        let shortcut = IncomingIntent {
            action: ACTION_MAIN.into(),
            data_uri: Some("setlist://open/42".into()),
            ..Default::default()
        };
        assert!(shortcut.has_payload());
        assert!(
            !shortcut.is_bare_launch(),
            "filtering on the action alone would swallow an app shortcut"
        );
    }

    #[test]
    fn a_share_is_never_mistaken_for_a_launch() {
        assert!(!send("hello").is_bare_launch());
        assert!(
            !IncomingIntent {
                action: ACTION_VIEW.into(),
                data_uri: Some("https://example.com".into()),
                ..Default::default()
            }
            .is_bare_launch()
        );
        assert!(
            !IncomingIntent {
                action: ACTION_SEND_MULTIPLE.into(),
                mime_type: Some("image/*".into()),
                stream_uris: vec!["content://a".into(), "content://b".into()],
                ..Default::default()
            }
            .is_bare_launch()
        );
    }

    #[test]
    fn the_filter_runs_on_the_way_into_the_queue() {
        let _g = setup();
        assert!(!enqueue(IncomingIntent {
            action: ACTION_MAIN.into(),
            ..Default::default()
        }));
        assert!(enqueue(send("kept")));
        assert_eq!(INCOMING.lock().unwrap().len(), 1);
    }

    #[test]
    fn intents_are_delivered_in_the_order_they_arrived() {
        let _g = setup();
        let seen = recording_handler();

        enqueue(send("first"));
        enqueue(send("second"));
        enqueue(send("third"));
        drain_incoming_intents();

        assert_eq!(*seen.borrow(), vec!["first", "second", "third"]);
    }

    #[test]
    fn draining_twice_delivers_once() {
        let _g = setup();
        let seen = recording_handler();

        enqueue(send("only"));
        drain_incoming_intents();
        drain_incoming_intents();
        drain_incoming_intents();

        assert_eq!(*seen.borrow(), vec!["only"]);
    }

    #[test]
    fn the_launch_intent_waits_for_a_handler_instead_of_being_thrown_away() {
        let _g = setup();

        // Cold start: `init` flushes the launch intent through before any app
        // code has run, and the loop takes a frame or two to get going.
        enqueue(send("the share that launched the app"));
        drain_incoming_intents();
        drain_incoming_intents();

        // Only now does the app get around to asking.
        let seen = recording_handler();
        drain_incoming_intents();

        assert_eq!(*seen.borrow(), vec!["the share that launched the app"]);
    }

    #[test]
    fn every_handler_sees_every_intent() {
        let _g = setup();
        let first = recording_handler();
        let second = recording_handler();

        enqueue(send("shared"));
        drain_incoming_intents();

        assert_eq!(*first.borrow(), vec!["shared"]);
        assert_eq!(*second.borrow(), vec!["shared"]);
    }

    // ── Handler lifetime (#183, #599) ────────────────────────────────────

    /// **The defect this registry had**: a handler outliving the component that
    /// registered it.
    ///
    /// A handler written inside a `#[component]` captures that component's
    /// `Signal`s; since #141 PR4 disposing the component frees them, and a read
    /// of a freed signal **panics**. An incoming intent arrives long after
    /// mount by nature — a share, a deep link, a shortcut — so the window is the
    /// whole life of the app rather than the tail of one event.
    #[test]
    fn a_handler_stops_firing_once_its_component_unmounts() {
        let _g = setup();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let scope = Scope::new();
        let sink = seen.clone();
        scope.run(|| {
            on_incoming_intent(move |intent| {
                sink.borrow_mut().push(intent.text.unwrap_or_default());
            })
        });

        // Fail-first inside the fixture: prove it fires while mounted, so "it
        // did not fire" below cannot pass for the wrong reason.
        enqueue(send("while mounted"));
        drain_incoming_intents();
        assert_eq!(
            seen.borrow().len(),
            1,
            "precondition: it runs while mounted"
        );

        scope.dispose();
        enqueue(send("after unmount"));
        drain_incoming_intents();
        assert_eq!(
            seen.borrow().len(),
            1,
            "and never again once its component is gone"
        );
    }

    /// The same defect where it actually bites: the handler reads a `Signal`
    /// its own component owned. This fixture fails by **panicking** in
    /// `Signal::get`'s freed-read path, which is the crash a user would see.
    #[test]
    fn an_unmounted_handler_never_reads_the_signal_its_component_owned() {
        let _g = setup();

        let scope = Scope::new();
        scope.run(|| {
            let count = Signal::new(0u32);
            on_incoming_intent(move |_| {
                // Reaching this line after `dispose` is the bug: reading a freed
                // signal panics rather than merely answering something stale.
                let _ = count.get();
            })
        });

        scope.dispose();
        enqueue(send("after unmount"));
        drain_incoming_intents();
    }

    /// **A dead handler must not swallow the queue**, which is the half of this
    /// a use-after-free test cannot see.
    ///
    /// `drain_incoming_intents` deliberately does nothing while nobody is
    /// listening, so that the share an app was launched with survives until it
    /// registers. If "listening" means "the list is non-empty" rather than "some
    /// entry is alive", an unmounted handler takes the queue and delivers it to
    /// nobody — and that share is gone, silently, on the one path the whole
    /// queue exists to protect.
    #[test]
    fn a_dead_handler_does_not_swallow_the_queue() {
        let _g = setup();

        let scope = Scope::new();
        scope.run(|| on_incoming_intent(|_| {}));
        scope.dispose();

        enqueue(send("the launch share"));
        drain_incoming_intents();

        let seen = recording_handler();
        drain_incoming_intents();
        assert_eq!(
            seen.borrow().as_slice(),
            ["the launch share"],
            "the share must still have been queued when a real listener arrived"
        );
    }

    /// The growth half, which no dispatch can reach: an app whose handler's
    /// component unmounts and to which **no intent ever arrives**. Nothing
    /// prunes on that path, and this registry has no removal API at all, so
    /// without the per-frame sweep the callback and everything it captured are
    /// held for the life of the process.
    #[test]
    fn an_unmounted_handler_is_released_even_if_no_intent_ever_arrives() {
        let _g = setup();

        let scope = Scope::new();
        scope.run(|| on_incoming_intent(|_| {}));
        assert_eq!(HANDLERS.with(|h| h.len()), 1, "precondition: it is held");

        scope.dispose();
        // No intent enqueued at all — the drain is the only thing that runs.
        drain_incoming_intents();
        assert_eq!(
            HANDLERS.with(|h| h.len()),
            0,
            "a drain with nothing to deliver must still let go of a dead handler"
        );
    }

    /// **Rule 1: registration outside any component keeps app lifetime.** An app
    /// that registers from `android_main`, before there is a tree at all, must
    /// not have its handler collected because some unrelated component
    /// unmounted. This is the assertion that stops the fix overshooting.
    #[test]
    fn a_handler_registered_outside_any_component_keeps_app_lifetime() {
        let _g = setup();
        let seen = recording_handler();

        let unrelated = Scope::new();
        unrelated.run(|| {});
        unrelated.dispose();

        enqueue(send("still listening"));
        drain_incoming_intents();
        assert_eq!(seen.borrow().as_slice(), ["still listening"]);
    }

    /// Delivery order is registration order, **and survives a prune**.
    ///
    /// With every handler alive this holds for any implementation that iterates
    /// the list, which is the fixed point: the cheap prune is a swap-removal,
    /// and a swap only reorders when something is actually removed. So the dead
    /// handler is registered *between* the two live ones.
    #[test]
    fn registration_order_survives_a_prune() {
        let _g = setup();
        let log = Rc::new(RefCell::new(Vec::new()));

        for name in ["first", "third"] {
            let sink = log.clone();
            if name == "third" {
                let scope = Scope::new();
                let dead = log.clone();
                scope.run(|| on_incoming_intent(move |_| dead.borrow_mut().push("second")));
                scope.dispose();
            }
            on_incoming_intent(move |_| sink.borrow_mut().push(name));
        }

        enqueue(send("x"));
        drain_incoming_intents();
        assert_eq!(
            log.borrow().as_slice(),
            ["first", "third"],
            "the middle registrant was dead, and removing it must not reorder \
             the two that survive"
        );
    }

    #[test]
    fn a_handler_may_register_another_handler_from_inside_a_delivery() {
        let _g = setup();
        // The double-borrow this guards against is a panic, so the assertion
        // is really "this test finishes at all".
        on_incoming_intent(|_| on_incoming_intent(|_| {}));
        enqueue(send("re-entrant"));
        drain_incoming_intents();
    }

    #[test]
    fn an_app_that_never_listens_does_not_grow_a_queue_forever() {
        let _g = setup();
        for i in 0..(MAX_PENDING + 10) {
            enqueue(send(&format!("share {i}")));
        }
        assert_eq!(INCOMING.lock().unwrap().len(), MAX_PENDING);

        // The oldest went, not the newest: what survives ends at the last one.
        let seen = recording_handler();
        drain_incoming_intents();
        assert_eq!(
            seen.borrow().last().map(String::as_str),
            Some(format!("share {}", MAX_PENDING + 9).as_str())
        );
        assert_eq!(
            seen.borrow().first().map(String::as_str),
            Some(format!("share {}", 10).as_str())
        );
    }
}
