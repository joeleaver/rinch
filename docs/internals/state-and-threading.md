# State, batching and threading

Read this page when your task touches: Signal/Memo/stores, event-handler batches, effect flushing, cross-thread dispatch.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## State Management

Component functions run **once** to build the DOM. Reactive closures (`{|| expr}`) in rsx handle all subsequent DOM updates surgically.

### Core Primitives

| Primitive | Purpose |
|-----------|---------|
| `Signal::new(value)` | Reactive state that triggers updates |
| `Memo::new(closure)` | Cached computed values (`T: Clone + PartialEq`; an equal recompute wakes nobody) |
| `batch(closure)` | One transaction: effects flush once at the end. Event handlers already run in one |
| `create_store(value)` | Share a store across components (recommended for shared state) |
| `use_store::<T>()` | Access a shared store (panics if missing) |
| `try_use_store::<T>()` | Try to access a shared store (returns Option<T>) |
| `create_context(value)` | Low-level shared state (used by framework internals) |
| `use_context::<T>()` | Access shared context (panics if missing) |

> **Note:** `Effect` is intentionally excluded from the prelude. Use `{|| expr}` in rsx for reactive DOM updates, and store methods for side effects. For rare advanced cases (syncing to external systems), import explicitly: `use rinch::reactive::Effect;`

### Basic Example

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates reactive DOM updates
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    App::new(app).title("My App").size(800, 600).run();
}
```

> **Important:** The closure syntax `{|| expr}` is required for fine-grained reactive updates. Without it, values are captured once at initial render and never update. See [RSX Syntax - Reactive Expressions](docs/src/guide/rsx-syntax.md#reactive-expressions). The `#[component]` macro injects `__scope` automatically, so the `rsx!` macro works without manually declaring it.

### Store Pattern (Recommended for Shared State)

A store is a struct with `Signal` fields and methods that encapsulate state mutations. Use `create_store()` / `use_store()` to share it across components:

```rust
#[derive(Clone, Copy)]
struct CounterStore {
    count: Signal<i32>,
}

impl CounterStore {
    fn new() -> Self {
        Self { count: Signal::new(0) }
    }
    fn increment(&self) {
        self.count.update(|n| *n += 1);
    }
}

#[component]
fn app() -> NodeHandle {
    create_store(CounterStore::new());
    rsx! { Counter {} }
}

#[component]
fn counter() -> NodeHandle {
    let store = use_store::<CounterStore>();
    rsx! {
        p { {|| store.count.get().to_string()} }
        button { onclick: move || store.increment(), "+" }
    }
}
```

For component-local state, just use `Signal::new()` directly — no store needed.

### Primitive Reference

**`Signal::new()`** - Reactive state:
```rust
let count = Signal::new(0);
count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function

// Cross-thread: send() auto-dispatches to the main thread
std::thread::spawn(move || {
    count.send(10);                       // T: Send required
    count.update_send(|n| *n += 1);       // closure must be Send + 'static
});
```

**`Memo::new()`** - Cached computed values:
```rust
let count = Signal::new(0);
let multiplier = Signal::new(2);

// Automatically tracks count and multiplier
let result = Memo::new(move || count.get() * multiplier.get());
```

**A memo has an equality cut-off, and its value type must be `PartialEq`**
(a breaking change; `Clone + PartialEq` is the bound on `Memo::new`/`get`/
`derived`). A recompute that yields an *equal* value wakes none of its
dependents, so `Memo::new(move || selected.get() == id)` per row re-runs two row
effects on a selection change, not every row's. The model is **push-dirty,
pull-value**: `Signal::notify` marks every memo reading the signal `Dirty` and
everything downstream `Check` *synchronously* (`mark_memos_stale`), nothing is
computed until a read; each memo carries a **version** that moves only on an
unequal recompute, every observer records the version it read in its
`DepKey::Memo { seen }`, and `flush_effects` runs an observer woken only through
a memo (a *maybe* — not in `Runtime::definite_effects`) only if one of those
versions moved. A memo's marker effect is never cut off (it cannot know what its
dependents saw) and is **not counted** in `effect_runs`. Skipping never reorders
anything, so the #154 contract holds for whatever runs
(`reactive/memo_cutoff_tests.rs`). A type with no meaningful equality can
implement `PartialEq` as always-`false` to get the old wake-everything behaviour.

