//! Empirical pinning tests for issues #134/#136: two `RinchContext`s on one
//! thread.
//!
//! Each test constructs headless embed contexts (no window, no GPU — `scene()`
//! is never called) and pins the behavior of the shared thread-local state:
//! per-context signal subscriptions and bounds registries (#134) and per-root
//! store/context namespacing with the thread-global root-0 fallback (#136).
//!
//! Requires the `embed` (or `gpu`) feature. Since #140 the painters are
//! additive, so `desktop` + `embed` co-compiles and default features can stay on:
//!     cargo test -p rinch --features embed --test multi_context
//! The per-document theme tests (#138) additionally need the `theme` feature:
//!     cargo test -p rinch --features embed,theme --test multi_context
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

#[derive(Clone, Copy)]
struct MemoStore {
    which: Signal<i32>,
}

/// FIXED (#141): a `Memo` re-enters its own creation root when it recomputes.
///
/// A memo is *lazy* — the user computation does not run in its dirty-marker
/// effect (which carries a root) but at whichever call site first reads it after
/// invalidation. So capturing a root on the marker, as #136 did for effects, is
/// not enough: it is inert for the computation. Here the memo is created inside
/// context A but never read there, then read for the first time from context B.
/// Before the fix the computation ran under B's root and resolved B's store,
/// silently returning 2.
#[test]
fn memo_recompute_resolves_its_creation_context_store() {
    on_ui_thread(|| {
        // Carries the Copy memo handle out of A and into B.
        let slot: Rc<std::cell::RefCell<Option<Memo<i32>>>> =
            Rc::new(std::cell::RefCell::new(None));

        let a_slot = slot.clone();
        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            create_store(MemoStore {
                which: Signal::new(1),
            });
            // Created under A's root — but deliberately NOT read here, so the
            // computation is still pending when B mounts.
            *a_slot.borrow_mut() = Some(Memo::new(|| use_store::<MemoStore>().which.get()));
            rsx! { div { p { "A" } } }
        });
        a.update(&[]);

        let b_slot = slot.clone();
        let mut b = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            create_store(MemoStore {
                which: Signal::new(2),
            });
            let memo = b_slot.borrow().expect("A created the memo");
            rsx! {
                div {
                    p { {move || format!("memo:{}", memo.get())} }
                }
            }
        });
        b.update(&[]);

        let text = doc_text(&b);
        assert!(
            text.contains("memo:1"),
            "a memo created in A must resolve A's store even when first read \
             from B (#141); got: {text}"
        );

        drop(b);
        drop(a);
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

// ── 5. per-document theme CSS (#138) ─────────────────────────────────────────

/// Config with an explicit light/dark theme (requires the `theme` feature).
#[cfg(feature = "theme")]
fn themed_cfg(dark_mode: bool) -> RinchContextConfig {
    RinchContextConfig {
        theme: Some(rinch::core::element::ThemeProviderProps {
            dark_mode,
            ..Default::default()
        }),
        ..cfg()
    }
}

/// The computed background color (Debug-formatted) of a context's `.probe`
/// element. The probe paints `var(--rinch-color-body)`, which the light and
/// dark themes resolve to different colors — an observable for which theme CSS
/// the document actually holds.
#[cfg(feature = "theme")]
fn probe_bg(ctx: &RinchContext) -> String {
    let doc = ctx.app().doc().expect("context has a document").clone();
    let d = doc.borrow();
    let id = *rinch_dom::testing::query_selector(&d.tree, ".probe")
        .first()
        .expect("probe element exists");
    format!(
        "{:?}",
        d.tree
            .get(id)
            .expect("probe node")
            .computed_style
            .background_color()
    )
}

