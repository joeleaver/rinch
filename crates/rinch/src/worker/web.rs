//! The web worker: a Web Worker running a second instance of the app's own
//! wasm module.
//!
//! Shared memory between instances needs the `atomics` target feature, and
//! that needs a nightly `-Z build-std`, so each worker is a separate instance
//! with its own memory and messages are copied as bytes. The UI side posts the
//! worker a small loader (a module worker from a `blob:` URL), the URL of the
//! wasm-bindgen glue (`import.meta.url`, so the glue must be an ES module:
//! `--target web`) and the already compiled `WebAssembly.Module`. The loader
//! imports the glue, instantiates the module — which runs the app's start
//! function, where `rinch_web::mount*` find no `document` and return — and
//! then calls [`__rinch_worker_run`] once per message.
//!
//! The handler reaches the other instance as the address of a `fn`: the two
//! instances are the same module, so they share one function table.

use std::cell::RefCell;
use std::marker::PhantomData;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::{Raw, Replies, WorkerMessage, deliver};

#[wasm_bindgen]
extern "C" {
    /// The URL of the wasm-bindgen glue module: `import.meta.url` evaluated in
    /// that module.
    #[wasm_bindgen(thread_local_v2, js_namespace = ["import", "meta"], js_name = url)]
    static GLUE_URL: String;
}

/// The worker's script. Its first message carries the glue URL, the module
/// and the handler; the messages after it are byte buffers, queued until the
/// instance is up. A failure (the glue would not load, a handler panicked and
/// trapped the instance) is posted back as `{ rinchFailed }` and closes the
/// worker.
const LOADER: &str = r#"
let glue = null;
let entry = null;
let queue = [];
const fail = (err) => {
  self.postMessage({ rinchFailed: String(err && err.message ? err.message : err) });
  self.close();
};
const run = (bytes) => {
  try {
    glue.__rinch_worker_run(entry.shim, entry.handler, bytes);
  } catch (err) {
    fail(err);
    return false;
  }
  return true;
};
self.onmessage = async (event) => {
  const data = event.data;
  if (entry === null) {
    entry = data;
    try {
      const g = await import(entry.glue);
      await g.default({ module_or_path: entry.module });
      glue = g;
    } catch (err) {
      fail(err);
      return;
    }
    for (const bytes of queue) {
      if (!run(bytes)) return;
    }
    queue = [];
    return;
  }
  if (glue === null) {
    queue.push(new Uint8Array(data));
    return;
  }
  run(new Uint8Array(data));
};
"#;

