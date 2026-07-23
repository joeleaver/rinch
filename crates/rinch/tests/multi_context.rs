//! Empirical pinning tests for issues #134/#136: two `RinchContext`s on one
//! thread.
//!
//! Each test constructs headless embed contexts (no window, no GPU — `scene()`
//! is never called) and pins the behavior of the shared thread-local state:
//! per-context signal subscriptions and bounds registries (#134) and per-root
//! store/context namespacing with the thread-global root-0 fallback (#136).
//!
//! Requires the `embed` (or `gpu`) feature — with default features OFF (the
//! `desktop` + `embed` combination does not compile; see #134's follow-ups):
//!     cargo test -p rinch --no-default-features --features embed --test multi_context
//!
//! ## Why every test body runs on one shared worker thread
//!
//! `RinchContext::new` calls `rinch_core::register_main_thread()`, a
//! process-wide `OnceLock` — the FIRST test thread to create a context becomes
//! "the main thread" forever, and `Signal::set` panics on every other thread.
//! The libtest harness runs each `#[test]` on its own thread, so all bodies are
//! marshalled onto a single long-lived worker thread (`on_ui_thread`), which
//! also serializes them. Consequence: thread-local reactive state leaks from
//! one test to the next, so each test cleans up what it can (contexts dropped,
//! `clear_context()` where stores are used).

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

// ── harness ──────────────────────────────────────────────────────────────────

type Job = Box<dyn FnOnce() + Send>;

/// Run `f` on the single shared "UI" worker thread and propagate its panic (if
/// any) back to the calling test thread.
fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });

    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 800,
        height: 600,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

/// All text content in a context's document, in tree order.
fn doc_text(ctx: &RinchContext) -> String {
    let doc = ctx.app().doc().expect("context has a document").clone();
    let d = doc.borrow();
    rinch_dom::testing::get_text_content(&d.tree, d.tree.root_id)
}

// ── 1. independent rendering ─────────────────────────────────────────────────

/// Two contexts, each with its own Signal + reactive text. Driving one signal
/// updates only that context's document. This is the part of the shared
/// reactive graph that WORKS: signals/effects live in unique slots, and
/// `Signal::notify` is subscriber-precise, so cross-context effects never
/// misfire.
#[test]
fn two_contexts_render_independently() {
    on_ui_thread(|| {
        let sig_a = Signal::new(0i32);
        let sig_b = Signal::new(0i32);

        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    p { {move || format!("A:{}", sig_a.get())} }
                }
            }
        });
        let mut b = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    p { {move || format!("B:{}", sig_b.get())} }
                }
            }
        });
        a.update(&[]);
        b.update(&[]);

        assert!(doc_text(&a).contains("A:0"), "A initial: {}", doc_text(&a));
        assert!(doc_text(&b).contains("B:0"), "B initial: {}", doc_text(&b));

        // Drive A's signal — only A's document changes.
        sig_a.set(1);
        a.update(&[]);
        b.update(&[]);
        assert!(
            doc_text(&a).contains("A:1"),
            "A must update after its own signal changed, got: {}",
            doc_text(&a)
        );
        assert!(
            doc_text(&b).contains("B:0") && !doc_text(&b).contains("A:"),
            "B must be untouched by A's signal, got: {}",
            doc_text(&b)
        );

        // Drive B's signal — only B's document changes.
        sig_b.set(7);
        a.update(&[]);
        b.update(&[]);
        assert!(
            doc_text(&b).contains("B:7"),
            "B must update after its own signal changed, got: {}",
            doc_text(&b)
        );
        assert!(
            doc_text(&a).contains("A:1"),
            "A must be untouched by B's signal, got: {}",
            doc_text(&a)
        );
    });
}

// ── 2. per-context signal-change subscriptions ───────────────────────────────

