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

### Intervals and polled signals off the native frame loop (#1773)

`set_interval` (`rinch-core/src/timer.rs`) is a chain of `set_timeout` ticks,
so it needs no backend of its own: a thread-local `INTERVALS` map holds each
interval's pending `TimeoutHandle` (the handle the app keeps is a stable id),
each tick re-arms from its own deadline (fixed rate; more than a period late it
re-arms from now), and `on_cleanup` drops the entry when the arming component
unmounts. `poll_signal` is drained by the native hosts once per frame; the web
has no frame loop, so `poll_signal` calls a waker hook
(`reactive::set_poll_waker`) after each registration and `rinch/src/web_runtime/polls.rs`
keeps one drain scheduled (rAF while `polls_want_every_frame()`, else
`setTimeout(next_poll_due())`, nothing when the registry is empty), each drain
inside a `batch`. Pins: `timer::tests` (`an_interval_*`, the two rate tests),
`poll::waker_tests`, `rinch-web/tests/poll_signal_1773.rs`.

### The web runtime without a root (#1956)

The browser's three scheduling halves live in `rinch/src/web_runtime/`
(wasm32 only): the `window.setTimeout` timer backend (`set_timer_backend`),
the `queueMicrotask` task scheduler (`set_task_scheduler`) and the poll drain
above. They are in `rinch`, not `rinch-web`, because `rinch-web` depends on
`rinch` and an embed-on-web app depends on `rinch` alone. Three callers:

- `rinch::install_web_runtime()` installs all three, idempotently (per-thread
  flags), with no DOM work; `rinch_web::install_runtime()` is the same call.
  Off wasm32 both exist as no-ops (native supplies all three itself), so a
  cross-platform app calls them with no cfg of its own.
- `rinch-web`'s `ensure_global_init` (every `mount*`) calls it, then does the
  mount-only page work: clearing stale handlers and contexts, the theme
  `<style>` hook. One install path for mounts and root-less pages.
- `RinchContext::new` on wasm32 calls `install_web_scheduling()` (timers and
  tasks, not polls) before its mount: `update()` already drains polls once a
  frame, and the rAF drain on top would sample an `EveryFrame` poll twice. Without
  it an embed-only page's timers never fired, and its tasks were polled only by
  the next `update()`'s main-queue drain.

A `set_timeout` armed before any install is lost (the backend-less branch of
`timer::schedule` warns and drops it); a task spawned before is picked up.
Pins: `rinch-web/tests/install_runtime_1956.rs` (no mount: control, install
twice, no DOM, timers, `sleep`, polls, then a mount). The `RinchContext` call
has no committed browser test: `rinch-web`'s tests build `rinch` without
`embed`, and turning it on there would change every test's `rinch`.

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

## Futures on the UI thread (`rinch_core::task`, #1767)

