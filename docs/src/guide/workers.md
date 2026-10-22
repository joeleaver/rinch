# Background Workers

Work that would freeze the UI — parsing a large file, evaluating a user's
script, a long computation — can run in a **worker**: a handler that runs off
the UI thread and sends its replies back to it. The same code runs on the
desktop (a thread) and on the web (a Web Worker). Enable the `worker` feature:

```toml
rinch = { workspace = true, features = ["desktop", "worker"] }
```

```rust
use rinch::prelude::*;
use rinch::worker::{Replies, Worker, WorkerEvent};

/// Runs in the worker, once per message.
fn count_words(text: String, replies: &Replies<String>) {
    replies.send(text.split_whitespace().count().to_string());
}

#[component]
fn word_count() -> NodeHandle {
    let words = Signal::new(String::new());
    let worker = Worker::<String, String>::spawn(count_words);
    worker.on_event(move |event| match event {
        WorkerEvent::Message(n) => words.set(n),
        WorkerEvent::Error(e) => words.set(format!("failed: {e}")),
        _ => {}
    });
    worker.send(load_document());
    rsx! { p { {|| words.get()} } }
}
```

- **`Worker::<In, Out>::spawn(handler)`** starts a worker. The handler is a
  plain `fn(In, &Replies<Out>)`, not a closure: on the web it runs in another
  instance of the app's module, where nothing a closure captured exists. It is
  called once per message, in the order the messages were sent. State it keeps
  between messages goes in a `thread_local!` — each worker has its own.
- **`worker.send(msg)`** queues a message; **`replies.send(msg)`** in the
  handler sends one back.
- **`worker.on_event(|event| …)`** receives the replies on the UI thread, one
  `batch` each. Registered while a component renders, it stops when the
  component unmounts, and the worker's events are dropped from then on until
  `on_event` is called again. **`worker.next().await`** receives them inside a
  [task](./async.md) instead (events that arrive while a callback is set do
  not reach it); it yields `None` once the worker has ended. Several tasks may
  await `next()` at once: each event goes to one of them, and all see the end.
- **`WorkerEvent::Error`** reports a message that could not be decoded (the
  worker goes on), or a handler that panicked or a worker that could not start
  (the worker has ended; nothing follows).
- **`worker.terminate()`** stops it (below). A worker spawned while a
  component renders is terminated when the component unmounts, and dropping the
  `Worker` terminates it too.
- **`rinch::worker::is_worker()`** says whether the calling code runs inside a
  worker.

## Handlers are synchronous

A handler runs to completion for each message; nothing in a worker runs rinch
tasks or timers. `spawn_local` and `set_timeout` called inside a handler
**panic** (the worker then ends with a `WorkerEvent::Error`) rather than
silently never running. Do the work in the handler, and send a reply for each
step of progress you want the UI to see.

A worker cannot spawn workers: `Worker::spawn` inside a worker starts nothing,
and returns a `Worker` that has already ended, whose `next()` yields one
`WorkerEvent::Error` saying why.

## Messages

Messages cross as bytes, through the `WorkerMessage` trait, which `String` and
`Vec<u8>` implement. For structured messages, implement it with the codec you
already use:

```rust
impl WorkerMessage for Query {
    fn into_bytes(self) -> Vec<u8> { serde_json::to_vec(&self).unwrap() }
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> { serde_json::from_slice(&bytes).ok() }
}
```

They go through bytes on the desktop as well, so a codec that loses something
loses it there too, not only in the browser.

## Stopping a worker, with a deadline

```rust
let worker = Rc::new(Worker::<String, String>::spawn(evaluate));
let w = worker.clone();
spawn_local(async move {
    sleep(Duration::from_secs(2)).await;
    w.terminate(); // a `next()` still waiting now yields `None`
});
worker.send(script);
```

After `terminate()` no event arrives, messages not yet handled are dropped, and
a pending `next()` resolves to `None`. What happens to a handler that is
**running** differs:

- **Web**: the Web Worker is stopped. Firefox stops it at once; Chrome lets a
  handler that never yields run for up to about two seconds before stopping it
  (its forcible-termination delay). Its replies are dropped either way.
- **Desktop**: a thread cannot be stopped from outside, so termination is
  **cooperative**: the handler keeps running until it returns. A handler that
  may run long should check `replies.is_terminated()` and return — a script
  interpreter's interrupt hook is the natural place.

## On the web

The worker is a Web Worker running a **second instance of the app's own wasm
module**: no second build, no extra file. What that requires and implies:

- The wasm-bindgen glue must be an ES module: `--target web`, which is what
  trunk and `wasm-bindgen-test-runner` use. `--target no-modules` and
  `--target bundler` are not supported.
- The page's Content-Security-Policy must allow `blob:` workers
  (`worker-src blob:`, or `script-src` with `blob:` when there is no
  `worker-src`): the worker's small loader script is a `blob:` URL.
- The app's start function runs again in every worker. `rinch_web::mount`,
  `mount_with_menu_bar` and the web `App::run` return without mounting there
  (there is no `document`); code that runs before them and needs the page
  should check `is_worker()` first. A `Worker::spawn` in the start function is
  refused in the worker instances (above), so it does not spawn itself in
  each of them.
- Each worker is a separate instance with its own memory: statics start at
  their initial values, and nothing is shared with the page. Each costs the
  module's initial memory; the compiled module itself is shared.

On the desktop the worker is a thread and shares the process's statics, so a
handler should not rely on a global it does not own.

Not provided: shared memory between the page and a worker (that needs the
wasm `atomics` feature, which needs a nightly toolchain), closures as handlers,
async handlers, and a thread pool.