/// FIXED (#134): each `RinchContext` holds its own guard-based
/// `subscribe_signal_change` subscription instead of fighting over the legacy
/// single slot. Creating a second context leaves the first connected, every
/// live context sees every signal change, and dropping a context detaches only
/// its own callback — the survivor keeps working.
#[test]
fn contexts_keep_independent_dirty_flags() {
    on_ui_thread(|| {
        // A "bare" signal: no effect subscribes to it, so it never dirties any
        // DOM node — isolating the signal-change dirty flag from the
        // has_dirty_nodes() fallback inside needs_update().
        let bare = Signal::new(0i32);

        let mut a = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! { div { "A static" } }
        });
        a.update(&[]);
        assert!(
            !a.needs_update(),
            "A must be settled after update() with no pending changes"
        );

        // Baseline (single context): any signal change sets A's dirty flag.
        bare.set(1);
        assert!(
            a.needs_update(),
            "baseline: a signal change must set the sole context's dirty flag"
        );
        a.update(&[]);
        assert!(!a.needs_update(), "update() must consume the dirty flag");

        // Create B: both contexts now subscribe independently.
        let mut b = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! { div { "B static" } }
        });
        b.update(&[]);

        bare.set(2);
        assert!(
            a.needs_update(),
            "creating context B must not disconnect A's dirty flag (#134)"
        );
        assert!(
            b.needs_update(),
            "B sees the same signal change through its own subscription"
        );
        a.update(&[]);
        b.update(&[]);

        // Drop B: its subscription guard detaches only B's callback.
        drop(b);
        bare.set(3);
        assert!(
            a.needs_update(),
            "after dropping B the survivor A must still see signal changes (#134)"
        );
        a.update(&[]);
        drop(a);
    });
}

// ── 3. CONTEXT_STORE (create_store/use_store) per-root scoping ───────────────

#[derive(Clone, Copy)]
struct SharedStore {
    which: Signal<i32>,
}

/// FIXED (#136): CONTEXT_STORE is keyed by `(root, TypeId)`. Each mounted
/// `RinchContext` namespaces its stores under its document's `doc_key`, so two
/// contexts creating the same store type no longer overwrite each other:
/// effects and handlers capture their root at creation time, so content A
/// builds LATER (an `if` branch flipping after B mounted) still resolves A's
/// OWN store. Dropping a context clears exactly its own namespace.
#[test]
fn store_crosstalk_between_contexts() {
    on_ui_thread(|| {
        let show_late = Signal::new(false);

        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            create_store(SharedStore {
                which: Signal::new(1),
            });
            rsx! {
                div {
                    if show_late.get() {
                        p { {move || format!("store:{}", use_store::<SharedStore>().which.get())} }
                    } else {
                        p { "waiting" }
                    }
                }
            }
        });
        a.update(&[]);

        let mut b = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            create_store(SharedStore {
                which: Signal::new(2),
            });
            rsx! {
                div {
                    p { {move || format!("store:{}", use_store::<SharedStore>().which.get())} }
                }
            }
        });
        b.update(&[]);
        assert!(
            doc_text(&b).contains("store:2"),
            "B's content resolves B's own store; got: {}",
            doc_text(&b)
        );

        // A's dynamically-built content — an `if` branch constructed AFTER B
        // mounted — resolves A's OWN store: the branch effect re-runs under
        // A's captured root (#136).
        show_late.set(true);
        a.update(&[]);
        let text = doc_text(&a);
        assert!(
            text.contains("store:1"),
            "A's late-built UI must read A's own store, not B's (#136); got: {text}"
        );

        // Remember A's root key so the post-drop check can look inside A's
        // (former) namespace.
        let a_root = a.app().doc().expect("A has a document").borrow().doc_key();

        // Dropping B clears only B's namespace — A keeps resolving its own
        // store when its branch rebuilds.
        drop(b);
        show_late.set(false);
        a.update(&[]);
        show_late.set(true);
        a.update(&[]);
        assert!(
            doc_text(&a).contains("store:1"),
            "after dropping B, A still resolves its own store; got: {}",
            doc_text(&a)
        );

        // Dropping A clears A's namespace: nothing is left under A's root
        // (the lookup below would fall back to root 0, where nothing ever
        // landed in this test either).
        drop(a);
        {
            let _root = rinch_core::push_context_root(a_root);
            assert!(
                try_use_store::<SharedStore>().is_none(),
                "A's namespaced store must be cleared when A drops (#136)"
            );
        }
        assert!(
            try_use_store::<SharedStore>().is_none(),
            "no store leaked into the thread-global root 0"
        );

        // Leave the shared worker thread clean for the other tests.
        rinch_core::clear_context();
    });
}