thread_local! {
    /// The loader's `blob:` URL, made once per page and never revoked.
    static LOADER_URL: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn is_worker() -> bool {
    js_sys::global()
        .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()
        .is_ok()
}

/// The UI side of a running Web Worker.
pub(super) struct Handle {
    worker: web_sys::Worker,
    // Kept alive as long as the worker is.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}

impl Handle {
    pub(super) fn send(&self, bytes: Vec<u8>) {
        let array = js_sys::Uint8Array::from(&bytes[..]);
        let transfer = js_sys::Array::of1(&array.buffer());
        let _ = self
            .worker
            .post_message_with_transfer(&array.buffer(), &transfer);
    }

    pub(super) fn terminate(self) {
        self.worker.set_onmessage(None);
        self.worker.set_onerror(None);
        self.worker.terminate();
    }
}

/// The worker side: posts replies to the UI thread.
pub(super) struct Sink;

impl Sink {
    pub(super) fn send(&self, bytes: Vec<u8>) {
        let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
        let array = js_sys::Uint8Array::from(&bytes[..]);
        let transfer = js_sys::Array::of1(&array.buffer());
        let _ = scope.post_message_with_transfer(&array.buffer(), &transfer);
    }

    pub(super) fn is_terminated(&self) -> bool {
        false
    }
}

fn loader_url() -> Result<String, JsValue> {
    if let Some(url) = LOADER_URL.with(|u| u.borrow().clone()) {
        return Ok(url);
    }
    let parts = js_sys::Array::of1(&JsValue::from_str(LOADER));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    LOADER_URL.with(|u| *u.borrow_mut() = Some(url.clone()));
    Ok(url)
}

pub(super) fn spawn<In: WorkerMessage, Out: WorkerMessage>(
    id: u64,
    handler: fn(In, &Replies<Out>),
) -> Option<Handle> {
    match start::<In, Out>(id, handler) {
        Ok(handle) => Some(handle),
        Err(err) => {
            let message = format!("the worker could not start: {}", describe(&err));
            // Reported after `spawn` returns, as every event is — in a
            // microtask of its own, not a rinch task: before the first mount
            // there is no task scheduler on the page to run one.
            let report = Closure::once_into_js(move || deliver(id, Raw::Failed(message)));
            let global = js_sys::global();
            if let Ok(queue) = js_sys::Reflect::get(&global, &"queueMicrotask".into()) {
                let _ = queue
                    .unchecked_into::<js_sys::Function>()
                    .call1(&global, &report);
            }
            None
        }
    }
}

fn start<In: WorkerMessage, Out: WorkerMessage>(
    id: u64,
    handler: fn(In, &Replies<Out>),
) -> Result<Handle, JsValue> {
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let worker = web_sys::Worker::new_with_options(&loader_url()?, &options)?;

    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let data = event.data();
            if let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
                deliver(id, Raw::Reply(js_sys::Uint8Array::new(buffer).to_vec()));
            } else if let Some(error) = js_sys::Reflect::get(&data, &"rinchError".into())
                .ok()
                .and_then(|v| v.as_string())
            {
                deliver(id, Raw::Error(error));
            } else {
                let failed = js_sys::Reflect::get(&data, &"rinchFailed".into())
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_else(|| "unknown error".into());
                deliver(id, Raw::Failed(format!("the worker failed: {failed}")));
            }
        });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The loader script itself failed (blocked by a Content-Security-Policy,
    // a syntax error): the worker never ran.
    let on_error =
        Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
            event.prevent_default();
            deliver(
                id,
                Raw::Failed(format!("the worker failed: {}", event.message())),
            );
        });
    worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let shim: fn(usize, Vec<u8>) = run_shim::<In, Out>;
    let init = js_sys::Object::new();
    js_sys::Reflect::set(&init, &"glue".into(), &GLUE_URL.with(|u| u.clone()).into())?;
    js_sys::Reflect::set(&init, &"module".into(), &wasm_bindgen::module())?;
    js_sys::Reflect::set(&init, &"shim".into(), &(shim as usize as f64).into())?;
    js_sys::Reflect::set(&init, &"handler".into(), &(handler as usize as f64).into())?;
    worker.post_message(&init)?;

    Ok(Handle {
        worker,
        _on_message: on_message,
        _on_error: on_error,
    })
}

fn describe(err: &JsValue) -> String {
    err.dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| err.as_string())
        .unwrap_or_else(|| format!("{err:?}"))
}

/// Turn a function address made by [`start`] back into the function.
///
/// # Safety
///
/// `addr` must be `f as usize` for an `f: F` of this very wasm module, and `F`
/// a `fn` pointer type. Both instances are the same module (the UI posts its
/// own `WebAssembly.Module`), so they share one function table and the
/// address names the same function in each.
unsafe fn fn_at<F: Copy>(addr: usize) -> F {
    const { assert!(size_of::<F>() == size_of::<usize>()) };
    // SAFETY: the caller's contract above; the sizes are asserted equal.
    unsafe { std::mem::transmute_copy(&addr) }
}

/// Per message, in the worker: decode, run the handler. Monomorphised on the
/// UI side, which posts its address.
fn run_shim<In: WorkerMessage, Out: WorkerMessage>(handler: usize, bytes: Vec<u8>) {
    // SAFETY: `handler` is the `fn(In, &Replies<Out>)` `start::<In, Out>`
    // posted together with this shim's own address.
    let handler: fn(In, &Replies<Out>) = unsafe { fn_at(handler) };
    let replies = Replies::<Out> {
        sink: Sink,
        _out: PhantomData,
    };
    match In::from_bytes(bytes) {
        Some(msg) => handler(msg, &replies),
        None => {
            // Not fatal: tell the UI and wait for the next message.
            let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
            let error = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &error,
                &"rinchError".into(),
                &"a message could not be decoded".into(),
            );
            let _ = scope.post_message(&error);
        }
    }
}

/// The loader's entry: run one message through the shim at `shim`.
#[doc(hidden)]
#[wasm_bindgen]
pub fn __rinch_worker_run(shim: usize, handler: usize, bytes: Vec<u8>) {
    // Nothing in a worker instance runs rinch tasks or timers: make
    // `spawn_local` / `set_timeout` in a handler panic rather than never run.
    rinch_core::task::__enter_worker_context();
    // SAFETY: `shim` is the address of a `run_shim::<In, Out>` posted by
    // `start` in an instance of this same module.
    let shim: fn(usize, Vec<u8>) = unsafe { fn_at(shim) };
    shim(handler, bytes);
}