**Event handlers are `batch()`es** — `events::dispatch_event`/`_input_`/
`_scroll_`/`_file_drop_`, the keyboard and paste interceptors, the dismiss
stack, `Drag` `on_move`/`on_end`/`on_cancel`, menu callbacks, focus-registry
callbacks, each drained main-thread callback (one batch **per callback**), each
timer, and web `rinch-http`/`rinch-ws` completions — on desktop, web and embed
alike. The guide lists what is **not** batched (`run_on_main_thread` on the main
thread, selection / configuration-change / child-observer callbacks, surface
events, `on_change`); keep that list exact. An outermost batch that wrote
no signal and finds nothing queued at its exit runs no flush and calls no
signal-change callback (#234: a no-op click posted a desktop `ReRender` and
dirtied every embed context); one that wrote notifies once even if its
effects ran mid-batch, and one that wrote nothing still flushes effects a
caught-panic batch left queued. The write test is the thread's
`SIGNAL_NOTIFIES` counter plus a host-change-owed flag, so every signal write
must keep bumping the counter, and **state outside the signal graph that a host
reads only when a signal-change callback tells it to** must call
`rinch_core::note_host_change()` when it changes — `set_current_theme_css`
(`rinch::update_theme`) does, or a no-write handler's theme change never
reached desktop or the web's theme `<style>` (PR #1134 review). The change is
*owed* until the next notification, so one made outside any batch is delivered
by the next handler of any kind. A theme change is also polled since #1144:
`RinchApp::theme_change_owed` (`theme_key` against `last_theme_key`) is folded
into `has_dirty_nodes` and `has_pending_layout`, so the Android loop, which
resolves only when `has_pending_layout` says so, applies a theme-only change
(it never did), and desktop's `AboutToWait` and an embed `update` apply one
with no notification (`theme_only_change_1144_tests.rs`). State a host polls itself (dirty nodes, focus requests,
the editor's owed overlay pass, image decodes, a queued
`NodeHandle::scroll_into_view` / `scroll_to_fraction` — #1145, which used to
wait for an unrelated layout) does not need it. Thread-local save/restore guards go through
`reactive::restore` (`RestoreCell<K: Slot>` over a `restore_slot!`, and
`with_runtime_on_drop`) — a new one should too; the slots dispatch statically
because a `fn`-pointer selector cost the effect-run path +2.4% instructions. A handler's
writes flush effects once, when it returns — **except that every `NodeHandle` operation from handler
code first runs the effects queued so far** (`NodeHandle::accessed_doc` →
`rinch_core::flush_pending_effects`), the analogue of a browser's forced style
flush. That keeps program order: `open.set(true); field.focus()` focuses after
the overlay effect's own `focus_into`, so the handler's request wins
(`app/batched_handler_focus_tests.rs`), and a reveal/insert-then-read or
-scroll sees the new DOM (`rinch-core/src/batch_dom_order_tests.rs`). The flush
lowers the batch flag while it runs (a normal flush) and raises it after; it is
a no-op outside a batch and inside any effect body or memo computation
(`REACTIVE_DEPTH`), whose queue is the running flush's own — except that an
**outermost** `batch()` (a handler dispatched from inside an effect) sets that
depth aside and is its own flush context — and under
`rinch_core::suppress_effect_flush()`. **Library code that touches the DOM while
holding a `RefCell` borrow of its own must hold that guard with the borrow** (and
call `flush_pending_effects()` just before borrowing, for program order), or a
user effect run by its own `NodeHandle` call re-enters it: `BorrowMutError`. The
editor routes every borrow of its core through `EditorHandle::core`/`core_mut`
(`CoreGuard`) for exactly this — only the mutable ones flush at entry, since a
query does no DOM work and must not run a user effect under the caller's own
borrows; a leaked guard trips a `debug_assert` at the outermost batch's exit; the PR #882 audit of every other such site is in
that PR. Memos are current
inside a batch (the eager marking above). A freed memo dependency counts as
*changed*. Guide: `docs/src/guide/reactivity.md#event-handlers-run-as-batches`.
On desktop the resulting signal-change callback queues at most **one** pending
`ReRender` (`shell::rinch_runtime::NativeEventQueue`, cleared under the queue
lock by the drain), and `resolve_and_repaint` detects a theme change from a
generation counter (`rinch_core::current_theme_css_generation`,
`RinchApp::theme_key`) instead of cloning and comparing the theme CSS string.

**`create_store` / `use_store`** - Shared state across components:
```rust
#[derive(Clone, Copy)]
struct AppStore {
    dark_mode: Signal<bool>,
    user_name: Signal<String>,
}

#[component]
fn app() -> NodeHandle {
    create_store(AppStore {
        dark_mode: Signal::new(false),
        user_name: Signal::new("Guest".into()),
    });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let store = use_store::<AppStore>();
    // For optional access: let store = try_use_store::<AppStore>(); // returns Option<T>
    // ...
}
```

**`create_context` / `use_context`** - Low-level shared state (still works, appropriate for framework internals):
```rust
#[derive(Clone)]
struct Theme { color: String }

#[component]
fn app() -> NodeHandle {
    create_context(Theme { color: "#007bff".into() });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let theme = use_context::<Theme>();
    // ...
}
```