#[derive(Clone, Copy)]
struct GlobalStore {
    value: Signal<i32>,
}

/// The thread-global fallback (#136): a store created at top level — no root
/// pushed, exactly like shell-startup code — lands in root 0, and a mounted
/// context that did NOT create that store type still finds it through the
/// root-0 fallback. Dropping the context must not clear root-0 stores.
#[test]
fn store_root_zero_fallback() {
    on_ui_thread(|| {
        create_store(GlobalStore {
            value: Signal::new(42),
        });

        let mut c = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    p { {move || format!("global:{}", use_store::<GlobalStore>().value.get())} }
                }
            }
        });
        c.update(&[]);
        assert!(
            doc_text(&c).contains("global:42"),
            "a context that created no such store falls back to the \
             thread-global root-0 store (#136); got: {}",
            doc_text(&c)
        );

        // A root-0 store survives the context that merely read it.
        drop(c);
        assert!(
            try_use_store::<GlobalStore>().is_some(),
            "dropping a context must not clear thread-global (root 0) stores"
        );

        // Leave the shared worker thread clean for the other tests.
        rinch_core::clear_context();
    });
}

// ── 4. BOUNDS_REGISTRY per-document scoping ──────────────────────────────────

/// FIXED (#134): bounds-signal registry entries carry their document's
/// `doc_key`, and each context's `resolve_and_repaint` refreshes only its own
/// document's entries. Two documents with colliding node ids (both slabs start
/// at 0) no longer stomp each other's bounds signals.
#[test]
fn bounds_registry_crosstalk_across_documents() {
    on_ui_thread(|| {
        // Both components build the identical node structure (root div → inner
        // div → text), so the inner div gets the SAME raw node id in both
        // documents. A registers a bounds signal on ITS inner div (100px wide);
        // B's inner div is 200px wide and registers nothing.
        let a_bounds: Rc<Cell<Option<Signal<rinch_core::ElementBounds>>>> =
            Rc::new(Cell::new(None));
        let a_bounds_setter = a_bounds.clone();

        let dirty_a = Signal::new(0i32);
        let dirty_b = Signal::new(0i32);

        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            let root = __scope.create_element("div");
            let inner = __scope.create_element("div");
            inner.set_attribute("style", "width: 100px; height: 50px;");
            root.append_child(&inner);
            // Reactive text gives the test a deterministic way to dirty A's
            // DOM so a layout pass definitely runs. (Since the #134 fix, A's
            // dirty flag also stays connected while B exists — see test 2.)
            let text = __scope.create_text("0");
            root.append_child(&text);
            let text_handle = text.clone();
            __scope.create_effect(move || {
                text_handle.set_text(&dirty_a.get().to_string());
            });
            a_bounds_setter.set(Some(inner.bounds_signal()));
            root
        });

        let mut b = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            let root = __scope.create_element("div");
            let inner = __scope.create_element("div");
            inner.set_attribute("style", "width: 200px; height: 50px;");
            root.append_child(&inner);
            let text = __scope.create_text("0");
            root.append_child(&text);
            let text_handle = text.clone();
            __scope.create_effect(move || {
                text_handle.set_text(&dirty_b.get().to_string());
            });
            root
        });

        let bounds = a_bounds.get().expect("A registered its bounds signal");

        // A resolves: its inner div is 100px wide.
        dirty_a.set(1);
        a.update(&[]);
        assert_eq!(
            bounds.get().width,
            100.0,
            "after A's layout, A's bounds signal reports A's geometry"
        );

        // B resolves: its layout pass refreshes only B's registry entries, so
        // A's bounds signal — registered under A's doc_key — is untouched even
        // though B's document contains the same raw node id.
        dirty_b.set(1);
        b.update(&[]);
        assert_eq!(
            bounds.get().width,
            100.0,
            "B's layout pass must not touch A's bounds signal (#134: entries \
             are scoped by doc_key)"
        );

        // A's own next resolve still refreshes it normally — no flapping.
        dirty_a.set(2);
        a.update(&[]);
        assert_eq!(
            bounds.get().width,
            100.0,
            "A's bounds signal keeps reporting A's document geometry"
        );

        drop(a);
        drop(b);
    });
}