/// FIXED (#138): each embed context owns its theme CSS per document. Creating
/// context B (light) no longer overwrites the thread-global theme slot that A
/// (dark) was compared against, so A's next resolve keeps A's own theme
/// instead of silently adopting B's.
#[cfg(feature = "theme")]
#[test]
fn creating_a_context_does_not_restyle_another() {
    on_ui_thread(|| {
        let sig_a = Signal::new(0i32);
        let mut a = RinchContext::new(themed_cfg(true), move |__scope: &mut RenderScope| {
            rsx! {
                div { class: "probe", style: "background-color: var(--rinch-color-body);",
                    p { {move || format!("A:{}", sig_a.get())} }
                }
            }
        });
        a.update(&[]);
        let dark_bg = probe_bg(&a);

        let mut b = RinchContext::new(themed_cfg(false), |__scope: &mut RenderScope| {
            rsx! {
                div { class: "probe", style: "background-color: var(--rinch-color-body);",
                    "B"
                }
            }
        });
        b.update(&[]);
        let light_bg = probe_bg(&b);
        assert_ne!(
            dark_bg, light_bg,
            "sanity: light and dark themes must resolve --rinch-color-body differently"
        );

        // Drive A through a real update AFTER B was created — on the old
        // thread-slot path this is where A compared the slot (holding B's
        // light CSS) against its cached dark CSS and adopted B's theme.
        sig_a.set(1);
        a.update(&[]);
        assert!(
            doc_text(&a).contains("A:1"),
            "A still renders: {}",
            doc_text(&a)
        );
        assert_eq!(
            probe_bg(&a),
            dark_bg,
            "creating context B must not restyle A's document (#138)"
        );
        assert_eq!(probe_bg(&b), light_bg, "B keeps its own light theme");

        drop(a);
        drop(b);
    });
}

/// `RinchContext::set_theme` restyles only its own document (#138): A flips to
/// dark on its next update; B — created with the same light theme — is
/// untouched.
#[cfg(feature = "theme")]
#[test]
fn set_theme_restyles_only_its_own_context() {
    on_ui_thread(|| {
        let mut a = RinchContext::new(themed_cfg(false), |__scope: &mut RenderScope| {
            rsx! {
                div { class: "probe", style: "background-color: var(--rinch-color-body);",
                    "A"
                }
            }
        });
        let mut b = RinchContext::new(themed_cfg(false), |__scope: &mut RenderScope| {
            rsx! {
                div { class: "probe", style: "background-color: var(--rinch-color-body);",
                    "B"
                }
            }
        });
        a.update(&[]);
        b.update(&[]);
        let light_bg = probe_bg(&a);
        assert_eq!(probe_bg(&b), light_bg, "both start on the light theme");

        a.set_theme(&rinch::core::element::ThemeProviderProps {
            dark_mode: true,
            ..Default::default()
        });
        a.update(&[]);
        b.update(&[]);

        let dark_bg = probe_bg(&a);
        assert_ne!(dark_bg, light_bg, "set_theme must restyle A's document");
        assert_eq!(
            probe_bg(&b),
            light_bg,
            "set_theme on A must not touch B (#138)"
        );

        drop(a);
        drop(b);
    });
}

/// A bounds-driven effect that **mutates the DOM** must not deadlock the
/// document `RefCell` (#141).
///
/// `NodeHandle::bounds_signal`'s own docs recommend reading a measured width
/// from a reactive `style:` closure. The `rsx!` macro compiles that to an
/// `Effect` calling `set_attribute`, which takes `doc.borrow_mut()`. The runtime
/// used to publish bounds while still holding `doc.borrow()`, so the very first
/// layout pass panicked with `RefCell already borrowed`.
#[test]
fn a_bounds_driven_effect_may_patch_the_dom() {
    on_ui_thread(|| {
        let observed_width: Rc<Cell<f32>> = Rc::new(Cell::new(-1.0));
        let seen = observed_width.clone();
        // As in `bounds_registry_crosstalk_across_documents`: reactive text is
        // the deterministic way to dirty the DOM so a layout pass definitely
        // runs and `refresh_bounds_signals` is reached.
        let dirty = Signal::new(0i32);

        let mut ctx = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            let root = __scope.create_element("div");
            let measured = __scope.create_element("div");
            measured.set_attribute("style", "width: 120px; height: 40px;");
            root.append_child(&measured);

            let bar = __scope.create_element("div");
            root.append_child(&bar);

            let text = __scope.create_text("0");
            root.append_child(&text);
            let text_handle = text.clone();
            __scope.create_effect(move || {
                text_handle.set_text(&dirty.get().to_string());
            });

            // The documented idiom: react to the measured rect by patching
            // another node's style.
            let bounds = measured.bounds_signal();
            let bar_handle = bar.clone();
            let seen = seen.clone();
            __scope.create_effect(move || {
                let b = bounds.get();
                seen.set(b.width);
                bar_handle.set_attribute("style", &format!("width: {}px;", b.width));
            });

            root
        });

        // Pre-fix this panicked inside `refresh_bounds_signals` with
        // "RefCell already borrowed".
        dirty.set(1);
        ctx.update(&[]);

        assert_eq!(
            observed_width.get(),
            120.0,
            "the effect ran and saw the measured width"
        );

        drop(ctx);
    });
}