## Threading Model

Rinch **owns the main thread**. `App::run()` calls winit's event loop, which takes over the thread and returns only once the application exits it (`close_current_window`, or an `on_close_requested` answering `true`). All UI state (`Signal`, `Effect`, `NodeHandle`, `RenderScope`) is `!Send` — the reactive system is thread-local.

**Async / tokio:** A tokio runtime can coexist but only on a **separate background thread**. You cannot run `#[tokio::main]` and `App::run()` on the same thread.

```rust
fn main() {
    // Spawn tokio on a background thread
    let rt = tokio::runtime::Runtime::new().unwrap();
    std::thread::spawn(move || {
        rt.block_on(async {
            // networking, file I/O, etc.
        });
    });

    // Main thread — rinch owns it
    App::new(app).title("My App").size(800, 600).run();
}
```

**Sending results back to the UI thread:** Use `Signal::send()` and `Signal::update_send()`, which auto-dispatch to the main thread from any thread. The `T` must be `Send`.

```rust
let data = Signal::new(Vec::new());

std::thread::spawn(move || {
    let result = fetch_data();     // blocking work
    data.send(result);             // dispatches to main thread
    data.update_send(|v| v.sort());// closure runs on main thread
});
```

**Key constraint:** Never call `Signal::set()` or `Signal::update()` from a background thread — they panic. Always use the `_send` variants for cross-thread updates.

**Embed works the same way** (issue #172). A `RinchContext` arms cross-thread
dispatch too, and drains the queued writes at the top of each `update()` —
before that frame's events, so a queued write and an event handler land in one
layout pass. `run_on_main_thread` and everything riding it (`set_timeout`,
`rinch-http`, `rinch-ws`) work in embed for the same reason. The queue itself
lives in `rinch-core` (`queue_main_callback` / `drain_main_callbacks`) so the
desktop shell, the Android loop and embed all share one; the shell's dispatcher
adds the "wake the event loop" side effect that embed has no use for. That wake
is coalesced (one pending `ReRender` at a time, in `NativeEventQueue`, re-armed
by every native drain), so a callback queued between a wake's callback drain
and its native-event drain would fold into the wake being served; the wake
therefore asks `rinch_core::main_callbacks_pending()` after the native drain
and owes itself another (`drain_wake_queues`, issue #988). **A `ReRender` sent
before `GLOBAL_PROXY` exists carries no wake** (#1540) — the mount's
signal-change callback sends one, and so does a worker's early `send` — and it
holds that coalescing flag up, so every later `ReRender` coalesces into it and
wakes nothing. A window that paints drains it at its first frame; a windowless
or hidden app (tray-only, started hidden) never did, and stopped updating.
`install_global_proxy` pays the owed wake right after setting the proxy
(`owe_startup_wake`). Every dispatch also asks for its `ReRender` rather than
only a push onto an *empty* main queue: the queue can be non-empty with no wake
owed for it (a closure pushed past the dispatcher, or queued before the
dispatcher existed), and under the empty-queue rule every later sender asked
for nothing. Hosts install the dispatcher **before** registering the main
thread: in the other order a worker sending in between found a main thread and
no dispatcher, and panicked. **Library code
must not push onto the queue with a bare `queue_main_callback`** where a host
may be waking: that closure gets no wake of its own and waits for the next
sender's (issue #1035 — a video sink dropped off-thread did this; before #1540
the next sender asked for none either).
**A worker that sends before the host registers its main thread** (spawned in
`main()` ahead of `App::run`) has its `send`/`update_send` queued through
`dispatch_main_callback` and applied once the host starts (#1541): with no main
thread registered, a thread whose signal store has never allocated cannot own
the signal (`reactive::unregistered_write_is_local`). It used to take the
main-thread branch (`is_main_thread()` answers `true` while unregistered) and
write its own empty store — dropped. The queued write is applied only when
drained on the registered main thread: signal ids alias across thread-local
stores, so a drain on any other thread (a second test, a host-less helper)
drops it rather than writing a stranger's signal. A thread that has allocated signals keeps
applying in place, which is the web's and a test's main thread; a worker that
creates signals of its own before the host starts still writes its own store.
Only the unregistered case pays the store check.
Use `rinch_core::dispatch_main_callback`, which goes through the registered
dispatcher (or queues plainly when there is none) and, unlike
`run_on_main_thread`, does not panic for a missing dispatcher and never runs
inline — the shape a `Drop` needs. (The queue and the desktop dispatcher it
reaches still `unwrap` their own locks.) A host's own dispatcher is the one place a bare push belongs.
