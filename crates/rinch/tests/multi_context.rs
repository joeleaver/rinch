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

// ── the root mount owns the tree it builds (#141) ────────────────────────────

/// The root component build runs under the root `RenderScope`'s ambient owner,
/// and that owner is popped before mounting returns.
///
/// This pins the cross-crate half of #141 PR2: the guard lives in
/// `RinchApp::mount_component`, so it cannot be exercised from `rinch-core`.
/// It is deliberately narrower than the surrounding `_root_guard` — the owner
/// must not cover initial layout, caret updates or scroll-clamp dispatch, none
/// of which belong to the component tree.
#[test]
fn the_root_build_is_attributed_to_the_root_scope() {
    on_ui_thread(|| {
        assert!(
            rinch::current_owner().is_none(),
            "no ambient owner before mounting"
        );

        let seen: Rc<std::cell::RefCell<Option<rinch::Owner>>> =
            Rc::new(std::cell::RefCell::new(None));
        let log = seen.clone();

        let mut ctx = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            *log.borrow_mut() = rinch::current_owner();
            Signal::new(0i32);
            rsx! { div { "owned" } }
        });
        ctx.update(&[]);

        assert!(
            rinch::current_owner().is_none(),
            "the mount guard must not outlive the build"
        );

        let owner = seen
            .borrow()
            .clone()
            .expect("the root component must build under an ambient owner");
        assert!(owner.is_alive(), "the root scope outlives the mount");
        assert_eq!(
            owner.owned_counts().map(|c| c.signals),
            Some(1),
            "a signal created by the root component belongs to the root scope"
        );

        drop(ctx);
        assert!(
            !owner.is_alive(),
            "dropping the context disposes the root scope"
        );
    });
}

// ── pointer-capture drags across contexts (issue #293) ───────────────────────

/// A press in context B that arms a drag while context A's drag is live ends
/// A's drag through its `on_cancel`, rather than dropping it with its teardown
/// unrun. Driven through real `update()` calls, so the dispatching-document
/// marker is the one `RinchApp::handle_event` pushes, not a test's.
#[test]
fn a_drag_armed_in_one_context_cancels_another_contexts_live_drag() {
    on_ui_thread(|| {
        use rinch_platform::{MouseButton, PlatformEvent};

        #[derive(Default)]
        struct Log {
            cancels: Cell<u32>,
            ends: Cell<u32>,
        }
        let a_log = Rc::new(Log::default());
        let b_log = Rc::new(Log::default());

        fn arming_root(log: Rc<Log>) -> impl Fn(&mut RenderScope) -> NodeHandle + 'static {
            move |__scope: &mut RenderScope| {
                let log = log.clone();
                rsx! {
                    div {
                        style: "width: 300px; height: 300px;",
                        onclick: move || {
                            let c = log.clone();
                            let e = log.clone();
                            rinch_core::Drag::absolute()
                                .on_cancel(move |_, _| c.cancels.set(c.cancels.get() + 1))
                                .on_end(move |_, _| e.ends.set(e.ends.get() + 1))
                                .start();
                        },
                    }
                }
            }
        }

        let mut a = RinchContext::new(cfg(), arming_root(a_log.clone()));
        let mut b = RinchContext::new(cfg(), arming_root(b_log.clone()));
        a.update(&[]);
        b.update(&[]);

        let press = PlatformEvent::MouseDown {
            x: 50.0,
            y: 50.0,
            button: MouseButton::Left,
        };
        let release = PlatformEvent::MouseUp {
            x: 60.0,
            y: 60.0,
            button: MouseButton::Left,
        };

        a.update(std::slice::from_ref(&press));
        assert!(
            rinch_core::Drag::is_active(),
            "positive control: A's press armed a drag"
        );
        b.update(std::slice::from_ref(&press));
        assert_eq!(
            a_log.cancels.get(),
            1,
            "B's arm ended A's drag through on_cancel"
        );

        // Only B's release may commit anything now.
        a.update(std::slice::from_ref(&release));
        assert_eq!(a_log.ends.get(), 0, "a superseded drag never commits");
        b.update(std::slice::from_ref(&release));
        assert_eq!(b_log.ends.get(), 1, "B's own drag commits on B's release");
        assert_eq!(b_log.cancels.get(), 0);

        drop(a);
        drop(b);
    });
}

// ── effects flush under their own document (issue #295) ─────────────────────

