//! The web half of [`super::dsp`]: the processor's raw wasm module runs inside an `AudioWorklet`.
//!
//! The page compiles the module (`WebAssembly.compile`), checks its exports and imports, and hands
//! the compiled `WebAssembly.Module` to the worklet in `processorOptions`; the processor
//! instantiates it synchronously with no imports, refuses a module whose `rinch_dsp_abi` is not
//! [`rinch_dsp::ABI_VERSION`], and calls the [`rinch_dsp`] exports once per 128-frame quantum.
//! Messages go in over the node's `MessagePort` and are applied at the top of `process()`; replies
//! come back the same way. No `SharedArrayBuffer`, so the page needs no cross-origin isolation.
//!
//! The port carries, worklet → page: a `Uint8Array` (a reply), `{applied: bytes}` (messages the
//! processor took, which frees that much of the message budget), `{error: string}` (the processor
//! failed: sent once, after which the worklet renders nothing).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioNode, AudioWorkletNode, AudioWorkletNodeOptions,
    BaseAudioContext, Event, MessageEvent,
};

use super::dsp::DspOutputConfig;
use crate::error::AvError;

/// The worklet side. `{NAME}` is replaced per output (a processor name registers once per context),
/// `{ABI}` by [`rinch_dsp::ABI_VERSION`].
///
/// Per quantum it allocates nothing of its own except the replies it posts: the byte view of the
/// module's memory and the per-channel sample views are cached, and re-made only when the memory
/// grows (which detaches the old buffer) or the render pointer moves.
const PROCESSOR: &str = r#"
class RinchDsp extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const o = options.processorOptions;
    this.ch = o.channels;
    this.dead = false;
    this.inbox = [];
    this.port.onmessage = (e) => { if (!this.dead) this.inbox.push(e.data); };
    try {
      this.ex = new WebAssembly.Instance(o.module, {}).exports;
      const abi = typeof this.ex.rinch_dsp_abi === "function" ? this.ex.rinch_dsp_abi() : undefined;
      if (abi !== {ABI}) {
        throw new Error("the module speaks rinch-dsp ABI " +
          (abi === undefined ? "(none: it has no rinch_dsp_abi export)" : abi) +
          ", this rinch-av speaks {ABI}: rebuild it against this rinch-dsp");
      }
      this.ex.rinch_dsp_init(sampleRate, this.ch);
    } catch (e) {
      this.fail(e);
    }
  }
  fail(e) {
    if (this.dead) return;
    this.dead = true;
    this.inbox.length = 0;
    const why = (e && e.message !== undefined) ? (e.name + ": " + e.message) : String(e);
    this.port.postMessage({ error: why });
  }
  bytes() {
    const b = this.ex.memory.buffer;
    if (this.u8 === undefined || this.u8.buffer !== b) this.u8 = new Uint8Array(b);
    return this.u8;
  }
  samples(p, n) {
    const b = this.ex.memory.buffer;
    if (this.sv === undefined || this.svBuf !== b || this.svPtr !== p || this.svN !== n) {
      this.sv = [];
      for (let c = 0; c < this.ch; c++) this.sv.push(new Float32Array(b, p + c * n * 4, n));
      this.svBuf = b; this.svPtr = p; this.svN = n;
    }
    return this.sv;
  }
  process(inputs, outputs) {
    if (this.dead) return false;
    try {
      const ex = this.ex;
      if (this.inbox.length > 0) {
        let applied = 0;
        for (const m of this.inbox) {
          const p = ex.rinch_dsp_alloc(m.length);
          this.bytes().set(m, p);
          ex.rinch_dsp_message(m.length);
          applied += m.length + 4;
        }
        this.inbox.length = 0;
        this.port.postMessage({ applied });
      }
      const out = outputs[0];
      const n = out[0].length;
      const views = this.samples(ex.rinch_dsp_render(n), n);
      for (let c = 0; c < out.length; c++) out[c].set(views[Math.min(c, this.ch - 1)]);
      let len;
      while ((len = ex.rinch_dsp_reply()) >= 0) {
        const at = ex.rinch_dsp_reply_ptr();
        this.port.postMessage(this.bytes().slice(at, at + len));
      }
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }
}
registerProcessor("{NAME}", RinchDsp);
"#;

/// The exports [`PROCESSOR`] calls; a module without one of them is refused before the worklet.
const EXPORTS: [&str; 8] = [
    "memory",
    "rinch_dsp_abi",
    "rinch_dsp_init",
    "rinch_dsp_alloc",
    "rinch_dsp_message",
    "rinch_dsp_render",
    "rinch_dsp_reply",
    "rinch_dsp_reply_ptr",
];