`spawn_local` keeps tasks in a main-thread-local table keyed by id
(`TASKS`); a task's `Waker` is an `Arc` holding the id and a `scheduled` flag,
so it is `Send + Sync` and coalesces wakes into one poll. A wake calls the
installed scheduler (`set_task_scheduler`; on wasm32 a `queueMicrotask` one,
installed by `rinch::web_runtime` — see below) or, with none, queues
`poll_task(id)` through `dispatch_main_callback` — the queue the desktop and
Android loops drain and embed drains in `update()`. Installing a scheduler
re-schedules every task still waiting for a poll, so a task spawned before the
first web mount (or `install_runtime`) is not stranded on a queue a browser
never drains.
`poll_task` takes the task out of the table (a `Slot::Running` marker stays,
which records an abort or a nested drain's poll that arrives during the poll),
polls it in `batch`, under its owner (or `unowned`) and its `CallbackFrame`,
then puts it back or drops it outside every borrow. Lifetime: one
`on_cleanup` per owning scope (`OWNER_TASKS`, keyed by `Owner::key`, the
owner kept beside the ids so the address is not reused), which aborts — drops
— the scope's live tasks at unmount; a finished task leaves its owner's set.
At thread exit the table forgets pending futures instead of dropping them,
because their `Drop` reaches for thread-locals that may already be gone (a
panic in a TLS destructor aborts); `cancel_main_callback` reads its registry
with `try_with` for the same reason. `Sleep` arms `set_timeout` (unowned) on
first poll and clears it on drop. `rinch_http::fetch_async` and
`rinch_ws::WsStream` are thin adapters over the callback APIs (their callbacks
registered `unowned`: the future's holder decides the lifetime). Pins:
`rinch-core/tests/task_1767.rs`, `task_default_wake_1767.rs`,
`rinch-http/tests/fetch_async_1767.rs`, `rinch-ws/tests/stream_1767.rs`,
`rinch-web/tests/spawn_local_1767.rs` (Chrome 155).

`rinch_ws::WsRequest` (#1773) is what `connect(url)` builds: the subprotocols
are validated by the browser's rule in `lib.rs` (`validate_protocols`) on both
targets, then handed to `WebSocket::new_with_str_sequence` on the web and sent
as one `Sec-WebSocket-Protocol` header joined with a bare `,` on native
(tungstenite reads its own request back by splitting on `,` untrimmed, so `, `
would make it refuse the server's answer). The native worker writes the chosen
protocol into the `Backend`'s `Arc<Mutex<String>>` before it emits `Open`; the
web backend reads `WebSocket.protocol`. Headers exist on native only (a
`#[cfg]`'d method), appended (a repeated name is sent twice), the handshake's
own refused; the worker connects with `connect_with_config(.., 0)` because
tungstenite's `connect` follows up to 3 redirects and copies every header to
the `Location` target (`wss` to `ws` included), which a browser never does. tungstenite fails a handshake whose server chose none of
the offered protocols, where a browser opens; pinned in
`rinch-ws/tests/subprotocols_1773.rs`. Web pins: `web_subprotocols_1773.rs`,
whose live half runs only when built with `RINCH_WS_TEST_URL` (CI sets none).

`WsHandle::drop` unregisters the connection's callbacks from the thread-local
`HANDLERS` table with `try_with`, and drops them after the borrow ends (#1952).
A handle kept in an app's own `thread_local!`, or captured by another
connection's callback (which `HANDLERS` itself drops as it is destroyed), is
dropped during thread-local teardown with the table already gone; `with`
panicked there, and a panic in a TLS destructor aborts the process on exit.
Dropping the callbacks inside the borrow made the second shape panic outside
exit too: the captured handle's own drop re-borrowed the table. Pins:
`rinch-ws/tests/drop_at_exit_1952.rs` (the app-local shape, the forced
table-teardown shape, and the ordinary nested drop).

## Background workers (`rinch::worker`, #1772)

Feature `worker` (off by default). `crates/rinch/src/worker/`: `mod.rs` is the
platform-neutral half, `native.rs` and `web.rs` the backends. A `Worker` is an
`Rc<Inner>` listed in a UI-thread-local `WORKERS` map (id → type-erased
receiver); a backend hands the UI thread a `Raw` (`Reply` bytes, a recoverable
`Error`, or `Failed`, which ends the worker) and `deliver(id, raw)` finds the
receiver, or drops the event when the worker has ended. The receiver decodes
(`WorkerMessage::from_bytes`) and calls the `on_event` callback — in `batch`,
under the callback's owner (or `unowned`) and `CallbackFrame`, dropped once
its owner is dead — or queues it for `next()` and wakes the waiting task.
`spawn` registers one `on_cleanup` that ends the worker (unmount), and `Drop`
ends it too. A callback found with a dead owner is taken and sets
`discarding`, which drops later events (instead of queueing them for a `next`
nobody may call) until the next `on_event`. `next()` keeps a list of wakers,
all woken per event. `Worker::spawn` inside a worker (`is_worker()`) returns an
already-ended worker holding one `Error`: on the web the app's start function
re-runs in every instance and would otherwise spawn itself recursively. A
worker thread and a worker instance call `rinch_core::task::__enter_worker_context`
(desktop: at thread start; web: in `__rinch_worker_run`), after which
`spawn_local`, `set_timeout` and `set_interval` panic (`refuse_in_worker`) — nothing drives
tasks or timers there (on the desktop the task table and timer queue are the
UI thread's; a web worker instance has no `window`, so even an
`install_web_runtime` the app's start function re-runs there schedules
nothing).
A startup failure on the web is reported from a `queueMicrotask`, not a rinch
task: before the web runtime is installed (the first mount, a
`RinchContext`, or `install_web_runtime`) no task scheduler is installed. Ending unlists the id first, so a reply already on its way is
dropped on the UI side whatever the backend does.

**Desktop**: one thread per worker (`rinch-worker-<id>`), an `mpsc` channel in,
`dispatch_main_callback` out — so replies arrive where `Signal::send`'s do, in
order. The handler runs under `catch_unwind`; a panic posts `Failed` and ends
the thread. `terminate` sets a shared `AtomicBool` (read by
`Replies::is_terminated` and checked before each message and each reply) and
drops the sender: cooperative, the running handler is not interrupted. Messages
go through bytes here too (Joe's decision, recorded on #1772), so a lossy codec
fails on both backends.

**Web**: shared memory is not available — `+atomics` needs `-Z build-std`, i.e.
nightly (measured on 1.99.0: `rust-lld: --shared-memory is disallowed by std …
not compiled with 'atomics'`) — so each worker is a second instance of the app's
own module. The UI creates a module `Worker` from a `blob:` loader (`LOADER`,
one URL per page, never revoked) and posts `{glue, module, shim, handler}`:
`glue` is `import.meta.url` read through a `js_namespace = ["import", "meta"]`
static (so the glue must be an ES module, `--target web`), `module` is
`wasm_bindgen::module()` (the compiled module, structured-cloned), and `shim` /
`handler` are `fn` addresses. The loader `import()`s the glue and instantiates
the module — which runs the app's start function; `rinch_web::mount` and
`mount_with_menu_bar` return when there is no document (`page_document()`) —
and calls the exported `__rinch_worker_run(shim, handler, bytes)` per message,
queuing those that arrive before the instance is up. `__rinch_worker_run` turns
the addresses back into `fn`s (`fn_at`, the one `unsafe`: the instances are the
same module and share one function table) and the monomorphised `run_shim`
decodes and calls the handler. Bytes travel as transferred `ArrayBuffer`s; a
decode failure posts `{rinchError}`, a thrown call (a panic traps the instance)
or a failed start posts `{rinchFailed}` and closes the worker, and the worker's
`error` event (the loader itself refused, e.g. by a CSP without `blob:`) is
`Failed` too. `terminate` is `Worker.terminate()`: Firefox stops a busy
worker at once, Chrome after its ~2 s forcible-termination delay (measured: a
1.5 s busy loop runs to its end, a 5 s one does not). Pins:
`crates/rinch/tests/worker_1772.rs` (desktop), `crates/rinch-web/tests/worker_1772.rs`
(Chrome 155 and Firefox 155; a `BroadcastChannel` shows whether a terminated
handler reached its end), `crates/rinch-web/tests/worker_start_failure_1772.rs`
(a failed start before any mount). CI runs the desktop file through its own
line in the feature-gated step (`[[test]] required-features = ["worker"]`).