/// An effect owned by context B, woken by a write made in context A's event
/// handler, runs under **B**'s document — not under A's, which is the one
/// dispatching when the thread-global effect queue drains. And B's mount runs
/// under B's document too, which is where that effect learned its document.
///
/// The observable consumer is the pointer-capture drag (#139/#293): a drag the
/// effect arms belongs to the document it records, so before #295 B's own
/// pointer moves could not drive it and A's could.
#[test]
fn an_effect_woken_from_another_contexts_handler_runs_under_its_own_document() {
    on_ui_thread(|| {
        use rinch::reactive::Effect;
        use rinch_platform::{MouseButton, PlatformEvent};
        use std::cell::RefCell;

        // Written from A's handler, read by B's effect. Created outside both
        // contexts, so it belongs to neither.
        let go = Signal::new(false);

        #[derive(Default)]
        struct Seen {
            b_click: Cell<Option<u64>>,
            b_mount: Cell<Option<u64>>,
            b_effect: RefCell<Vec<Option<u64>>>,
            b_moves: Cell<u32>,
        }
        let seen = Rc::new(Seen::default());

        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    style: "width: 300px; height: 300px;",
                    onclick: move || go.set(true),
                }
            }
        });
        let s = seen.clone();
        let mut b = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            s.b_mount.set(rinch_core::current_dispatching_doc());
            let e = s.clone();
            // Leaked into the root scope's lifetime by the scope that owns it.
            let _effect = Effect::new(move || {
                let armed = go.get();
                e.b_effect
                    .borrow_mut()
                    .push(rinch_core::current_dispatching_doc());
                if armed {
                    let m = e.clone();
                    rinch_core::Drag::absolute()
                        .on_move(move |_, _| m.b_moves.set(m.b_moves.get() + 1))
                        .start();
                }
            });
            let c = s.clone();
            rsx! {
                div {
                    style: "width: 300px; height: 300px;",
                    onclick: move || c.b_click.set(rinch_core::current_dispatching_doc()),
                }
            }
        });
        a.update(&[]);
        b.update(&[]);

        let press = PlatformEvent::MouseDown {
            x: 50.0,
            y: 50.0,
            button: MouseButton::Left,
        };
        let release = PlatformEvent::MouseUp {
            x: 50.0,
            y: 50.0,
            button: MouseButton::Left,
        };
        // B's own click tells us B's document key (the oracle).
        b.update(&[press.clone(), release.clone()]);
        let b_doc = seen.b_click.get();
        assert!(b_doc.is_some(), "positive control: B's click dispatched");

        // A's click writes `go`; B's effect flushes inside A's dispatch.
        a.update(std::slice::from_ref(&press));
        assert!(
            rinch_core::Drag::is_active(),
            "positive control: the effect armed a drag"
        );
        assert_eq!(
            *seen.b_effect.borrow(),
            vec![b_doc, b_doc],
            "B's effect ran under B's document at mount and when A woke it"
        );
        assert_eq!(
            seen.b_mount.get(),
            b_doc,
            "B's mount ran under B's document"
        );

        // The drag belongs to B: A's moves do not drive it, B's do.
        let mv = PlatformEvent::MouseMove { x: 70.0, y: 70.0 };
        a.update(std::slice::from_ref(&mv));
        assert_eq!(
            seen.b_moves.get(),
            0,
            "A's pointer stream is not B's drag's"
        );
        b.update(std::slice::from_ref(&mv));
        assert_eq!(seen.b_moves.get(), 1, "B's pointer stream drives B's drag");

        rinch_core::Drag::cancel();
        drop(a);
        drop(b);
    });
}

// ── REVIEW-960 probes ──
fn rv960_key() -> rinch_platform::PlatformEvent {
    use rinch_platform::{KeyCode, KeyRepeat, Modifiers, PlatformEvent};
    PlatformEvent::KeyDown {
        key: KeyCode::KeyQ,
        logical_key: Some("q".into()),
        text: Some("q".into()),
        modifiers: Modifiers::default(),
        repeat: KeyRepeat::Fresh,
    }
}

/// Single document: a mount-time interceptor, then a re-registration from
/// outside any dispatch (a timer / menu / run_on_main_thread callback).
#[test]
fn rv960_outside_reregistration_replaces_mount_interceptor() {
    on_ui_thread(|| {
        use std::cell::RefCell;
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let h = hits.clone();
        let mut a = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            let h = h.clone();
            rinch_core::events::set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("mount");
                false
            });
            rsx! { div { style: "width: 300px; height: 300px;" } }
        });
        a.update(&[]);
        a.update(&[rv960_key()]);
        let h2 = hits.clone();
        // Outside any dispatch, e.g. a set_timeout callback.
        rinch_core::events::set_keyboard_interceptor(move |_| {
            h2.borrow_mut().push("outside");
            false
        });
        a.update(&[rv960_key()]);
        rinch_core::events::clear_keyboard_interceptor();
        a.update(&[rv960_key()]);
        let got = hits.borrow().clone();
        rinch_core::events::clear_keyboard_interceptor();
        rinch_core::events::clear_keyboard_interceptor();
        drop(a);
        assert_eq!(
            got,
            vec!["mount", "outside"],
            "last registration wins; clear clears"
        );
    });
}