thread_local! {
    static NEXT: Cell<u32> = const { Cell::new(0) };
}

/// The port's listener, kept alive as long as the output.
type OnMessage = Closure<dyn FnMut(MessageEvent)>;
/// The node's `processorerror` listener.
type OnError = Closure<dyn FnMut(Event)>;

struct Shared {
    /// The context the output owns (closed on drop, resumed by [`Web::resume`]); `None` when the
    /// caller owns the graph ([`open_in`]).
    owned: Option<AudioContext>,
    ctx: BaseAudioContext,
    dest: AudioNode,
    node: RefCell<Option<AudioWorkletNode>>,
    /// Messages sent before the worklet was running, delivered in order once it is.
    pending: RefCell<Vec<Vec<u8>>>,
    /// Message bytes (+4 each) sent and not yet applied by the processor: pending here, in flight
    /// on the port, or waiting in the worklet's inbox. Bounded by `message_capacity`.
    in_flight: Cell<usize>,
    message_capacity: usize,
    replies: RefCell<VecDeque<Vec<u8>>>,
    /// Reply bytes (+4 each) in `replies`. Bounded by `reply_capacity`.
    reply_bytes: Cell<usize>,
    reply_capacity: usize,
    dropped: Cell<u64>,
    error: RefCell<Option<String>>,
    listeners: RefCell<Option<(OnMessage, OnError)>>,
}

impl Shared {
    fn fail(&self, why: String) {
        let mut e = self.error.borrow_mut();
        if e.is_none() {
            tracing::error!("rinch-av: the DSP output failed: {why}");
            *e = Some(why);
        }
        self.pending.borrow_mut().clear();
    }

    fn on_port(&self, data: JsValue) {
        if let Some(bytes) = data.dyn_ref::<Uint8Array>() {
            let cost = bytes.length() as usize + 4;
            if self.reply_bytes.get() + cost > self.reply_capacity {
                self.dropped.set(self.dropped.get() + 1);
            } else {
                self.reply_bytes.set(self.reply_bytes.get() + cost);
                self.replies.borrow_mut().push_back(bytes.to_vec());
            }
        } else if let Some(n) = Reflect::get(&data, &"applied".into())
            .ok()
            .and_then(|v| v.as_f64())
        {
            self.in_flight
                .set(self.in_flight.get().saturating_sub(n as usize));
        } else if let Some(why) = Reflect::get(&data, &"error".into())
            .ok()
            .and_then(|v| v.as_string())
        {
            self.fail(why);
        }
    }
}

pub(super) struct Web {
    shared: Rc<Shared>,
}

impl Web {
    pub(super) fn send(&self, bytes: &[u8]) -> bool {
        let s = &self.shared;
        let cost = bytes.len() + 4;
        if s.error.borrow().is_some() || s.in_flight.get() + cost > s.message_capacity {
            return false;
        }
        let port = s.node.borrow().as_ref().and_then(|n| n.port().ok());
        match port {
            Some(port) => {
                if port.post_message(&Uint8Array::from(bytes)).is_err() {
                    return false;
                }
            }
            None => s.pending.borrow_mut().push(bytes.to_vec()),
        }
        s.in_flight.set(s.in_flight.get() + cost);
        true
    }

    pub(super) fn replies(&self) -> Vec<Vec<u8>> {
        self.shared.reply_bytes.set(0);
        self.shared.replies.borrow_mut().drain(..).collect()
    }

    pub(super) fn dropped_replies(&self) -> u64 {
        self.shared.dropped.get()
    }

    pub(super) fn error(&self) -> Option<String> {
        self.shared.error.borrow().clone()
    }

    pub(super) fn resume(&self) {
        if let Some(ctx) = &self.shared.owned {
            let _ = ctx.resume();
        }
    }

    #[cfg(test)]
    pub(super) fn started(&self) -> bool {
        self.shared.node.borrow().is_some()
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        if let Some(node) = self.shared.node.borrow_mut().take() {
            let _ = node.disconnect();
            node.set_onprocessorerror(None);
            if let Ok(port) = node.port() {
                port.set_onmessage(None);
            }
        }
        if let Some(ctx) = &self.shared.owned {
            let _ = ctx.close();
        }
    }
}

fn js_err(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| format!("{}: {}", String::from(e.name()), String::from(e.message())))
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

