# Async Tasks

rinch's asynchronous APIs call you back on the UI thread: `set_timeout`,
`rinch_http::fetch`, `rinch_ws` and `rinch_rtc` callbacks. When the logic is sequential — sign
in, then list projects, then open one; retry with a back-off; debounce — the
same calls can be awaited instead, in a **task** that runs on the UI thread on
every platform (desktop, Android, embed and the web):

```rust
use rinch::prelude::*;
use rinch_http::{Request, fetch_async};
use std::time::Duration;

#[component]
fn projects() -> NodeHandle {
    let status = Signal::new(String::from("signing in…"));
    spawn_local(async move {
        let login = fetch_async(Request::post("/api/login")).await;
        if login.is_err() {
            status.set("offline".into());
            return;
        }
        sleep(Duration::from_millis(250)).await;
        match fetch_async(Request::get("/api/projects")).await {
            Ok(list) => status.set(list.text()),
            Err(e) => status.set(format!("{e}")),
        }
    });
    rsx! { p { {|| status.get()} } }
}
```

- **`spawn_local(future) -> TaskHandle`** runs a future that need not be `Send`:
  it stays on the UI thread, so it may capture `Rc`s and set signals directly,
  as a callback does. The future is not polled inside `spawn_local`; it starts
  once the caller returns. Each poll is one `batch`, so the signals a task sets
  between two `.await`s update the DOM once.
- **`sleep(duration)`** is the awaitable `set_timeout` (rounded up to whole
  milliseconds).
- **`rinch_http::fetch_async(request)`** resolves to what `fetch` would have
  passed its callback. The request starts when the future is first polled.
  Dropping the future discards the result; it does not cancel the request.
- **`rinch_ws::connect(url)?.into_stream()`** reads a socket by awaiting its
  events: `stream.next().await` yields a `WsEvent` (`Open`, `Message`, `Close`,
  `Error`) and `None` once the stream has ended — after the close, or after an
  error that came before the connection opened. `send_text`, `send_bytes` and
  `close` are on the stream too. The stream takes the handle's four callback
  slots, and dropping it closes the connection. To offer subprotocols, start
  from `WsRequest` instead of `connect` (below).

`spawn_local`, `sleep` and `TaskHandle` are in the prelude and in `rinch::task`.

### WebSocket subprotocols and headers

`connect(url)` offers no subprotocol. To offer some — a server that
authenticates through `Sec-WebSocket-Protocol`, say — build a `WsRequest`:

```rust
use rinch_ws::{WsEvent, WsRequest};

let mut feed = WsRequest::new("wss://example.com/feed")
    .protocol("feed.v2")                // in order of preference
    .protocol(format!("token.{token}"))
    .connect()?
    .into_stream();
if let Some(WsEvent::Open) = feed.next().await {
    // The one the server chose, or "" when it chose none.
    status.set(feed.protocol());
}
```

`WsHandle::protocol()` / `WsStream::protocol()` answer what a browser's
`WebSocket.protocol` does: `""` before the open, then the server's choice; it
is set before the open is delivered, so `on_open` can read it (hold the handle
in an `Rc` and give the callback a `Weak`). Each subprotocol must be a non-empty
HTTP token offered once, or `connect()` fails with `WsError::InvalidUrl` on both
targets, as the browser's constructor does.

`WsRequest::header(name, value)` adds a handshake header, **on native only**: a
browser `WebSocket` cannot send headers, so the method does not exist on
`wasm32` and code that uses it needs a `#[cfg(not(target_arch = "wasm32"))]`.
Use a subprotocol, a cookie or the URL for what must work on the web. A name
given twice is sent twice; the handshake's own headers (`Host`, `Upgrade`,
`Connection`, `Sec-WebSocket-*`) are refused. A redirect answer is not
followed, as in a browser: the connect fails rather than carry the headers to
another host. On native a URL the `http` crate cannot parse (`ws://` with no
host) is refused by `connect()` itself.

One difference remains: on native, offering subprotocols to a server that
chooses none of them fails the connect (`on_error` with `WsError::Connect`,
from tungstenite's handshake check), where a browser opens the connection with
`protocol() == ""`.

## Lifetime

A task spawned **while a component renders** belongs to that component. When the
component unmounts, the task's future is dropped on the spot, without being
polled again — and with it whatever it holds: a pending `sleep` clears its
timer, a `WsStream` closes its socket. That is the rule `set_timeout` follows,
for the same reason: the future captured the component's signals, which are
freed with it.

For work that must finish anyway (saving on close), spawn it outside any render
or opt out explicitly:

```rust
rinch::reactive::unowned(|| spawn_local(async move { save().await }));
```

A task spawned inside an effect or a reactive closure belongs to the
component, not to that run of the effect, so it is **not** cancelled when the
effect re-runs. For a fetch per query change, keep the previous `TaskHandle`
and `abort()` it before spawning the next:

```rust
let current: Rc<Cell<Option<TaskHandle>>> = Rc::default();
Effect::new(move || {
    let q = query.get();
    if let Some(previous) = current.take() {
        previous.abort();
    }
    current.set(Some(spawn_local(async move { results.set(search(q).await) })));
});
```

`TaskHandle::abort()` stops a task early; `is_finished()` says whether it has
completed, been aborted, panicked, or been dropped with its component. Dropping
the handle does not stop the task. Call both on the UI thread: the handle is
`Send`, but `abort()` from another thread finds no task and does nothing.

## How tasks are driven

Only a task's id crosses threads. A wake from any thread — a request finishing
on a worker, the timer thread — asks for one poll on the UI thread: on the web
in a microtask, elsewhere through the same main-thread queue `Signal::send`
uses, which the desktop and Android loops wake for and an embedded
`RinchContext` drains at the top of `update()`. No thread is started for it.
The web's microtask scheduler comes with a `rinch-web` mount, with a
`RinchContext` created on wasm32, or, on a page with neither, from
`rinch_web::install_runtime()` (see
[Running on WASM](wasm.md#a-page-with-no-rinch-root)); without one, a task on
the web is never polled.

A task polls under the document and the context root it was spawned in, as a
timer fires in them, so `use_store` inside a task spawned by an embedded
context finds that context's stores.

For work that would block the UI thread, on the web as well as the desktop,
see [Background Workers](./workers.md): a worker's replies can be awaited in a
task with `worker.next().await`.

Not provided: tasks with a return value (send the result through a `Signal`),
a `Stream` trait, and `Send` futures on a thread pool — run a tokio runtime on
a background thread for that (see [Cross-Thread Dispatch](./signals.md#cross-thread-dispatch)) and hand results
back with `Signal::send`.