pub(super) fn open(
    config: DspOutputConfig,
    module: &[u8],
) -> Result<(Web, DspOutputConfig), AvError> {
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(config.sample_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .map_err(|e| AvError::Backend(format!("Failed to create AudioContext: {}", js_err(&e))))?;
    let base: BaseAudioContext = ctx.clone().unchecked_into();
    let dest: AudioNode = ctx.destination().unchecked_into();
    Ok(open_in(Some(ctx), &base, &dest, config, module))
}

/// Start the output in `ctx`, connected to `dest`. `owned` is the context to close on drop and to
/// resume, when the output owns it.
pub(super) fn open_in(
    owned: Option<AudioContext>,
    ctx: &BaseAudioContext,
    dest: &AudioNode,
    config: DspOutputConfig,
    module: &[u8],
) -> (Web, DspOutputConfig) {
    let real = DspOutputConfig {
        sample_rate: ctx.sample_rate() as u32,
        // A worklet renders in quanta of 128 frames.
        buffer_size: 128,
        ..config
    };
    let shared = Rc::new(Shared {
        owned,
        ctx: ctx.clone(),
        dest: dest.clone(),
        node: RefCell::new(None),
        pending: RefCell::new(Vec::new()),
        in_flight: Cell::new(0),
        message_capacity: config.message_capacity,
        replies: RefCell::new(VecDeque::new()),
        reply_bytes: Cell::new(0),
        reply_capacity: config.reply_capacity,
        dropped: Cell::new(0),
        error: RefCell::new(None),
        listeners: RefCell::new(None),
    });
    let bytes = Uint8Array::from(module);
    let channels = config.channels.max(1);
    wasm_bindgen_futures::spawn_local(start(Rc::downgrade(&shared), bytes, channels));
    (Web { shared }, real)
}

/// Load the processor script and the module, then start the node. Gives up quietly if the output
/// was dropped meanwhile (dropping closes the context, which rejects `addModule`: not a failure);
/// otherwise a failure is the output's [`Web::error`].
async fn start(shared: Weak<Shared>, bytes: Uint8Array, channels: u16) {
    if let Err(e) = try_start(&shared, bytes, channels).await
        && let Some(s) = shared.upgrade()
    {
        s.fail(format!("the audio worklet did not start: {}", js_err(&e)));
    }
}

/// The module's problems a worklet would only report as a failed constructor, said plainly.
fn check_module(module: &js_sys::WebAssembly::Module) -> Result<(), JsValue> {
    let imports = js_sys::WebAssembly::Module::imports(module);
    if imports.length() > 0 {
        let names: Vec<String> = imports
            .iter()
            .filter_map(|d| {
                let m = Reflect::get(&d, &"module".into()).ok()?.as_string()?;
                let n = Reflect::get(&d, &"name".into()).ok()?.as_string()?;
                Some(format!("{m}.{n}"))
            })
            .collect();
        return Err(JsValue::from_str(&format!(
            "the DSP module imports {} (an AudioWorklet instantiates it with no imports: \
             keep wasm-bindgen and anything that imports from the page out of its graph)",
            names.join(", ")
        )));
    }
    let exports: Vec<String> = js_sys::WebAssembly::Module::exports(module)
        .iter()
        .filter_map(|d| Reflect::get(&d, &"name".into()).ok()?.as_string())
        .collect();
    let missing: Vec<&str> = EXPORTS
        .iter()
        .copied()
        .filter(|e| !exports.iter().any(|x| x == e))
        .collect();
    if !missing.is_empty() {
        return Err(JsValue::from_str(&format!(
            "the DSP module does not export {} (is it a rinch_dsp::export_dsp! cdylib, built \
             against rinch-dsp ABI {}?)",
            missing.join(", "),
            rinch_dsp::ABI_VERSION
        )));
    }
    Ok(())
}

async fn try_start(shared: &Weak<Shared>, bytes: Uint8Array, channels: u16) -> Result<(), JsValue> {
    let name = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        format!("rinch-dsp-{v}")
    });
    let Some(ctx) = shared.upgrade().map(|s| s.ctx.clone()) else {
        return Ok(());
    };
    // Compile (and check) first: a bad module is the likeliest failure and needs no worklet.
    let module: js_sys::WebAssembly::Module = JsFuture::from(js_sys::WebAssembly::compile(&bytes))
        .await?
        .unchecked_into();
    check_module(&module)?;
    let source = PROCESSOR
        .replace("{NAME}", &name)
        .replace("{ABI}", &rinch_dsp::ABI_VERSION.to_string());
    let parts: Array = [JsValue::from_str(&source)].into_iter().collect();
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("application/javascript");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &bag)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let loaded = JsFuture::from(ctx.audio_worklet()?.add_module(&url)?).await;
    let _ = web_sys::Url::revoke_object_url(&url);
    loaded?;
    let Some(shared) = shared.upgrade() else {
        return Ok(());
    };

    let processor_options = Object::new();
    Reflect::set(&processor_options, &"module".into(), &module)?;
    Reflect::set(
        &processor_options,
        &"channels".into(),
        &JsValue::from(u32::from(channels)),
    )?;
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(0);
    options.set_number_of_outputs(1);
    options.set_output_channel_count(
        &[JsValue::from(u32::from(channels))]
            .into_iter()
            .collect::<Array>(),
    );
    options.set_processor_options(Some(&processor_options));
    let node = AudioWorkletNode::new_with_options(&shared.ctx, &name, &options)?;
    node.connect_with_audio_node(&shared.dest)?;

    let weak = Rc::downgrade(&shared);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        if let Some(s) = weak.upgrade() {
            s.on_port(e.data());
        }
    });
    // An exception the processor's own try/catch did not see (the browser's, or one thrown
    // before the constructor ran).
    let weak = Rc::downgrade(&shared);
    let on_error = Closure::<dyn FnMut(Event)>::new(move |e: Event| {
        if let Some(s) = weak.upgrade() {
            let detail = Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "processorerror".into());
            s.fail(format!("the audio worklet processor failed: {detail}"));
        }
    });
    node.set_onprocessorerror(Some(on_error.as_ref().unchecked_ref()));
    let port = node.port()?;
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    for m in shared.pending.borrow_mut().drain(..) {
        port.post_message(&Uint8Array::from(m.as_slice()))?;
    }
    *shared.listeners.borrow_mut() = Some((on_message, on_error));
    *shared.node.borrow_mut() = Some(node);
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Through an `OfflineAudioContext`, so the rendered samples can be read back per channel.
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::*;
    use web_sys::{AudioBuffer, AudioNode, BaseAudioContext, OfflineAudioContext};

    use crate::audio::DspOutputConfig;
    use crate::audio::dsp::open_dsp_output_in;

    wasm_bindgen_test_configure!(run_in_browser);

    /// See `tests/dsp_web.rs`: with no message it renders `0.5 + 0.25 × channel` on each channel.
    const ECHO: &[u8] = include_bytes!("../../tests/fixtures/dsp_echo.wasm");

    async fn sleep_ms(ms: i32) {
        let p = js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = JsFuture::from(p).await;
    }

    /// Each output channel carries its own processor channel (the review's mutant W3, every
    /// output channel copying channel 0, renders 0.5 on all three).
    #[wasm_bindgen_test]
    async fn each_output_channel_carries_its_own_processor_channel() {
        const FRAMES: u32 = 4096;
        let ctx = OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            3, FRAMES, 48_000.0,
        )
        .expect("offline context");
        let base: BaseAudioContext = ctx.clone().unchecked_into();
        let dest: AudioNode = ctx.destination().unchecked_into();
        let out = open_dsp_output_in(
            &base,
            &dest,
            DspOutputConfig {
                sample_rate: 48_000,
                channels: 3,
                ..Default::default()
            },
            ECHO,
        );
        let mut waited = 0;
        while !out.started() && out.error().is_none() && waited < 3000 {
            sleep_ms(20).await;
            waited += 20;
        }
        assert!(
            out.started(),
            "the worklet never started: {:?}",
            out.error()
        );
        assert_eq!(out.config().buffer_size, 128);
        let rendered: AudioBuffer = JsFuture::from(ctx.start_rendering().unwrap())
            .await
            .expect("rendering")
            .unchecked_into();
        assert_eq!(rendered.number_of_channels(), 3);
        for c in 0..3 {
            let data = rendered.get_channel_data(c).unwrap();
            // The processor may be constructed a few quanta in; the last 1024 frames are its.
            let tail = &data[data.len() - 1024..];
            let want = 0.5 + 0.25 * c as f32;
            assert!(
                tail.iter().all(|&s| s == want),
                "channel {c}: want {want}, got {:?}…",
                &tail[..4]
            );
        }
        assert!(out.error().is_none(), "{:?}", out.error());
    }
}
